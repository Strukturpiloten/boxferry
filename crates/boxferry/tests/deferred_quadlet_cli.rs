//! Explicit service execution environment deferral and disclosure boundaries.
#![cfg(all(feature = "cli", feature = "compose", feature = "quadlet", feature = "podman"))]
use serde_json::Value;
use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self, Box<dyn Error>> {
        let path = std::env::temp_dir().join(format!(
            "boxferry-deferred-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
    fn source(&self, fields: &str) -> Result<PathBuf, Box<dyn Error>> {
        let path = self.0.join("compose.yaml");
        fs::write(
            &path,
            format!(
                "---\nname: deferred\nservices:\n  app:\n    image: example.invalid/app:1\n    restart: 'no'\n{fields}"
            ),
        )?;
        Ok(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn command(source: &Path, out: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_boxferry"));
    command
        .args(["convert", "compose", "quadlet", "--input-file"])
        .arg(source)
        .arg("--output-directory")
        .arg(out)
        .args(["--console-format", "json"]);
    command
}
fn json(output: &Output) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&output.stdout)?)
}
fn message(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn pure_references_and_external_files_are_exact_without_acquisition() -> Result<(), Box<dyn Error>> {
    let root = Scratch::new()?;
    let input = root.source("    environment:\n      APP_MODE: \"${RUN_MODE}${EXTRA}\"\n    volumes:\n      - \"${DATA_ROOT}/config:/config:ro\"\n")?;
    let out = root.0.join("out");
    let result = command(&input, &out)
        .args([
            "--quadlet-defer",
            "app:RUN_MODE",
            "--quadlet-defer",
            "app:EXTRA",
            "--quadlet-defer",
            "app:DATA_ROOT",
            "--quadlet-service-env-file",
            "app=/deliberately/nonexistent/provider.env",
        ])
        .env("RUN_MODE", "AMBIENT_PRIVATE_CANARY")
        .env("DATA_ROOT", "AMBIENT_PATH_CANARY")
        .output()?;
    assert!(result.status.success(), "{}", message(&result));
    let text = fs::read_to_string(out.join("app.container"))?;
    assert!(text.contains("APP_MODE=${RUN_MODE}${EXTRA}"), "{text}");
    assert!(text.contains("${DATA_ROOT}/config:/config:ro"), "{text}");
    let service = text.split("[Service]").nth(1).ok_or("service section")?;
    assert!(
        service.contains("EnvironmentFile=\"/deliberately/nonexistent/provider.env\""),
        "{text}"
    );
    assert!(!service.contains("APP_MODE="));
    assert!(!text.contains("AMBIENT"));
    let report = json(&result)?;
    assert!(
        report["diagnostics"]
            .as_array()
            .ok_or("diagnostics")?
            .iter()
            .any(|item| item["code"] == "BFQ0015")
    );
    assert!(!message(&result).contains("AMBIENT"));
    Ok(())
}

#[test]
fn inline_providers_are_protected_and_files_keep_precedence_order() -> Result<(), Box<dyn Error>> {
    let root = Scratch::new()?;
    let input = root.source("    environment: [\"APP_MODE=pre-${RUN_MODE}-post\"]\n")?;
    let out = root.0.join("out");
    let result = command(&input, &out)
        .args([
            "--quadlet-defer",
            "app:RUN_MODE",
            "--quadlet-service-env",
            "app:RUN_MODE=PRIVATE_PROVIDER_$OTHER_%h=a b",
            "--quadlet-service-env-file",
            "app=/first.env",
            "--quadlet-service-env-file",
            "app=/second.env",
            "--environment-values",
            "include",
        ])
        .output()?;
    assert!(result.status.success(), "{}", message(&result));
    let text = fs::read_to_string(out.join("app.container"))?;
    assert!(text.contains("APP_MODE=pre-${RUN_MODE}-post"), "{text}");
    let inline = text
        .find("RUN_MODE=PRIVATE_PROVIDER_$OTHER_%%h=a b")
        .ok_or("protected literal provider")?;
    let first = text.find("EnvironmentFile=\"/first.env\"").ok_or("first provider")?;
    let second = text.find("EnvironmentFile=\"/second.env\"").ok_or("second provider")?;
    assert!(inline < first && first < second, "{text}");
    assert!(!message(&result).contains("PRIVATE_PROVIDER"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(out.join("app.container"))?.permissions().mode() & 0o777,
            0o600
        );
    }
    Ok(())
}

#[test]
fn malformed_or_unauthorized_plans_fail_before_writes() -> Result<(), Box<dyn Error>> {
    let root = Scratch::new()?;
    let input = root.source("    environment: { APP_MODE: \"${RUN_MODE}\" }\n")?;
    let cases: &[&[&str]] = &[
        &[
            "--quadlet-defer",
            "unknown:RUN_MODE",
            "--quadlet-service-env-file",
            "unknown=/provider.env",
        ],
        &["--quadlet-defer", "app:RUN_MODE"],
        &[
            "--quadlet-defer",
            "app:UNUSED",
            "--quadlet-service-env-file",
            "app=/provider.env",
        ],
        &[
            "--quadlet-defer",
            "app:RUN_MODE",
            "--quadlet-defer",
            "app:RUN_MODE",
            "--quadlet-service-env-file",
            "app=/provider.env",
        ],
        &[
            "--quadlet-defer",
            "app:RUN_MODE",
            "--quadlet-service-env",
            "app:RUN_MODE=PRIVATE_NEVER_ECHO",
        ],
        &[
            "--quadlet-defer",
            "app:RUN_MODE",
            "--quadlet-service-env",
            "app:RUN-MODE=PRIVATE_NEVER_ECHO",
            "--environment-values",
            "include",
        ],
        &[
            "--quadlet-defer",
            "app:RUN_MODE",
            "--quadlet-service-env-file",
            "app=relative.env",
        ],
        &[
            "--quadlet-defer",
            "app:RUN_MODE",
            "--quadlet-service-env-file",
            "app=/provider.env",
            "--quadlet-service-env-file",
            "app=/provider.env",
        ],
        &[
            "--quadlet-defer",
            "app:RUN_MODE",
            "--quadlet-service-env-file",
            "app=/provider.env",
            "--env",
            "RUN_MODE=PRIVATE_NEVER_ECHO",
        ],
        &[
            "--quadlet-defer",
            "app:RUN_MODE",
            "--quadlet-service-env-file",
            "app=/provider.env",
            "--interpolate",
        ],
        &["--quadlet-service-env-file", "app=/provider.env"],
    ];
    for (index, args) in cases.iter().enumerate() {
        let out = root.0.join(format!("failure-{index}"));
        let result = command(&input, &out)
            .args(*args)
            .args(["--loss-policy", "partial"])
            .output()?;
        assert!(!result.status.success(), "case {index}: {}", message(&result));
        assert!(!out.exists(), "case {index}");
        assert!(
            !message(&result).contains("PRIVATE_NEVER_ECHO"),
            "case {index}: {}",
            message(&result)
        );
    }
    Ok(())
}

#[test]
fn literal_bearing_environment_requires_include_but_empty_inline_is_valid() -> Result<(), Box<dyn Error>> {
    for value in [
        "pre-${RUN_MODE}",
        "${RUN_MODE}-post",
        "${RUN_MODE} ",
        "${RUN_MODE}$$literal",
    ] {
        let root = Scratch::new()?;
        let input = root.source(&format!("    environment:\n      APP_MODE: \"{value}\"\n"))?;
        let out = root.0.join("out");
        let result = command(&input, &out)
            .args([
                "--quadlet-defer",
                "app:RUN_MODE",
                "--quadlet-service-env-file",
                "app=/provider.env",
            ])
            .output()?;
        assert!(!result.status.success(), "{}", message(&result));
        assert!(!out.exists());
        assert!(message(&result).contains("include"));
    }
    let root = Scratch::new()?;
    let input = root.source("    environment: { APP_MODE: \"${RUN_MODE}\" }\n")?;
    let result = command(&input, &root.0.join("out"))
        .args([
            "--quadlet-defer",
            "app:RUN_MODE",
            "--quadlet-service-env",
            "app:RUN_MODE=",
            "--environment-values",
            "include",
        ])
        .output()?;
    assert!(result.status.success(), "{}", message(&result));
    Ok(())
}

#[test]
fn operators_excluded_styles_and_unsafe_binds_are_actionable() -> Result<(), Box<dyn Error>> {
    let fields = [
        "    environment: { APP_MODE: \"${RUN_MODE:-production}\" }\n",
        "    environment: { APP_MODE: \"${RUN_MODE-production}\" }\n",
        "    environment: { APP_MODE: \"${RUN_MODE:+production}\" }\n",
        "    environment: { APP_MODE: \"${RUN_MODE?required}\" }\n",
        "    environment: { APP_MODE: '${RUN_MODE}' }\n",
        "    environment:\n      APP_MODE: |\n        ${RUN_MODE}\n",
        "    volumes: [\"prefix/${RUN_MODE}:/data\"]\n",
        "    volumes: [\"${RUN_MODE}/cache.volume:/data\"]\n",
        "    volumes: [\"${RUN_MODE}/data.artifact:/data\"]\n",
        "    volumes: [\"${RUN_MODE}/${RUN_MODE}:/data\"]\n",
        "    volumes:\n      - type: volume\n        source: ${RUN_MODE}\n        target: /data\n",
    ];
    for (index, field) in fields.iter().enumerate() {
        let root = Scratch::new()?;
        let input = root.source(field)?;
        let out = root.0.join("out");
        let result = command(&input, &out)
            .args([
                "--quadlet-defer",
                "app:RUN_MODE",
                "--quadlet-service-env-file",
                "app=/provider.env",
                "--environment-values",
                "include",
            ])
            .output()?;
        assert!(!result.status.success(), "case {index}: {}", message(&result));
        assert!(!out.exists());
    }
    Ok(())
}

#[test]
fn inline_bind_roots_are_checked_only_when_no_file_can_override() -> Result<(), Box<dyn Error>> {
    for (index, value) in ["", "relative", "/bad:delimiter", "/srv/data"].iter().enumerate() {
        let root = Scratch::new()?;
        let input = root.source("    volumes: [\"${ROOT}/config:/config:ro\"]\n")?;
        let out = root.0.join("out");
        let assignment = format!("app:ROOT={value}");
        let result = command(&input, &out)
            .args([
                "--quadlet-defer",
                "app:ROOT",
                "--quadlet-service-env",
                &assignment,
                "--environment-values",
                "include",
            ])
            .output()?;
        assert_eq!(result.status.success(), index == 3, "{}", message(&result));
        let result = command(&input, &root.0.join("files"))
            .args([
                "--quadlet-defer",
                "app:ROOT",
                "--quadlet-service-env",
                &assignment,
                "--quadlet-service-env-file",
                "app=/external.env",
                "--environment-values",
                "include",
            ])
            .output()?;
        assert!(result.status.success(), "{}", message(&result));
    }
    Ok(())
}

#[test]
fn deferred_bundles_preserve_native_bytes_and_validation_writes_nothing() -> Result<(), Box<dyn Error>> {
    let root = Scratch::new()?;
    let input = root.source(
        "    environment: { APP_MODE: \"${RUN_MODE}\" }\n    volumes: [\"${DATA_ROOT}/config:/config:ro\"]\n",
    )?;
    let args = [
        "--quadlet-defer",
        "app:RUN_MODE",
        "--quadlet-defer",
        "app:DATA_ROOT",
        "--quadlet-service-env-file",
        "app=/external/$ROOT/%h.env",
    ];
    let out = root.0.join("native");
    let native = command(&input, &out).args(args).output()?;
    assert!(native.status.success(), "{}", message(&native));
    let text = fs::read(out.join("app.container"))?;
    assert!(String::from_utf8_lossy(&text).contains("EnvironmentFile=\"/external/$ROOT/%%h.env\""));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(out.join("app.container"))?.permissions().mode() & 0o777,
            0o600
        );
    }
    let bundled = root.0.join("bundled");
    let result = command(&input, &bundled)
        .args(args)
        .args(["--output-layout", "quadlet-bundle"])
        .output()?;
    assert!(result.status.success(), "{}", message(&result));
    let bundle = boxferry::quadlet_bundle::QuadletBundle::decode(&fs::read(bundled.join("quadlet.bundle"))?)?;
    assert_eq!(bundle.members().len(), 1);
    assert_eq!(bundle.members()[0].text().as_bytes(), text);
    let result = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["validate", "compose", "quadlet", "--input-file"])
        .current_dir(&root.0)
        .arg(&input)
        .args(args)
        .args(["--console-format", "json"])
        .output()?;
    assert!(result.status.success(), "{}", message(&result));
    assert_eq!(
        json(&result)?["output_artifacts"]
            .as_array()
            .ok_or("planned artifacts")?
            .len(),
        1
    );
    assert_eq!(fs::read_dir(&root.0)?.count(), 3);
    assert!(!root.0.join("quadlet-output").exists());
    Ok(())
}

