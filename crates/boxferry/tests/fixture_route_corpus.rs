//! Capability-driven conversion coverage for every positive importer fixture.

#![cfg(all(feature = "cli", feature = "compose", feature = "podman", feature = "quadlet"))]

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::Deserialize;

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

const EXPECTED_FIXTURE_IDS: [&str; 7] = [
    "compose-import-core",
    "compose-to-quadlet-core",
    "compose-to-quadlet-dependencies",
    "compose-to-quadlet-interpolation",
    "compose-to-quadlet-pod",
    "compose-to-quadlet-secrets",
    "document-route-matrix",
];
const EXPECTED_SCENARIO_IDS: [&str; 10] = [
    "compose-import-core/compose",
    "compose-to-quadlet-core/compose",
    "compose-to-quadlet-dependencies/compose",
    "compose-to-quadlet-interpolation/compose",
    "compose-to-quadlet-pod/compose",
    "compose-to-quadlet-secrets/compose",
    "document-route-matrix/lossy-compose",
    "document-route-matrix/normal-compose",
    "document-route-matrix/normal-quadlet",
    "document-route-matrix/tmpfs-compose",
];

#[derive(Debug, Deserialize)]
struct FixtureManifest {
    id: String,
    files: BTreeSet<String>,
    extensions: Extensions,
}

