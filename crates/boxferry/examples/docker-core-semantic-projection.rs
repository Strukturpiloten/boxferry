//! Offline, private comparison of one authored core Compose graph.
//!
//! No socket, runtime, command execution, environment lookup or output files are used.
//! Image configuration facts do not establish daemon defaults or native admission.

#[cfg(feature = "compose")]
mod projection {
    use std::{
        collections::BTreeMap,
        io::{self, Read, Write},
    };

    use boxferry::compose::compose_lens::{
        loader::{DocumentInput, DocumentOrigin, LoadedProject},
        merge::{MergedProject, MergedScalarKind, MergedValue, MergedValueKind, merge_project},
        source::SourceId as ComposeSourceId,
    };
    use boxferry::{
        Application, Command, ComposeImporter, ComposeSource, ConversionKind, EnvironmentValue, Identifier,
        ImportAdapter, Network, NetworkAttachment, ResourceOwnership, Service, SourceId, Sourced,
    };
    use serde_json::{Value, json};

    const EXPECTATIONS: &str =
        include_str!("../../../fixtures/conformance/docker-application/core-semantic-expectations.json");
    const MAX_DOCUMENT: usize = 16_384;
    const FLAGS: [&str; 8] = [
        "--image-alias",
        "--lane",
        "--engine-release",
        "--api-version",
        "--daemon-mode",
        "--image-index",
        "--image-manifest",
        "--image-config",
    ];

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Failure {
        Arguments,
        Context,
        Input,
        Shape,
        Neutral,
        Outcomes,
        Output,
    }