#[test]
fn inactive_services_and_cross_service_missing_providers_are_invalid() -> Result<(), Box<dyn Error>> {
    let root = Scratch::new()?;
    let input = root.source("    profiles: [optional]\n    environment: { APP_MODE: \"${RUN_MODE}\" }\n")?;
    let result = command(&input, &root.0.join("inactive"))
        .args([
            "--quadlet-defer",
            "app:RUN_MODE",
            "--quadlet-service-env-file",
            "app=/provider.env",
            "--loss-policy",
            "partial",
        ])
        .output()?;
    assert!(!result.status.success());
    assert!(message(&result).contains("BFC0005"));
    assert!(!root.0.join("inactive").exists());
    let input = root.source("    environment: { APP_MODE: \"${RUN_MODE}\" }\n  worker:\n    image: example.invalid/worker:1\n    restart: 'no'\n    environment: { WORKER_MODE: \"${RUN_MODE}\" }\n")?;
    let result = command(&input, &root.0.join("cross"))
        .args([
            "--quadlet-defer",
            "app:RUN_MODE",
            "--quadlet-defer",
            "worker:RUN_MODE",
            "--quadlet-service-env-file",
            "app=/provider.env",
            "--loss-policy",
            "partial",
        ])
        .output()?;
    assert!(!result.status.success());
    assert!(message(&result).contains("BFC0001"));
    assert!(!root.0.join("cross").exists());
    Ok(())
}