#[derive(Debug, Deserialize)]
struct Extensions {
    #[serde(default)]
    scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Scenario {
    id: String,
    input: String,
    sources: Vec<String>,
    application_name: Option<String>,
    #[serde(default)]
    interpolate: bool,
    #[serde(default)]
    environment: Vec<String>,
    #[serde(default)]
    protected_values: Vec<String>,
    exports: BTreeMap<String, ExportExpectation>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct ExportExpectation {
    loss_policy: String,
    #[serde(default)]
    diagnostic_codes: Vec<String>,
    #[serde(default)]
    fixed_diagnostic_codes: Vec<String>,
    #[serde(default)]
    artifact_values: Vec<String>,
    #[serde(default)]
    normalize_project_root: bool,
    grouping: Option<String>,
    pod_name: Option<String>,
    podman_minimum: Option<String>,
    podman_maximum: Option<String>,
    #[serde(default)]
    podman_tmpfs_destinations: Vec<String>,
    artifacts: Vec<ArtifactExpectation>,
}

#[derive(Debug, Deserialize)]
struct ArtifactExpectation {
    artifact: String,
    expected: String,
}

#[test]
#[allow(clippy::too_many_lines)]
fn every_positive_importer_fixture_covers_every_registered_exporter() -> Result<(), Box<dyn Error>> {
    let repository = repository_root();
    let manifests = fixture_manifests(&repository)?;
    let capabilities = capability_routes()?;
    let registered_inputs = capabilities
        .keys()
        .filter(|input| matches!(input.as_str(), "compose" | "quadlet"))
        .cloned()
        .collect::<BTreeSet<_>>();
    let root = TemporaryDirectory::new("fixture-route-corpus")?;
    let mut covered_inputs = BTreeSet::new();
    let mut scenario_ids = BTreeSet::new();
    let mut fixture_ids = BTreeSet::new();

    for manifest_path in manifests {
        let fixture = manifest_path.parent().ok_or("fixture directory")?;
        let manifest: FixtureManifest = toml::from_str(&fs::read_to_string(&manifest_path)?)?;
        assert!(
            fixture_ids.insert(manifest.id.clone()),
            "duplicate fixture id {}",
            manifest.id
        );
        assert!(
            !manifest.extensions.scenarios.is_empty(),
            "{} has no positive importer scenario coverage",
            manifest_path.display()
        );

        for scenario in &manifest.extensions.scenarios {
            let scenario_name = format!("{}/{}", manifest.id, scenario.id);
            assert!(
                scenario_ids.insert(scenario_name.clone()),
                "duplicate fixture scenario {scenario_name}"
            );
            assert!(!scenario.sources.is_empty(), "{scenario_name} has no sources");
            assert!(
                registered_inputs.contains(&scenario.input),
                "{scenario_name} uses unregistered importer {}",
                scenario.input
            );
            covered_inputs.insert(scenario.input.clone());

            let registered_outputs = capabilities
                .get(&scenario.input)
                .ok_or("registered outputs for importer")?;
            assert_eq!(
                scenario.exports.keys().cloned().collect::<BTreeSet<_>>(),
                *registered_outputs,
                "{scenario_name} must declare every registered exporter"
            );

            let sources = scenario
                .sources
                .iter()
                .map(|source| {
                    assert!(
                        manifest.files.contains(source),
                        "{scenario_name} source {source} is absent from fixture files"
                    );
                    fixture.join(source)
                })
                .collect::<Vec<_>>();

            for (output, expectation) in &scenario.exports {
                assert!(
                    !expectation.artifacts.is_empty(),
                    "{scenario_name} -> {output} has no artifact expectations"
                );
                for artifact in &expectation.artifacts {
                    assert!(
                        manifest.files.contains(&artifact.expected),
                        "{scenario_name} -> {output} expectation {} is absent from fixture files",
                        artifact.expected
                    );
                }

                for blocked_policy in blocked_policies(&expectation.loss_policy)? {
                    let blocked_directory = root.path().join(format!(
                        "{}-{output}-blocked-{blocked_policy}",
                        safe_name(&scenario_name)
                    ));
                    let blocked = execute_conversion(
                        &scenario.input,
                        output,
                        &sources,
                        scenario.application_name.as_deref(),
                        Some(scenario),
                        expectation,
                        blocked_policy,
                        &blocked_directory,
                    )?;
                    assert!(
                        !blocked.status.success(),
                        "{scenario_name} -> {output} unexpectedly accepted {blocked_policy}"
                    );
                    assert!(
                        blocked.stderr.is_empty(),
                        "{}",
                        String::from_utf8_lossy(&blocked.stderr)
                    );
                    let report: serde_json::Value = serde_json::from_slice(&blocked.stdout)?;
                    assert_eq!(report["status"], "blocked");
                    assert_eq!(report["exit_category"], "policy-blocked");
                    assert!(!blocked_directory.exists());
                    assert_report_redacted(&report, &scenario.protected_values, &scenario_name);
                }

                let output_directory = root
                    .path()
                    .join(format!("{}-{output}-authorized", safe_name(&scenario_name)));
                let result = execute_conversion(
                    &scenario.input,
                    output,
                    &sources,
                    scenario.application_name.as_deref(),
                    Some(scenario),
                    expectation,
                    &expectation.loss_policy,
                    &output_directory,
                )?;
                assert!(
                    result.status.success(),
                    "{scenario_name} -> {output} failed: stdout={} stderr={}",
                    String::from_utf8_lossy(&result.stdout),
                    String::from_utf8_lossy(&result.stderr)
                );
                assert!(result.stderr.is_empty());
                let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
                assert_eq!(report["status"], "success");
                assert_eq!(report["source_type"], scenario.input);
                assert_eq!(report["target_type"], *output);
                assert_eq!(
                    report_diagnostic_codes(&report)?,
                    expectation
                        .diagnostic_codes
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>(),
                    "{scenario_name} -> {output} diagnostic sequence changed"
                );
                assert_report_redacted(&report, &scenario.protected_values, &scenario_name);
                assert_expected_artifacts(fixture, &output_directory, expectation, &scenario_name, output)?;
                assert_artifact_values(&output_directory, &expectation.artifact_values, &scenario_name, output)?;

                if output == "podman" {
                    assert_podman_artifacts(
                        &output_directory,
                        &scenario.protected_values,
                        &expectation.artifact_values,
                        &expectation.podman_tmpfs_destinations,
                        &scenario_name,
                    )?;
                    let repeated_directory = root
                        .path()
                        .join(format!("{}-{output}-repeated", safe_name(&scenario_name)));
                    let repeated = execute_conversion(
                        &scenario.input,
                        output,
                        &sources,
                        scenario.application_name.as_deref(),
                        Some(scenario),
                        expectation,
                        &expectation.loss_policy,
                        &repeated_directory,
                    )?;
                    assert!(
                        repeated.status.success(),
                        "{scenario_name} -> {output} repeat failed: stdout={} stderr={}",
                        String::from_utf8_lossy(&repeated.stdout),
                        String::from_utf8_lossy(&repeated.stderr)
                    );
                    assert!(repeated.stderr.is_empty());
                    let repeated_report: serde_json::Value = serde_json::from_slice(&repeated.stdout)?;
                    assert_eq!(
                        report_diagnostic_codes(&repeated_report)?,
                        expectation
                            .diagnostic_codes
                            .iter()
                            .map(String::as_str)
                            .collect::<Vec<_>>(),
                        "{scenario_name} -> {output} repeat diagnostic sequence changed"
                    );
                    assert_report_redacted(&repeated_report, &scenario.protected_values, &scenario_name);
                    assert_podman_artifacts(
                        &repeated_directory,
                        &scenario.protected_values,
                        &expectation.artifact_values,
                        &expectation.podman_tmpfs_destinations,
                        &scenario_name,
                    )?;
                    assert_eq!(
                        normalized_artifacts(&output_directory, fixture, expectation.normalize_project_root,)?,
                        normalized_artifacts(&repeated_directory, fixture, expectation.normalize_project_root,)?,
                        "{scenario_name} -> Podman is not deterministic"
                    );
                    continue;
                }

                let fixed_directory = root
                    .path()
                    .join(format!("{}-{output}-fixed", safe_name(&scenario_name)));
                let generated_sources = artifact_paths(&output_directory)?;
                let fixed_application_name = if output == "quadlet" {
                    Some(
                        report["application"]
                            .as_str()
                            .ok_or_else(|| format!("{scenario_name} missing application name for Quadlet re-import"))?,
                    )
                } else {
                    None
                };
                let fixed = execute_conversion(
                    output,
                    output,
                    &generated_sources,
                    fixed_application_name,
                    None,
                    expectation,
                    "partial",
                    &fixed_directory,
                )?;
                assert!(
                    fixed.status.success(),
                    "{scenario_name} generated {output} did not re-import: stdout={} stderr={}",
                    String::from_utf8_lossy(&fixed.stdout),
                    String::from_utf8_lossy(&fixed.stderr)
                );
                assert!(fixed.stderr.is_empty(), "{}", String::from_utf8_lossy(&fixed.stderr));
                let fixed_report: serde_json::Value = serde_json::from_slice(&fixed.stdout)?;
                assert_eq!(fixed_report["status"], "success");
                assert_eq!(fixed_report["source_type"], *output);
                assert_eq!(fixed_report["target_type"], *output);
                assert_eq!(
                    report_diagnostic_codes(&fixed_report)?,
                    expectation
                        .fixed_diagnostic_codes
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>(),
                    "{scenario_name} generated {output} diagnostic sequence changed at its fixed point"
                );
                assert_report_redacted(&fixed_report, &scenario.protected_values, &scenario_name);
                assert_eq!(
                    normalized_artifacts(&output_directory, fixture, expectation.normalize_project_root)?,
                    normalized_artifacts(&fixed_directory, fixture, expectation.normalize_project_root)?,
                    "{scenario_name} -> {output} did not reach a deterministic fixed point"
                );
            }
        }
    }

    assert_eq!(
        fixture_ids,
        EXPECTED_FIXTURE_IDS.into_iter().map(str::to_owned).collect(),
        "positive fixture manifest inventory changed; update the explicit corpus ratchet"
    );
    assert_eq!(
        scenario_ids,
        EXPECTED_SCENARIO_IDS.into_iter().map(str::to_owned).collect(),
        "positive fixture scenario inventory changed; update the explicit corpus ratchet"
    );
    assert_eq!(
        covered_inputs, registered_inputs,
        "fixture corpus must contain at least one scenario for every registered importer"
    );
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn execute_conversion(
    input: &str,
    output: &str,
    sources: &[PathBuf],
    application_name: Option<&str>,
    scenario: Option<&Scenario>,
    expectation: &ExportExpectation,
    policy: &str,
    output_directory: &Path,
) -> Result<Output, Box<dyn Error>> {
    let mut command = boxferry_command();
    command.args(["convert", input, output]);
    for source in sources {
        command.arg("--input-file").arg(source);
    }
    if let Some(application_name) = application_name {
        command.args(["--application-name", application_name]);
    }
    if let Some(scenario) = scenario {
        if scenario.interpolate {
            command.arg("--interpolate");
        }
        for assignment in &scenario.environment {
            command.arg(format!("--env={assignment}"));
        }
    }
    if output == "quadlet" {
        if let Some(minimum) = expectation.podman_minimum.as_deref() {
            command.args(["--podman-minimum-version", minimum]);
        }
        if let Some(maximum) = expectation.podman_maximum.as_deref() {
            command.args(["--podman-maximum-version", maximum]);
        }
        if let Some(grouping) = expectation.grouping.as_deref() {
            command.args(["--quadlet-grouping", grouping]);
        }
        if let Some(pod_name) = expectation.pod_name.as_deref() {
            command.args(["--pod-name", pod_name]);
        }
    }
    if output == "podman" {
        command.args(["--podman-target-context", "unknown"]);
    }
    Ok(command
        .args(["--loss-policy", policy, "--output-directory"])
        .arg(output_directory)
        .args(["--console-format", "json"])
        .output()?)
}

fn blocked_policies(minimum: &str) -> Result<&'static [&'static str], Box<dyn Error>> {
    match minimum {
        "exact" => Ok(&[]),
        "approximate" => Ok(&["exact"]),
        "partial" => Ok(&["exact", "approximate"]),
        value => Err(format!("unsupported fixture loss policy {value}").into()),
    }
}

fn assert_expected_artifacts(
    fixture: &Path,
    output_directory: &Path,
    expectation: &ExportExpectation,
    scenario: &str,
    output: &str,
) -> Result<(), Box<dyn Error>> {
    let expected_names = expectation
        .artifacts
        .iter()
        .map(|artifact| artifact.artifact.clone())
        .collect::<BTreeSet<_>>();
    let actual_names = artifact_names(output_directory)?.into_iter().collect::<BTreeSet<_>>();
    assert_eq!(
        actual_names, expected_names,
        "{scenario} -> {output} artifact set changed"
    );

    let project_root = fixture.canonicalize()?.to_string_lossy().into_owned();
    for artifact in &expectation.artifacts {
        let mut actual = fs::read_to_string(output_directory.join(&artifact.artifact))?;
        if expectation.normalize_project_root {
            actual = actual.replace(&project_root, "<project>");
        }
        assert_eq!(
            actual,
            fs::read_to_string(fixture.join(&artifact.expected))?,
            "{scenario} -> {output} artifact {} changed",
            artifact.artifact
        );
    }
    Ok(())
}

fn assert_artifact_values(
    output_directory: &Path,
    values: &[String],
    scenario: &str,
    output: &str,
) -> Result<(), Box<dyn Error>> {
    let mut artifacts = String::new();
    for path in artifact_paths(output_directory)? {
        artifacts.push_str(&fs::read_to_string(path)?);
    }
    for value in values {
        assert!(
            artifacts.contains(value),
            "{scenario} -> {output} did not retain authorized protected artifact value"
        );
    }
    Ok(())
}

fn assert_podman_artifacts(
    output_directory: &Path,
    protected_values: &[String],
    authorized_values: &[String],
    tmpfs_destinations: &[String],
    scenario: &str,
) -> Result<(), Box<dyn Error>> {
    let deployment_text = fs::read_to_string(output_directory.join("podman.json"))?;
    let review = fs::read_to_string(output_directory.join("review.sh"))?;
    let deployment: serde_json::Value = serde_json::from_str(&deployment_text)?;

    assert_eq!(deployment["schema_version"], 1, "{scenario} Podman schema changed");
    assert!(
        matches!(
            deployment["status"].as_str(),
            Some("exact" | "deferred_sensitive_input")
        ),
        "{scenario} Podman deployment has an unexpected status"
    );
    assert!(
        deployment["connection"].is_null(),
        "{scenario} Podman output must remain connection-independent"
    );
    assert!(
        deployment["external_preconditions"].is_array(),
        "{scenario} Podman preconditions must be explicit"
    );

    let operations = deployment["operations"]
        .as_array()
        .ok_or_else(|| format!("{scenario} Podman operations array missing"))?;
    assert!(!operations.is_empty(), "{scenario} Podman deployment has no operations");
    assert_podman_operation_equivalence(operations, &review, scenario)?;
    assert_podman_tmpfs_evidence(operations, &review, tmpfs_destinations, scenario)?;

    assert!(review.starts_with("#!/bin/sh\n"));
    assert!(review.contains("# Review generated Podman commands before running this file.\n"));
    assert!(review.contains("set -eu\n"));
    for protected in protected_values {
        let explicitly_authorized = authorized_values
            .iter()
            .any(|authorized| authorized.contains(protected) || protected.contains(authorized));
        if !explicitly_authorized {
            assert!(
                !deployment_text.contains(protected) && !review.contains(protected),
                "{scenario} Podman artifacts disclosed an unretained protected value"
            );
        }
    }
    Ok(())
}

fn assert_podman_operation_equivalence(
    operations: &[serde_json::Value],
    review: &str,
    scenario: &str,
) -> Result<(), Box<dyn Error>> {
    let command_lines = review
        .lines()
        .filter(|line| line.starts_with("podman "))
        .collect::<Vec<_>>();
    assert_eq!(
        command_lines.len(),
        operations.len(),
        "{scenario} review script operation count changed"
    );

    for (operation, command_line) in operations.iter().zip(command_lines) {
        assert!(
            operation["status"].as_str().is_some(),
            "{scenario} Podman operation status missing"
        );
        assert!(
            operation["action"].as_str().is_some_and(|action| !action.is_empty()),
            "{scenario} Podman operation action missing"
        );
        assert!(
            operation["resource"]["kind"]
                .as_str()
                .is_some_and(|kind| !kind.is_empty()),
            "{scenario} Podman operation resource kind missing"
        );
        assert!(
            operation["resource"]["name"]
                .as_str()
                .is_some_and(|name| !name.is_empty()),
            "{scenario} Podman operation resource name missing"
        );
        assert_eq!(operation["cli"]["program"], "podman");
        let arguments = operation["cli"]["argv"]
            .as_array()
            .ok_or_else(|| format!("{scenario} Podman CLI arguments missing"))?
            .iter()
            .map(|argument| {
                argument
                    .as_str()
                    .ok_or_else(|| format!("{scenario} Podman CLI argument is not text"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        assert!(!arguments.is_empty(), "{scenario} Podman CLI operation is empty");
        let expected_command = format!(
            "podman {}",
            arguments
                .iter()
                .map(|argument| shell_quote(argument))
                .collect::<Vec<_>>()
                .join(" ")
        );
        assert!(
            command_line.starts_with(&expected_command),
            "{scenario} review command differs from podman.json: {command_line}"
        );
        assert_eq!(operation["libpod"]["method"], "POST");
        assert!(
            operation["libpod"]["path_and_query"]
                .as_str()
                .is_some_and(|path| path.starts_with("/v6.1.0/libpod/")),
            "{scenario} Podman operation does not use the default reviewed 6.1 target"
        );
        assert!(
            operation["libpod"]["body"].is_object(),
            "{scenario} Podman operation body missing"
        );

        if operation["cli"]["external_sensitive_input_required"] == true {
            assert_eq!(operation["libpod"]["body"]["kind"], "external_sensitive_input");
            assert!(
                review.contains("PODMAN_LENS_SECRET_INPUT_"),
                "{scenario} sensitive operation lacks an explicit review placeholder"
            );
        }
    }
    Ok(())
}

fn assert_podman_tmpfs_evidence(
    operations: &[serde_json::Value],
    review: &str,
    destinations: &[String],
    scenario: &str,
) -> Result<(), Box<dyn Error>> {
    for destination in destinations {
        let mount_argument = format!("type=tmpfs,target={destination}");
        let create = operations
            .iter()
            .find(|operation| {
                operation["action"] == "create"
                    && operation["resource"]["kind"] == "container"
                    && operation["cli"]["argv"].as_array().is_some_and(|arguments| {
                        arguments
                            .windows(2)
                            .any(|pair| pair[0] == "--mount" && pair[1] == mount_argument)
                    })
            })
            .ok_or_else(|| {
                format!("{scenario} lacks a container create operation for tmpfs destination {destination}")
            })?;
        assert_eq!(create["status"], "exact");
        let mounts = create["libpod"]["body"]["json"]["mounts"]
            .as_array()
            .ok_or_else(|| format!("{scenario} tmpfs Libpod mounts missing"))?;
        let mount = mounts
            .iter()
            .find(|mount| {
                mount["destination"] == *destination && mount["source"] == "tmpfs" && mount["type"] == "tmpfs"
            })
            .ok_or_else(|| format!("{scenario} lacks Libpod tmpfs evidence for destination {destination}"))?;
        assert!(
            mount["options"]
                .as_array()
                .is_some_and(|options| options.iter().any(|option| option == "rw")),
            "{scenario} tmpfs mount does not retain explicit writable access"
        );
        assert!(
            review.contains(&format!("'--mount' {}", shell_quote(&mount_argument))),
            "{scenario} review script lacks tmpfs create evidence"
        );
    }
    Ok(())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn assert_report_redacted(report: &serde_json::Value, protected_values: &[String], scenario: &str) {
    for value in protected_values {
        assert!(
            !json_contains(report, value),
            "{scenario} report disclosed a protected value"
        );
    }
}

fn json_contains(document: &serde_json::Value, needle: &str) -> bool {
    match document {
        serde_json::Value::String(value) => value.contains(needle),
        serde_json::Value::Array(values) => values.iter().any(|value| json_contains(value, needle)),
        serde_json::Value::Object(entries) => entries
            .iter()
            .any(|(key, value)| key.contains(needle) || json_contains(value, needle)),
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => false,
    }
}

fn report_diagnostic_codes(report: &serde_json::Value) -> Result<Vec<&str>, String> {
    report["diagnostics"]
        .as_array()
        .ok_or_else(|| "report diagnostics array".to_owned())?
        .iter()
        .map(|diagnostic| diagnostic["code"].as_str().ok_or_else(|| "diagnostic code".to_owned()))
        .collect()
}

fn normalized_artifacts(
    directory: &Path,
    fixture: &Path,
    normalize_project_root: bool,
) -> Result<BTreeMap<String, String>, Box<dyn Error>> {
    let project_root = fixture.canonicalize()?.to_string_lossy().into_owned();
    artifact_names(directory)?
        .into_iter()
        .map(|name| {
            let mut contents = fs::read_to_string(directory.join(&name))?;
            if normalize_project_root {
                contents = contents.replace(&project_root, "<project>");
            }
            Ok((name, contents))
        })
        .collect()
}

fn capability_routes() -> Result<BTreeMap<String, BTreeSet<String>>, Box<dyn Error>> {
    let result = boxferry_command()
        .args(["capabilities", "--console-format", "json"])
        .output()?;
    if !result.status.success() {
        return Err(format!(
            "capability discovery failed: {}",
            String::from_utf8_lossy(&result.stderr)
        )
        .into());
    }
    let document: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    let mut capabilities = BTreeMap::<String, BTreeSet<String>>::new();
    for route in document["routes"].as_array().ok_or("capability routes")? {
        capabilities
            .entry(route["input_type"].as_str().ok_or("capability input")?.to_owned())
            .or_default()
            .insert(route["output_type"].as_str().ok_or("capability output")?.to_owned());
    }
    Ok(capabilities)
}

fn fixture_manifests(repository: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut manifests = Vec::new();
    collect_manifests(&repository.join("fixtures/adapter-contract"), &mut manifests)?;
    collect_manifests(&repository.join("fixtures/conversion"), &mut manifests)?;
    manifests.sort();
    Ok(manifests)
}

fn collect_manifests(directory: &Path, manifests: &mut Vec<PathBuf>) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_manifests(&path, manifests)?;
        } else if path.file_name().is_some_and(|name| name == "fixture.toml") {
            manifests.push(path);
        }
    }
    Ok(())
}

fn artifact_names(directory: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let mut names = fs::read_dir(directory)?
        .map(|entry| {
            entry?
                .file_name()
                .into_string()
                .map_err(|name| format!("non-UTF-8 artifact name: {}", PathBuf::from(name).display()).into())
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    names.sort();
    Ok(names)
}

fn artifact_paths(directory: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    Ok(artifact_names(directory)?
        .into_iter()
        .map(|name| directory.join(name))
        .collect())
}

fn safe_name(value: &str) -> String {
    value.replace('/', "-")
}

fn boxferry_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_boxferry"))
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new(label: &str) -> Result<Self, Box<dyn Error>> {
        let id = TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("boxferry-{label}-{}-{id}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