    impl Failure {
        pub const fn code(self) -> &'static str {
            match self {
                Self::Arguments => "core-projection-arguments",
                Self::Context => "core-projection-context",
                Self::Input => "core-projection-input",
                Self::Shape => "core-projection-shape",
                Self::Neutral => "core-projection-neutral",
                Self::Outcomes => "core-projection-outcomes",
                Self::Output => "core-projection-output",
            }
        }
    }

    fn require(value: bool, failure: Failure) -> Result<(), Failure> {
        if value { Ok(()) } else { Err(failure) }
    }

    fn expectations() -> Result<Value, Failure> {
        serde_json::from_str(EXPECTATIONS).map_err(|_| Failure::Context)
    }

    pub fn arguments(values: &[String], expected: &Value) -> Result<BTreeMap<String, String>, Failure> {
        require(values.len() == FLAGS.len() * 2 + 2, Failure::Arguments)?;
        require(
            values[values.len() - 2..] == ["--platform", "linux/amd64"],
            Failure::Context,
        )?;
        let mut result = BTreeMap::new();
        for (pair, flag) in values[..values.len() - 2].as_chunks::<2>().0.iter().zip(FLAGS) {
            require(
                pair[0] == flag && !pair[1].is_empty() && pair[1].len() <= 256,
                Failure::Arguments,
            )?;
            let _ = result.insert(flag.to_owned(), pair[1].clone());
        }
        let image = result.get("--image-alias").ok_or(Failure::Arguments)?;
        require(
            image == "registry.invalid/boxferry-core/busybox:1.37.0",
            Failure::Context,
        )?;
        let context = &expected["contexts"][result.get("--lane").ok_or(Failure::Arguments)?];
        require(context.is_object(), Failure::Context)?;
        for (flag, field) in [
            ("--engine-release", "engine_release"),
            ("--api-version", "acquisition_api"),
            ("--daemon-mode", "daemon_mode"),
        ] {
            require(
                context[field].as_str() == result.get(flag).map(String::as_str),
                Failure::Context,
            )?;
        }
        for (flag, field) in [
            ("--image-index", "index_digest"),
            ("--image-manifest", "manifest_digest"),
            ("--image-config", "config_digest"),
        ] {
            require(
                expected["image"][field].as_str() == result.get(flag).map(String::as_str),
                Failure::Context,
            )?;
        }
        Ok(result)
    }

    fn load(document: &str) -> Result<MergedProject, Failure> {
        require(!document.is_empty() && document.len() <= MAX_DOCUMENT, Failure::Input)?;
        let loaded = LoadedProject::load([DocumentInput::new(
            ComposeSourceId::new(1),
            DocumentOrigin::new("private-core.compose.yaml", "/offline-core"),
            document,
        )])
        .map_err(|_| Failure::Input)?;
        require(loaded.is_valid() && loaded.diagnostics().is_empty(), Failure::Input)?;
        // No interpolation provider, include loader, filesystem or environment is supplied.
        let merged = merge_project(&loaded, None);
        require(merged.is_valid() && merged.diagnostics().is_empty(), Failure::Input)?;
        merged.project().cloned().ok_or(Failure::Input)
    }

    fn native_value(value: &MergedValue, depth: usize, nodes: &mut usize) -> Result<Value, Failure> {
        *nodes += 1;
        require(depth <= 16 && *nodes <= 4096, Failure::Input)?;
        match value.kind() {
            MergedValueKind::Null(_) => Ok(Value::Null),
            MergedValueKind::Scalar(scalar) => {
                require(scalar.value().len() <= MAX_DOCUMENT, Failure::Input)?;
                let kind = match scalar.kind() {
                    MergedScalarKind::String => "string",
                    MergedScalarKind::Boolean => "boolean",
                    MergedScalarKind::Number => "number",
                };
                Ok(json!({"scalar_kind":kind,"value":scalar.value()}))
            }
            MergedValueKind::Mapping(entries) => {
                require(entries.len() <= 256, Failure::Input)?;
                let mut result = serde_json::Map::new();
                for entry in entries {
                    require(
                        entry.is_strict_yaml_string() && !result.contains_key(entry.key()),
                        Failure::Shape,
                    )?;
                    let _ = result.insert(entry.key().to_owned(), native_value(entry.value(), depth + 1, nodes)?);
                }
                Ok(Value::Object(result))
            }
            MergedValueKind::Sequence(entries) => {
                require(entries.len() <= 256, Failure::Input)?;
                entries
                    .iter()
                    .map(|entry| native_value(entry, depth + 1, nodes))
                    .collect::<Result<Vec<_>, _>>()
                    .map(Value::Array)
            }
            _ => Err(Failure::Shape),
        }
    }

    fn like<T, U>(value: T, source: &Sourced<U>) -> Sourced<T> {
        let mut result = Sourced::generated(value);
        for origin in source.origins() {
            result.add_origin(origin.clone());
        }
        result
    }

    fn check_service(service: &Service, image: &str, expected: &Value) -> Result<Service, Failure> {
        require(service.name().as_str() == "docker-service-1", Failure::Neutral)?;
        let runtime_name = service.runtime_name().ok_or(Failure::Neutral)?;
        require(runtime_name.value().expose() == "bf-docker-core", Failure::Neutral)?;
        let actual_image = service.image().ok_or(Failure::Neutral)?;
        require(actual_image.value().as_str() == image, Failure::Neutral)?;
        let command = service.command().ok_or(Failure::Neutral)?;
        let Command::Exec(arguments) = command.value() else {
            return Err(Failure::Neutral);
        };
        let actual: Vec<&str> = arguments.iter().map(boxferry::ProtectedString::expose).collect();
        require(
            actual
                == [
                    "sh",
                    "-c",
                    "printf boxferry-core-ready > /tmp/boxferry-core-marker; exec sleep 3600",
                ],
            Failure::Neutral,
        )?;
        require(
            service.environment().len() == 1 && service.networks().len() == 1,
            Failure::Neutral,
        )?;
        let environment = &service.environment()[0];
        require(environment.value().name().as_str() == "PATH", Failure::Neutral)?;
        let EnvironmentValue::Literal(value) = environment.value().value() else {
            return Err(Failure::Neutral);
        };
        require(
            expected["image"]["config"]["Env"][0].as_str() == Some(format!("PATH={}", value.expose()).as_str()),
            Failure::Neutral,
        )?;
        let attachment = &service.networks()[0];
        let name = Identifier::new("docker-network-1").map_err(|_| Failure::Neutral)?;
        require(
            attachment.value() == &NetworkAttachment::new(name, Vec::new()),
            Failure::Neutral,
        )?;
        // Values above are checked against authored intent/image facts. Copying only their
        // provenance permits whole-struct equality to reject every other semantic setting.
        let mut shape = Service::new(service.name().clone());
        shape.set_runtime_name(runtime_name.clone());
        shape.set_image(actual_image.clone());
        shape.set_command(command.clone());
        shape.add_environment(environment.clone());
        shape.add_network(attachment.clone());
        require(service == &shape, Failure::Neutral)?;
        Ok(shape)
    }

    fn check_network(network: &Network) -> Result<Network, Failure> {
        require(
            network.name().as_str() == "docker-network-1" && network.ownership() == ResourceOwnership::External,
            Failure::Neutral,
        )?;
        let runtime_name = network.runtime_name().ok_or(Failure::Neutral)?;
        require(runtime_name.value().expose() == "bridge", Failure::Neutral)?;
        let mut shape = Network::new(network.name().clone(), ResourceOwnership::External);
        shape.set_runtime_name(runtime_name.clone());
        require(network == &shape, Failure::Neutral)?;
        Ok(shape)
    }

    fn check_application(application: &Application, image: &str, expected: &Value) -> Result<(), Failure> {
        require(
            application.name().as_str() == "bf-docker-core"
                && application.services().len() == 1
                && application.networks().len() == 1,
            Failure::Neutral,
        )?;
        let mut shape = Application::new(application.name().clone());
        let service = &application.services()[0];
        shape
            .add_service(like(check_service(service.value(), image, expected)?, service))
            .map_err(|_| Failure::Neutral)?;
        let network = &application.networks()[0];
        shape
            .add_network(like(check_network(network.value())?, network))
            .map_err(|_| Failure::Neutral)?;
        require(application == &shape, Failure::Neutral)
    }

    pub fn compare(document: &str, values: &[String]) -> Result<Value, Failure> {
        let expected = expectations()?;
        let args = arguments(values, &expected)?;
        let image = args.get("--image-alias").ok_or(Failure::Arguments)?;
        let template = expected["compose_template"].as_str().ok_or(Failure::Context)?;
        let expected_project = load(&template.replace("BOXFERRY_CORE_IMAGE_LITERAL", image))?;
        let actual = load(document)?;
        require(
            native_value(actual.root(), 0, &mut 0)? == native_value(expected_project.root(), 0, &mut 0)?,
            Failure::Shape,
        )?;
        let source = ComposeSource::new(actual, Identifier::new("bf-docker-core").map_err(|_| Failure::Context)?)
            .map_err(|_| Failure::Input)?
            .with_source_id(
                ComposeSourceId::new(1),
                SourceId::new("private-core.compose.yaml").map_err(|_| Failure::Context)?,
            );
        let imported = ComposeImporter::new().map_err(|_| Failure::Context)?.import(&source);
        require(imported.diagnostics().is_empty(), Failure::Outcomes)?;
        check_application(imported.application().ok_or(Failure::Neutral)?, image, &expected)?;
        let mut outcomes = Vec::new();
        for outcome in imported.outcomes() {
            require(
                outcome.kind() == ConversionKind::Exact && outcome.diagnostic().is_none(),
                Failure::Outcomes,
            )?;
            outcomes.push(json!({"subject":outcome.subject(),"kind":"exact","diagnostic":null}));
        }
        require(
            Value::Array(outcomes.clone()) == expected["compose_import_outcomes"],
            Failure::Outcomes,
        )?;
        Ok(
            json!({"schema_version":1,"kind":"boxferry-private-core-semantic-comparison","native_execution":false,
            "native_admission":false,"status":"matched","compose_import_outcomes":outcomes,"compose_import_diagnostics":[]}),
        )
    }

    pub fn run() -> Result<(), Failure> {
        let mut raw = Vec::new();
        io::stdin()
            .lock()
            .take((MAX_DOCUMENT + 1) as u64)
            .read_to_end(&mut raw)
            .map_err(|_| Failure::Input)?;
        require(raw.len() <= MAX_DOCUMENT, Failure::Input)?;
        let document = std::str::from_utf8(&raw).map_err(|_| Failure::Input)?;
        let arguments = std::env::args_os()
            .skip(1)
            .take(FLAGS.len() * 2 + 3)
            .map(|value| {
                require(value.as_encoded_bytes().len() <= 256, Failure::Arguments)?;
                value.into_string().map_err(|_| Failure::Arguments)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let result = compare(document, &arguments)?;
        let bytes = serde_json::to_vec(&result).map_err(|_| Failure::Output)?;
        io::stdout().lock().write_all(&bytes).map_err(|_| Failure::Output)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn fixture() -> Result<(String, Vec<String>), Failure> {
            let value = expectations()?;
            let image = "registry.invalid/boxferry-core/busybox:1.37.0";
            let mut args = vec![
                "--image-alias".to_owned(),
                image.to_owned(),
                "--lane".to_owned(),
                "upstream-rootless".to_owned(),
                "--engine-release".to_owned(),
                "29.8.1".to_owned(),
                "--api-version".to_owned(),
                "1.49".to_owned(),
                "--daemon-mode".to_owned(),
                "rootless".to_owned(),
            ];
            for (flag, field) in [
                ("--image-index", "index_digest"),
                ("--image-manifest", "manifest_digest"),
                ("--image-config", "config_digest"),
            ] {
                args.extend([
                    flag.to_owned(),
                    value["image"][field].as_str().ok_or(Failure::Context)?.to_owned(),
                ]);
            }
            args.extend(["--platform".to_owned(), "linux/amd64".to_owned()]);
            Ok((
                value["compose_template"]
                    .as_str()
                    .ok_or(Failure::Context)?
                    .replace("BOXFERRY_CORE_IMAGE_LITERAL", image),
                args,
            ))
        }

        #[test]
        fn authored_graph_passes_without_native_admission() -> Result<(), Failure> {
            let (document, args) = fixture()?;
            let result = compare(&document, &args)?;
            assert_eq!(result["native_admission"], false);
            assert!(
                !serde_json::to_string(&result)
                    .map_err(|_| Failure::Output)?
                    .contains("PATH=")
            );
            let flow = document.replace(
                "    networks:\n      docker-network-1: {}",
                "    networks: {docker-network-1: {}}",
            );
            assert_eq!(compare(&flow, &args)?, result);
            Ok(())
        }

        #[test]
        fn every_extra_field_even_null_false_zero_or_empty_is_rejected() -> Result<(), Failure> {
            let (document, args) = fixture()?;
            for field in [
                "restart",
                "healthcheck",
                "entrypoint",
                "user",
                "working_dir",
                "read_only",
                "init",
                "stop_grace_period",
                "ports",
                "volumes",
                "deploy",
                "labels",
            ] {
                for value in [
                    "null",
                    "false",
                    "0",
                    "''",
                    "[]",
                    "{}",
                    "{disable: false}",
                    "{test: [NONE]}",
                ] {
                    let altered = document.replacen("    image:", &format!("    {field}: {value}\n    image:"), 1);
                    assert!(compare(&altered, &args).is_err(), "unreviewed field accepted");
                }
            }
            Ok(())
        }

        #[test]
        fn graph_command_order_external_identity_and_nested_endpoints_are_closed() -> Result<(), Failure> {
            let (document, args) = fixture()?;
            for (from, to) in [
                ("docker-service-1:", "extra-service:"),
                ("exec sleep 3600", "exec sleep 9"),
                ("      - sh\n      - -c", "      - -c\n      - sh"),
                ("container_name: bf-docker-core", "container_name: other"),
                ("name: bridge", "name: other"),
                ("external: true", "external: false"),
                ("external: true", "external: 'true'"),
                ("docker-network-1: {}", "docker-network-1: {aliases: []}"),
                ("docker-network-1: {}", "docker-network-1: {ipv4_address: null}"),
                ("PATH=/usr/local", "PATH=/changed"),
                ("    command:", "    unreviewed: true\n    command:"),
            ] {
                assert!(
                    compare(&document.replace(from, to), &args).is_err(),
                    "altered intent accepted"
                );
            }
            for suffix in [
                "volumes: {}\n",
                "configs: {}\n",
                "secrets: {}\n",
                "x-unknown: false\n",
                "services: {}\n",
            ] {
                assert!(
                    compare(&(document.clone() + suffix), &args).is_err(),
                    "extra root accepted"
                );
            }
            Ok(())
        }

        #[test]
        fn source_context_and_image_descriptor_chain_are_explicit() -> Result<(), Failure> {
            let (document, args) = fixture()?;
            for (lane, release, api, mode) in [
                ("debian11-rootful", "20.10.5", "1.41", "rootful"),
                ("debian11-rootless", "20.10.5", "1.41", "rootless"),
                ("upstream-rootful", "29.8.1", "1.49", "rootful"),
                ("upstream-rootless", "29.8.1", "1.49", "rootless"),
            ] {
                let mut context = args.clone();
                for (index, value) in [(3, lane), (5, release), (7, api), (9, mode)] {
                    context[index] = value.to_owned();
                }
                assert_eq!(compare(&document, &context)?["native_admission"], false);
            }
            for index in (1..args.len()).step_by(2) {
                let mut altered = args.clone();
                altered[index] = "unreviewed".to_owned();
                assert!(compare(&document, &altered).is_err(), "context change accepted");
            }
            let mut duplicate = args.clone();
            duplicate.extend(["--platform".to_owned(), "linux/amd64".to_owned()]);
            assert_eq!(compare(&document, &duplicate), Err(Failure::Arguments));
            Ok(())
        }

        #[test]
        fn required_values_cannot_be_missing_null_false_zero_or_empty() -> Result<(), Failure> {
            let (document, args) = fixture()?;
            for (from, key) in [
                (
                    "    image: registry.invalid/boxferry-core/busybox:1.37.0\n",
                    "    image",
                ),
                ("    container_name: bf-docker-core\n", "    container_name"),
                (
                    "    environment:\n      - PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin\n",
                    "    environment",
                ),
                ("    networks:\n      docker-network-1: {}\n", "    networks"),
                ("    name: bridge\n", "    name"),
                ("    external: true\n", "    external"),
            ] {
                assert!(document.contains(from));
                assert!(
                    compare(&document.replace(from, ""), &args).is_err(),
                    "missing required setting accepted"
                );
                for value in ["null", "false", "0", "''", "[]", "{}"] {
                    assert!(
                        compare(&document.replace(from, &format!("{key}: {value}\n")), &args).is_err(),
                        "different required setting accepted"
                    );
                }
            }
            Ok(())
        }

        #[test]
        fn malformed_private_input_remains_bounded_and_value_free() -> Result<(), Failure> {
            let (_, args) = fixture()?;
            for input in [
                String::new(),
                "{".to_owned(),
                "x".repeat(MAX_DOCUMENT + 1),
                "---\nname: private-canary\nservices: [".to_owned(),
            ] {
                let error = compare(&input, &args).err().ok_or(Failure::Input)?;
                assert!(!error.code().contains("private-canary"));
            }
            Ok(())
        }
    }
}

#[cfg(feature = "compose")]
fn main() {
    std::panic::set_hook(Box::new(|_| eprintln!("core-projection-internal-failure")));
    if let Err(error) = projection::run() {
        eprintln!("{}", error.code());
        std::process::exit(1);
    }
}

#[cfg(not(feature = "compose"))]
fn main() {
    eprintln!("core-projection-compose-feature-required");
    std::process::exit(1);
}