#[test]
fn interpolation_provider_literals_are_not_reparsed_and_legacy_expressions_still_forward() -> Result<(), Box<dyn Error>>
{
    for (index, (value, expected)) in [
        ("$OTHER", "$$OTHER"),
        ("${OTHER}", "$${OTHER}"),
        ("$$", "$$$$"),
        ("%h", "%%h"),
    ]
    .iter()
    .enumerate()
    {
        let root = Scratch::new()?;
        let input = root.source("    environment: { APP_MODE: \"${RUN_MODE}\" }\n")?;
        let out = root.0.join("out");
        let assignment = format!("RUN_MODE={value}");
        let result = command(&input, &out)
            .args(["--env", &assignment, "--environment-values", "include"])
            .output()?;
        assert!(result.status.success(), "case {index}: {}", message(&result));
        let text = fs::read_to_string(out.join("app.container"))?;
        assert!(text.contains(&format!("APP_MODE={expected}")), "case {index}: {text}");
        if value.contains('$') {
            let result = Command::new(env!("CARGO_BIN_EXE_boxferry"))
                .args(["convert", "compose", "compose", "--input-file"])
                .arg(&input)
                .arg("--output-directory")
                .arg(root.0.join("compose"))
                .args([
                    "--env",
                    &assignment,
                    "--environment-values",
                    "include",
                    "--console-format",
                    "json",
                ])
                .output()?;
            assert!(!result.status.success(), "case {index}: {}", message(&result));
            assert!(message(&result).contains("BFC0007"));
            assert!(!root.0.join("compose").exists());
        }
    }
    let root = Scratch::new()?;
    let input = root.source("    environment: { APP_MODE: \"${UNRESOLVED}\" }\n")?;
    let out = root.0.join("legacy");
    let result = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["convert", "compose", "compose", "--input-file"])
        .arg(&input)
        .arg("--output-directory")
        .arg(&out)
        .args(["--environment-values", "include", "--console-format", "json"])
        .output()?;
    assert!(result.status.success(), "{}", message(&result));
    assert!(fs::read_to_string(out.join("compose.yaml"))?.contains("APP_MODE=${UNRESOLVED}"));
    Ok(())
}

#[test]
fn eligible_authored_dollar_escapes_stay_opaque_without_deferral() -> Result<(), Box<dyn Error>> {
    for value in ["$$HOME", "$${HOME}", "$$"] {
        let root = Scratch::new()?;
        let input = root.source(&format!("    environment:\n      VALUE: \"{value}\"\n"))?;
        let out = root.0.join("out");
        let result = Command::new(env!("CARGO_BIN_EXE_boxferry"))
            .args(["convert", "compose", "compose", "--input-file"])
            .arg(&input)
            .arg("--output-directory")
            .arg(&out)
            .args(["--environment-values", "include", "--console-format", "json"])
            .output()?;
        assert!(result.status.success(), "{}", message(&result));
        assert!(fs::read_to_string(out.join("compose.yaml"))?.contains(&format!("VALUE={value}")));
    }
    Ok(())
}

#[test]
fn excluded_environment_styles_never_become_active_source_expressions() -> Result<(), Box<dyn Error>> {
    let fields = [
        "    environment: { VALUE: '${PRIVATE_LITERAL}' }\n",
        "    environment: { VALUE: '$$HOME' }\n",
        "    environment:\n      VALUE: |-\n        ${PRIVATE_LITERAL}\n",
        "    environment:\n      VALUE: >-\n        $$HOME\n",
    ];
    for field in fields {
        let root = Scratch::new()?;
        let input = root.source(field)?;
        let out = root.0.join("out");
        let result = Command::new(env!("CARGO_BIN_EXE_boxferry"))
            .args(["convert", "compose", "compose", "--input-file"])
            .arg(&input)
            .arg("--output-directory")
            .arg(&out)
            .args(["--environment-values", "include", "--console-format", "json"])
            .output()?;
        assert!(!result.status.success(), "{}", message(&result));
        assert!(!out.exists());
        assert!(message(&result).contains("BFC0004"));
        assert!(!message(&result).contains("PRIVATE_LITERAL"));
    }
    Ok(())
}

#[test]
fn overlay_dollar_escapes_stay_literal_even_without_sensitive_substitution() -> Result<(), Box<dyn Error>> {
    for (value, expected) in [("$$HOME", "$$HOME"), ("$${HOME}", "$${HOME}"), ("$$", "$$")] {
        let root = Scratch::new()?;
        let input = root.source(&format!("    environment:\n      VALUE: \"{value}\"\n"))?;
        let env_file = root.0.join("empty.env");
        fs::write(&env_file, "")?;
        let out = root.0.join("out");
        let result = command(&input, &out)
            .arg("--env-file")
            .arg(&env_file)
            .args(["--environment-values", "include"])
            .output()?;
        assert!(result.status.success(), "{}", message(&result));
        assert!(fs::read_to_string(out.join("app.container"))?.contains(&format!("VALUE={expected}")));
        let result = Command::new(env!("CARGO_BIN_EXE_boxferry"))
            .args(["convert", "compose", "compose", "--input-file"])
            .arg(&input)
            .arg("--env-file")
            .arg(&env_file)
            .arg("--output-directory")
            .arg(root.0.join("compose"))
            .args(["--environment-values", "include", "--console-format", "json"])
            .output()?;
        assert!(!result.status.success(), "{}", message(&result));
        assert!(message(&result).contains("BFC0007"));
        assert!(!root.0.join("compose").exists());
    }
    Ok(())
}

#[test]
fn overlay_mount_source_and_target_dollars_and_percents_stay_literal_or_explicitly_unsupported()
-> Result<(), Box<dyn Error>> {
    for (root_value, succeeds) in [("/srv/%h", true), ("/srv/$ROOT", false)] {
        let root = Scratch::new()?;
        let input = root.source("    volumes: [\"${ROOT}:/data/$$TARGET/%h:ro\"]\n")?;
        let out = root.0.join("out");
        let assignment = format!("ROOT={root_value}");
        let result = command(&input, &out).args(["--env", &assignment]).output()?;
        assert_eq!(result.status.success(), succeeds, "{}", message(&result));
        if succeeds {
            let text = fs::read_to_string(out.join("app.container"))?;
            assert!(text.contains("Volume=/srv/%%h:/data/$$TARGET/%%h:ro"), "{text}");
            let source = boxferry::QuadletSource::parse(
                boxferry::Identifier::new("literal")?,
                [boxferry::QuadletDocumentInput::new(
                    "app.container",
                    boxferry::quadlet::quadlet_lens::source::SourceId::new(1),
                    text,
                )],
            )?
            .into_source();
            let imported = boxferry::ImportAdapter::import(&boxferry::QuadletImporter::new()?, &source);
            let mount = imported.application().ok_or("application")?.services()[0]
                .value()
                .mounts()[0]
                .value();
            assert!(mount.paths_are_literal());
            assert_eq!(mount.target(), "/data/$TARGET/%h");
        } else {
            assert!(!out.exists());
            assert!(message(&result).contains("BFQ0003"));
        }
    }
    Ok(())
}
