//! Capability-driven conversion coverage for every positive importer fixture.
//!
//! Native renderer quote spelling is byte-checked; core mount intent is independently asserted
//! for both plain and quoted scalars so dependency-driven formatting changes cannot mask loss.

#![cfg(all(
    feature = "cli",
    feature = "compose",
    feature = "docker",
    feature = "podman",
    feature = "quadlet"
))]

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::Deserialize;
use sha2::{Digest, Sha256};

use boxferry::compose::compose_lens::{
    loader::{DocumentInput, DocumentOrigin, LoadedProject},
    merge::merge_project,
    source::SourceId as ComposeSourceId,
};
use boxferry::{ComposeImporter, ComposeSource, Identifier, ImportAdapter, MountSource, SelinuxRelabel};

static TEMP_ID: AtomicU64 = AtomicU64::new(0);

const EXPECTED_FIXTURE_IDS: [&str; 8] = [
    "compose-import-core",
    "compose-to-quadlet-core",
    "compose-to-quadlet-dependencies",
    "compose-to-quadlet-interpolation",
    "compose-to-quadlet-pod",
    "compose-to-quadlet-secrets",
    "document-route-matrix",
    "quadlet-native-values",
];
const EXPECTED_SCENARIO_IDS: [&str; 11] = [
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
    "quadlet-native-values/quadlet",
];

#[derive(Debug, Deserialize)]
struct FixtureManifest {
    id: String,
    secrets_reviewed: bool,
    files: BTreeSet<String>,
    provenance: FixtureProvenance,
    extensions: Extensions,
}

#[derive(Debug, Deserialize)]
struct FixtureProvenance {
    source: String,
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
    blocked: bool,
    terminal_status: Option<TerminalStatus>,
    invalid_outcomes: Option<u64>,
    terminal_diagnostic_orders: Option<TerminalDiagnosticOrders>,
    #[serde(default)]
    include_environment_values: bool,
    #[serde(default)]
    diagnostic_codes: Vec<String>,
    #[serde(default)]
    diagnostic_subjects: Vec<String>,
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
    #[serde(default)]
    artifacts: Vec<ArtifactExpectation>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
enum TerminalStatus {
    Blocked,
    Failure,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TerminalDiagnosticOrders {
    exact: Vec<usize>,
    approximate: Vec<usize>,
    partial: Vec<usize>,
}

impl TerminalDiagnosticOrders {
    fn for_policy(&self, policy: &str) -> Option<&[usize]> {
        match policy {
            "exact" => Some(&self.exact),
            "approximate" => Some(&self.approximate),
            "partial" => Some(&self.partial),
            _ => None,
        }
    }
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
                assert_eq!(
                    validate_terminal_expectation(expectation),
                    Ok(()),
                    "{scenario_name} -> {output} has an invalid terminal contract"
                );
                assert_eq!(
                    validate_value_inclusion(
                        expectation.include_environment_values,
                        manifest.secrets_reviewed,
                        &manifest.provenance.source,
                        output,
                    ),
                    Ok(()),
                    "{scenario_name} -> {output} has unauthorized value inclusion"
                );
                assert_eq!(
                    expectation.artifacts.is_empty(),
                    expectation.blocked,
                    "{scenario_name} -> {output} must declare artifacts exactly for successful routes"
                );
                for artifact in &expectation.artifacts {
                    assert!(
                        manifest.files.contains(&artifact.expected),
                        "{scenario_name} -> {output} expectation {} is absent from fixture files",
                        artifact.expected
                    );
                }

                for blocked_policy in terminal_or_stricter_policies(expectation)? {
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
                    let exit_category = report["exit_category"]
                        .as_str()
                        .filter(|value| {
                            matches!(
                                *value,
                                "success" | "policy-blocked" | "input-or-execution" | "output-write" | "report-write"
                            )
                        })
                        .unwrap_or("<missing-or-unknown>");
                    let primary_code = report["primary_diagnostic_code"]
                        .as_str()
                        .filter(|code| {
                            code.len() == 7
                                && ["BFC", "BFD", "BFP", "BFQ", "BFO"]
                                    .iter()
                                    .any(|prefix| code.starts_with(*prefix))
                                && code.as_bytes()[3..].iter().all(u8::is_ascii_digit)
                        })
                        .unwrap_or("<missing-or-unknown>");
                    assert_eq!(
                        validate_terminal_report_for_policy(
                            expectation,
                            &report,
                            blocked.status.code(),
                            path_entry_exists(&blocked_directory)?,
                            expectation.terminal_status == Some(TerminalStatus::Failure),
                            blocked_policy,
                        ),
                        Ok(()),
                        "scenario={scenario_name} route={}->{} blocked_policy={blocked_policy} exit_category={exit_category} primary_code={primary_code}; expected_facts=[{}]; actual_facts=[{}]",
                        scenario.input,
                        output,
                        terminal_diagnostic_facts(expectation, None, blocked_policy),
                        terminal_diagnostic_facts(expectation, Some(&report), blocked_policy),
                    );
                    assert_eq!(report["source_type"], scenario.input);
                    assert_eq!(report["target_type"], *output);
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
                if expectation.blocked {
                    assert_eq!(
                        output, "docker",
                        "{scenario_name} only Docker may declare a terminal block"
                    );
                    assert!(
                        !result.status.success(),
                        "{scenario_name} -> {output} unexpectedly produced a candidate"
                    );
                    assert!(result.stderr.is_empty());
                    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
                    assert_eq!(
                        validate_terminal_report(
                            expectation,
                            &report,
                            result.status.code(),
                            path_entry_exists(&output_directory)?,
                            true,
                        ),
                        Ok(()),
                        "{scenario_name} -> {output} terminal contract changed; expected_facts=[{}]; actual_facts=[{}]",
                        terminal_diagnostic_facts(expectation, None, &expectation.loss_policy),
                        terminal_diagnostic_facts(expectation, Some(&report), &expectation.loss_policy),
                    );
                    assert_eq!(report["source_type"], scenario.input);
                    assert_eq!(report["target_type"], *output);
                    assert_report_redacted(&report, &scenario.protected_values, &scenario_name);
                    continue;
                }
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
                if output == "docker" {
                    assert_eq!(
                        report_diagnostic_subjects(&report)?,
                        expectation
                            .diagnostic_subjects
                            .iter()
                            .map(String::as_str)
                            .collect::<Vec<_>>(),
                        "{scenario_name} -> Docker diagnostic subjects changed"
                    );
                }
                assert_report_redacted(&report, &scenario.protected_values, &scenario_name);
                assert_expected_artifacts(fixture, &output_directory, expectation, &scenario_name, output)?;
                assert_artifact_values(&output_directory, &expectation.artifact_values, &scenario_name, output)?;

                if output == "podman" || output == "docker" {
                    if output == "podman" {
                        assert_podman_artifacts(
                            &output_directory,
                            &scenario.protected_values,
                            &expectation.artifact_values,
                            &expectation.podman_tmpfs_destinations,
                            &scenario_name,
                        )?;
                    } else {
                        assert_docker_artifacts(
                            &output_directory,
                            &scenario.protected_values,
                            expectation.include_environment_values,
                            &expectation.artifact_values,
                            &scenario_name,
                        )?;
                    }
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
                    if output == "podman" {
                        assert_podman_artifacts(
                            &repeated_directory,
                            &scenario.protected_values,
                            &expectation.artifact_values,
                            &expectation.podman_tmpfs_destinations,
                            &scenario_name,
                        )?;
                    } else {
                        assert_docker_artifacts(
                            &repeated_directory,
                            &scenario.protected_values,
                            expectation.include_environment_values,
                            &expectation.artifact_values,
                            &scenario_name,
                        )?;
                    }
                    assert_eq!(
                        normalized_artifacts(&output_directory, fixture, expectation.normalize_project_root,)?,
                        normalized_artifacts(&repeated_directory, fixture, expectation.normalize_project_root,)?,
                        "{scenario_name} -> {output} is not deterministic"
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

#[test]
fn core_compose_goldens_preserve_mount_intent_with_plain_or_quoted_scalars() -> Result<(), Box<dyn Error>> {
    let repository = repository_root();
    let importer = ComposeImporter::new()?;
    let expected = [
        (
            MountSource::Volume(Identifier::new("data")?),
            "/var/lib/data",
            true,
            None,
        ),
        (
            MountSource::HostPath("./config".to_owned()),
            "/etc/app",
            true,
            Some(SelinuxRelabel::Private),
        ),
        (
            MountSource::HostPath("./shared".to_owned()),
            "/srv/shared",
            false,
            Some(SelinuxRelabel::Shared),
        ),
        (
            MountSource::HostPath("./data".to_owned()),
            "/srv/data",
            true,
            Some(SelinuxRelabel::Private),
        ),
    ];
    for path in [
        "fixtures/adapter-contract/compose-import-core/expected-matrix-compose.yaml",
        "fixtures/conversion/compose-to-quadlet-core/expected-matrix-compose.txt",
    ] {
        let plain = fs::read_to_string(repository.join(path))?;
        let quoted = plain
            .replace("- ./config:/etc/app:Z,ro", "- \"./config:/etc/app:Z,ro\"")
            .replace("- ./data:/srv/data:Z,ro", "- \"./data:/srv/data:Z,ro\"");
        assert_ne!(plain, quoted, "{path} must exercise both scalar spellings");
        for text in [&plain, &quoted] {
            let loaded = LoadedProject::load([DocumentInput::new(
                ComposeSourceId::new(1),
                DocumentOrigin::new("compose.yaml", "core-mount-intent"),
                text.as_str(),
            )])?;
            let merged = merge_project(&loaded, None);
            let project = merged.project().ok_or("merged Compose project")?.clone();
            let source = ComposeSource::new(project, Identifier::new("core-mount-intent")?)?;
            let result = importer.import(&source);
            assert!(result.diagnostics().is_empty(), "{path}: {:#?}", result.diagnostics());
            let application = result.application().ok_or("imported core application")?;
            let web = application
                .services()
                .iter()
                .find(|service| service.value().name().as_str() == "web")
                .ok_or("core web service")?
                .value();
            assert_eq!(web.mounts().len(), expected.len(), "{path}");
            for (mount, (source, target, read_only, relabel)) in web.mounts().iter().zip(&expected) {
                assert_eq!(mount.value().source(), source, "{path}");
                assert_eq!(mount.value().target(), *target, "{path}");
                assert_eq!(mount.value().read_only(), *read_only, "{path}");
                assert_eq!(mount.value().selinux_relabel(), *relabel, "{path}");
            }
        }
    }
    Ok(())
}

fn validate_value_inclusion(
    include: bool,
    secrets_reviewed: bool,
    provenance_source: &str,
    output: &str,
) -> Result<(), &'static str> {
    if !include {
        return Ok(());
    }
    if !secrets_reviewed || provenance_source != "authored" {
        return Err("value inclusion requires a privacy-reviewed authored fixture");
    }
    if output == "podman" {
        return Err("Podman output cannot safely render protected environment values");
    }
    Ok(())
}

#[test]
fn value_inclusion_requires_reviewed_authored_non_podman_output() {
    assert!(validate_value_inclusion(true, true, "authored", "compose").is_ok());
    assert!(validate_value_inclusion(true, true, "authored", "quadlet").is_ok());
    assert_eq!(
        validate_value_inclusion(true, false, "authored", "compose"),
        Err("value inclusion requires a privacy-reviewed authored fixture")
    );
    assert_eq!(
        validate_value_inclusion(true, true, "external", "quadlet"),
        Err("value inclusion requires a privacy-reviewed authored fixture")
    );
    assert_eq!(
        validate_value_inclusion(true, true, "authored", "podman"),
        Err("Podman output cannot safely render protected environment values")
    );
    assert!(validate_value_inclusion(false, false, "external", "podman").is_ok());
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
    if expectation.include_environment_values {
        assert!(
            output != "podman",
            "Podman output cannot safely render protected environment values"
        );
        command.args(["--environment-values", "include"]);
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
    if output == "docker" {
        command.args(["--docker-target", "upstream-29.8.1-rootful"]);
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

fn validate_terminal_expectation(expectation: &ExportExpectation) -> Result<(), String> {
    if (expectation.terminal_status.is_some()
        || expectation.invalid_outcomes.is_some()
        || expectation.terminal_diagnostic_orders.is_some())
        && (!expectation.blocked || !expectation.artifacts.is_empty())
    {
        return Err("explicit terminal fields require a blocked route without artifacts".to_owned());
    }
    if let Some(orders) = &expectation.terminal_diagnostic_orders {
        if expectation.terminal_status != Some(TerminalStatus::Failure) {
            return Err("terminal diagnostic orders require explicit failure".to_owned());
        }
        let count = expectation.diagnostic_codes.len();
        if count == 0 || count != expectation.diagnostic_subjects.len() {
            return Err("terminal diagnostic orders require complete paired base facts".to_owned());
        }
        let identity = (0..count).collect::<Vec<_>>();
        for order in [&orders.exact, &orders.approximate, &orders.partial] {
            if order.len() != count
                || order.iter().copied().collect::<BTreeSet<_>>() != identity.iter().copied().collect::<BTreeSet<_>>()
            {
                return Err("terminal diagnostic order is not a complete unique permutation".to_owned());
            }
        }
        if orders.for_policy(&expectation.loss_policy) != Some(identity.as_slice()) {
            return Err("declared policy diagnostic order must retain the base identity order".to_owned());
        }
    }
    match expectation.terminal_status.unwrap_or(TerminalStatus::Blocked) {
        TerminalStatus::Failure if expectation.invalid_outcomes.is_none_or(|count| count == 0) => {
            Err("terminal failure requires a positive exact invalid-outcomes count".to_owned())
        }
        TerminalStatus::Blocked if expectation.invalid_outcomes.is_some() => {
            Err("policy-blocked routes cannot declare invalid-outcomes".to_owned())
        }
        _ => Ok(()),
    }
}

fn terminal_or_stricter_policies(expectation: &ExportExpectation) -> Result<Vec<&'static str>, Box<dyn Error>> {
    let stricter = blocked_policies(&expectation.loss_policy)?;
    if expectation.terminal_status == Some(TerminalStatus::Failure) {
        Ok(["exact", "approximate", "partial"]
            .into_iter()
            .filter(|policy| *policy != expectation.loss_policy.as_str())
            .collect())
    } else {
        Ok(stricter.to_vec())
    }
}

fn path_entry_exists(path: &Path) -> Result<bool, std::io::Error> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn terminal_expected_diagnostics<'a>(
    expectation: &'a ExportExpectation,
    policy: &str,
) -> Result<(Vec<&'a str>, Vec<&'a str>), String> {
    validate_terminal_expectation(expectation)?;
    if expectation.diagnostic_codes.len() != expectation.diagnostic_subjects.len() {
        return Err("terminal base diagnostic facts are not paired".to_owned());
    }
    let order = if let Some(orders) = &expectation.terminal_diagnostic_orders {
        orders
            .for_policy(policy)
            .ok_or("unknown terminal diagnostic policy")?
            .to_vec()
    } else {
        (0..expectation.diagnostic_codes.len()).collect()
    };
    let codes = order
        .iter()
        .map(|index| expectation.diagnostic_codes[*index].as_str())
        .collect();
    let subjects = order
        .iter()
        .map(|index| expectation.diagnostic_subjects[*index].as_str())
        .collect();
    Ok((codes, subjects))
}

fn terminal_diagnostic_facts(
    expectation: &ExportExpectation,
    report: Option<&serde_json::Value>,
    policy: &str,
) -> String {
    let (codes, subjects) = if let Some(report) = report {
        let (Ok(codes), Ok(subjects)) = (report_diagnostic_codes(report), report_diagnostic_subjects(report)) else {
            return "unavailable".to_owned();
        };
        (codes, subjects)
    } else {
        let Ok(facts) = terminal_expected_diagnostics(expectation, policy) else {
            return "unavailable".to_owned();
        };
        facts
    };
    let facts = codes
        .iter()
        .zip(&subjects)
        .take(16)
        .map(|(code, subject)| {
            let code = if code.len() == 7
                && ["BFC", "BFD", "BFP", "BFQ", "BFO"]
                    .iter()
                    .any(|prefix| code.starts_with(*prefix))
                && code.as_bytes()[3..].iter().all(u8::is_ascii_digit)
            {
                *code
            } else {
                "unknown-rule"
            };
            let subject = if subject.len() <= 128
                && expectation
                    .diagnostic_subjects
                    .iter()
                    .any(|known| known.as_str() == *subject)
            {
                *subject
            } else {
                "unknown-subject"
            };
            format!("{code}:{subject}")
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{facts}; omitted={}",
        codes.len().max(subjects.len()).saturating_sub(16)
    )
}

fn validate_terminal_report(
    expectation: &ExportExpectation,
    report: &serde_json::Value,
    exit_code: Option<i32>,
    output_created: bool,
    exact_diagnostics: bool,
) -> Result<(), String> {
    validate_terminal_report_for_policy(
        expectation,
        report,
        exit_code,
        output_created,
        exact_diagnostics,
        &expectation.loss_policy,
    )
}

fn validate_terminal_report_for_policy(
    expectation: &ExportExpectation,
    report: &serde_json::Value,
    exit_code: Option<i32>,
    output_created: bool,
    exact_diagnostics: bool,
    policy: &str,
) -> Result<(), String> {
    validate_terminal_expectation(expectation)?;
    let (status, category, exit) = match expectation.terminal_status.unwrap_or(TerminalStatus::Blocked) {
        TerminalStatus::Blocked => ("blocked", "policy-blocked", 2),
        TerminalStatus::Failure => ("failure", "input-or-execution", 1),
    };
    if report["schema_version"] != 1
        || report["status"] != status
        || report["exit_category"] != category
        || exit_code != Some(exit)
    {
        return Err("terminal schema, status, exit category or process status differs".to_owned());
    }
    if output_created || !report["output_artifacts"].as_array().is_some_and(Vec::is_empty) {
        return Err("terminal route emitted an artifact or created an output entry".to_owned());
    }
    if report["fidelity"]["invalid"].as_u64() != Some(expectation.invalid_outcomes.unwrap_or(0)) {
        return Err("terminal invalid fidelity differs from its exact declaration".to_owned());
    }
    if expectation.terminal_status == Some(TerminalStatus::Failure)
        && !report["diagnostics"]
            .as_array()
            .is_some_and(|diagnostics| diagnostics.iter().any(|diagnostic| diagnostic["severity"] == "error"))
    {
        return Err("terminal failure lacks its invalid-source error diagnostic".to_owned());
    }
    if exact_diagnostics || expectation.terminal_status == Some(TerminalStatus::Failure) {
        let (codes, subjects) = terminal_expected_diagnostics(expectation, policy)?;
        if report_diagnostic_codes(report)? != codes || report_diagnostic_subjects(report)? != subjects {
            return Err("terminal diagnostic code or subject sequence differs".to_owned());
        }
    }
    Ok(())
}

#[test]
fn terminal_diagnostic_order_metadata_rejects_non_permutations_and_schema_drift() -> Result<(), Box<dyn Error>> {
    let control = terminal_order_control();
    let expectation: ExportExpectation = toml::from_str(control)?;
    assert_eq!(validate_terminal_expectation(&expectation), Ok(()));
    for (policy, order) in [("exact", "[1, 0]"), ("approximate", "[1, 0]"), ("partial", "[0, 1]")] {
        let assignment = format!("{policy} = {order}\n");
        assert!(toml::from_str::<ExportExpectation>(&control.replace(&assignment, "")).is_err());
        for replacement in ["[0, 0]", "[0]", "[0, 1, 2]", "[0, 2]"] {
            let changed: ExportExpectation =
                toml::from_str(&control.replace(&assignment, &format!("{policy} = {replacement}\n")))?;
            assert!(validate_terminal_expectation(&changed).is_err());
        }
    }
    assert!(toml::from_str::<ExportExpectation>(&format!("{control}unknown = [0, 1]\n")).is_err());
    for changed in [
        control.replace("partial = [0, 1]", "partial = [1, 0]"),
        control.replace("loss-policy = 'partial'", "loss-policy = 'exact'"),
        control
            .replace("terminal-status = 'failure'\n", "")
            .replace("invalid-outcomes = 1\n", ""),
        control.replace("'services.synthetic.container'", ""),
    ] {
        let expectation: ExportExpectation = toml::from_str(&changed)?;
        assert!(validate_terminal_expectation(&expectation).is_err());
    }
    assert!(terminal_expected_diagnostics(&expectation, "unknown").is_err());
    let without_orders: ExportExpectation = toml::from_str(
        control
            .split_once("[terminal-diagnostic-orders]")
            .ok_or("control order table")?
            .0,
    )?;
    assert_eq!(
        terminal_expected_diagnostics(&without_orders, "exact")?,
        terminal_expected_diagnostics(&without_orders, "partial")?
    );
    Ok(())
}

fn terminal_order_control() -> &'static str {
    "loss-policy = 'partial'\nblocked = true\nterminal-status = 'failure'\ninvalid-outcomes = 1\ndiagnostic-codes = ['BFD0007', 'BFD0006']\ndiagnostic-subjects = ['services.synthetic.mounts.0', 'services.synthetic.container']\n[terminal-diagnostic-orders]\nexact = [1, 0]\napproximate = [1, 0]\npartial = [0, 1]\n"
}

#[test]
fn terminal_policy_orders_preserve_paired_facts_and_declared_final_order() -> Result<(), Box<dyn Error>> {
    let expectation: ExportExpectation = toml::from_str(terminal_order_control())?;
    let base = serde_json::json!({
        "schema_version": 1, "status": "failure", "exit_category": "input-or-execution",
        "output_artifacts": [], "fidelity": {"invalid": 1},
        "diagnostics": [
            {"code": "BFD0007", "severity": "error",
             "fields": [{"name": "subject", "value": "services.synthetic.mounts.0"}]},
            {"code": "BFD0006", "severity": "warning",
             "fields": [{"name": "subject", "value": "services.synthetic.container"}]},
        ],
    });
    assert_eq!(
        validate_terminal_report(&expectation, &base, Some(1), false, true),
        Ok(())
    );
    for policy in ["exact", "approximate", "partial"] {
        let mut report = base.clone();
        if policy != "partial" {
            report["diagnostics"]
                .as_array_mut()
                .ok_or("control diagnostics")?
                .reverse();
        }
        assert_eq!(
            validate_terminal_report_for_policy(&expectation, &report, Some(1), false, true, policy),
            Ok(())
        );
        let mut wrong_order = report.clone();
        wrong_order["diagnostics"]
            .as_array_mut()
            .ok_or("control diagnostics")?
            .reverse();
        assert!(validate_terminal_report_for_policy(&expectation, &wrong_order, Some(1), false, true, policy).is_err());
        let mut codes_only = report.clone();
        let first_code = codes_only["diagnostics"][0]["code"].clone();
        codes_only["diagnostics"][0]["code"] = codes_only["diagnostics"][1]["code"].clone();
        codes_only["diagnostics"][1]["code"] = first_code;
        assert!(validate_terminal_report_for_policy(&expectation, &codes_only, Some(1), false, true, policy).is_err());
        let first_subject = report["diagnostics"][0]["fields"][0]["value"].clone();
        report["diagnostics"][0]["fields"][0]["value"] = report["diagnostics"][1]["fields"][0]["value"].clone();
        report["diagnostics"][1]["fields"][0]["value"] = first_subject;
        assert!(validate_terminal_report_for_policy(&expectation, &report, Some(1), false, true, policy).is_err());
    }
    let mut reversed = base;
    reversed["diagnostics"]
        .as_array_mut()
        .ok_or("control diagnostics")?
        .reverse();
    assert!(validate_terminal_report(&expectation, &reversed, Some(1), false, true).is_err());
    Ok(())
}

#[test]
fn terminal_failure_schema_is_closed_and_legacy_blocks_keep_their_defaults() -> Result<(), Box<dyn Error>> {
    let legacy: ExportExpectation = toml::from_str("loss-policy = 'partial'\nblocked = true\n")?;
    assert_eq!(validate_terminal_expectation(&legacy), Ok(()));
    assert_eq!(terminal_or_stricter_policies(&legacy)?, ["exact", "approximate"]);
    for status in ["success", "unknown", "Failure"] {
        assert!(
            toml::from_str::<ExportExpectation>(&format!(
                "loss-policy = 'partial'\nblocked = true\nterminal-status = '{status}'\ninvalid-outcomes = 1\n"
            ))
            .is_err()
        );
    }
    let failure = "loss-policy = 'partial'\nblocked = true\nterminal-status = 'failure'\ninvalid-outcomes = 1\n";
    for changed in [
        failure.replace("blocked = true", "blocked = false"),
        failure.replace("invalid-outcomes = 1\n", ""),
        failure.replace("invalid-outcomes = 1", "invalid-outcomes = 0"),
        failure.replace("terminal-status = 'failure'", "terminal-status = 'blocked'"),
        format!("{failure}[[artifacts]]\nartifact = 'unexpected.json'\nexpected = 'unexpected.json'\n"),
    ] {
        let expectation: ExportExpectation = toml::from_str(&changed)?;
        assert!(validate_terminal_expectation(&expectation).is_err());
    }
    for policy in ["exact", "approximate", "partial"] {
        let expectation: ExportExpectation = toml::from_str(&failure.replace("partial", policy))?;
        let mut policies = terminal_or_stricter_policies(&expectation)?;
        policies.push(match policy {
            "exact" => "exact",
            "approximate" => "approximate",
            _ => "partial",
        });
        assert_eq!(
            policies.into_iter().collect::<BTreeSet<_>>(),
            BTreeSet::from(["exact", "approximate", "partial"])
        );
    }
    Ok(())
}

#[test]
fn terminal_failure_report_rejects_counterfactual_status_fidelity_and_output() -> Result<(), Box<dyn Error>> {
    let expectation: ExportExpectation = toml::from_str(
        "loss-policy = 'partial'\nblocked = true\nterminal-status = 'failure'\ninvalid-outcomes = 1\ndiagnostic-codes = ['BFD0007']\ndiagnostic-subjects = ['services.synthetic.mounts.0']\n",
    )?;
    let report = serde_json::json!({
        "schema_version": 1, "status": "failure", "exit_category": "input-or-execution",
        "output_artifacts": [], "fidelity": {"invalid": 1},
        "diagnostics": [{"code": "BFD0007", "severity": "error",
            "fields": [{"name": "subject", "value": "services.synthetic.mounts.0"}]}],
    });
    assert_eq!(
        validate_terminal_report(&expectation, &report, Some(1), false, true),
        Ok(())
    );
    for (field, changed_value) in [
        ("status", serde_json::json!("blocked")),
        ("status", serde_json::json!("success")),
        ("exit_category", serde_json::json!("policy-blocked")),
        (
            "output_artifacts",
            serde_json::json!([{"name": "unexpected.json", "size": 1}]),
        ),
        ("fidelity", serde_json::json!({"invalid": 0})),
        ("fidelity", serde_json::json!({"invalid": "1"})),
        ("diagnostics", serde_json::json!([])),
    ] {
        let mut changed = report.clone();
        changed[field] = changed_value;
        assert!(validate_terminal_report(&expectation, &changed, Some(1), false, true).is_err());
    }
    assert!(validate_terminal_report(&expectation, &report, Some(2), false, true).is_err());
    assert!(validate_terminal_report(&expectation, &report, None, false, true).is_err());
    assert!(validate_terminal_report(&expectation, &report, Some(1), true, true).is_err());
    let mut changed = report.clone();
    changed["diagnostics"][0]["fields"][0]["value"] = serde_json::json!("services.synthetic.mounts.1");
    assert!(validate_terminal_report(&expectation, &changed, Some(1), false, true).is_err());
    assert!(validate_terminal_report(&expectation, &changed, Some(1), false, false).is_err());
    changed = report.clone();
    changed["diagnostics"][0]["severity"] = serde_json::json!("warning");
    assert!(validate_terminal_report(&expectation, &changed, Some(1), false, true).is_err());
    Ok(())
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
        let expected = fs::read_to_string(fixture.join(&artifact.expected))?;
        if artifact.artifact == "docker-dependencies.json" {
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&actual)?,
                serde_json::from_str::<serde_json::Value>(&expected)?,
                "{scenario} -> {output} dependency decision changed"
            );
        } else {
            assert_eq!(
                actual, expected,
                "{scenario} -> {output} artifact {} changed",
                artifact.artifact
            );
        }
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
    let review = fs::read_to_string(output_directory.join("podman-commands.sh"))?;
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

fn assert_docker_artifacts(
    output_directory: &Path,
    protected_values: &[String],
    include_environment_values: bool,
    authorized_values: &[String],
    scenario: &str,
) -> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::PermissionsExt;

    let plan_path = output_directory.join("docker-plan.json");
    let plan_bytes = fs::read(&plan_path)?;
    let plan: serde_json::Value = serde_json::from_slice(&plan_bytes)?;
    assert_eq!(plan["schema_version"], 1, "{scenario} Docker schema changed");
    assert_eq!(plan["context"]["kind"], "target");
    assert_eq!(plan["context"]["build"]["kind"], "upstream");
    assert_eq!(plan["context"]["engine_release"], "29.8.1");
    assert_eq!(plan["context"]["rendering_api_version"], "1.56");
    assert_eq!(plan["context"]["daemon_mode"], "rootful");
    assert_eq!(
        plan["context"]["evidence_sha256"],
        "b0825b797f052257f706b0102c5c720e7fcbbaef0e585869bb6d23d2e756fb94"
    );
    let requests = plan["requests"].as_array().ok_or("Docker requests missing")?;
    assert!(!requests.is_empty(), "{scenario} Docker plan has no requests");
    for request in requests {
        assert_eq!(
            request["method"], "POST",
            "{scenario} Docker plan must remain inert POST intent"
        );
        assert!(request["path"].as_str().is_some_and(|path| path.starts_with("/v1.56/")));
        assert!(request["body"].is_object());
    }
    assert!(plan["prerequisites"].is_array());
    assert_eq!(fs::metadata(output_directory)?.permissions().mode() & 0o777, 0o700);
    assert_eq!(fs::metadata(&plan_path)?.permissions().mode() & 0o777, 0o600);
    let dependency_path = output_directory.join("docker-dependencies.json");
    let mut dependency_document = None;
    if dependency_path.exists() {
        let dependency_bytes = fs::read(&dependency_path)?;
        let dependency: serde_json::Value = serde_json::from_slice(&dependency_bytes)?;
        assert_eq!(dependency["schema_version"], 1);
        assert_eq!(dependency["kind"], "boxferry-docker-dependency-decisions");
        assert_eq!(dependency["native_execution"], false);
        let digest = Sha256::digest(&plan_bytes);
        let mut digest_hex = String::with_capacity(digest.len() * 2);
        for byte in digest {
            write!(&mut digest_hex, "{byte:02x}")?;
        }
        assert_eq!(dependency["docker_plan_sha256"], digest_hex);
        assert!(
            dependency["decisions"]
                .as_array()
                .is_some_and(|decisions| !decisions.is_empty())
        );
        assert_eq!(fs::metadata(&dependency_path)?.permissions().mode() & 0o777, 0o600);
        dependency_document = Some(dependency);
    }
    validate_docker_artifact_privacy(
        &plan,
        dependency_document.as_ref(),
        protected_values,
        include_environment_values,
        authorized_values,
    )
    .map_err(|error| format!("{scenario}: {error}"))?;
    Ok(())
}

fn validate_docker_artifact_privacy(
    plan: &serde_json::Value,
    dependency: Option<&serde_json::Value>,
    protected_values: &[String],
    include_environment_values: bool,
    authorized_values: &[String],
) -> Result<(), &'static str> {
    let mut redacted = plan.clone();
    let requests = redacted
        .get_mut("requests")
        .and_then(serde_json::Value::as_array_mut)
        .ok_or("Docker privacy review requires native requests")?;
    for request in requests {
        if let Some(environment) = request.get_mut("body").and_then(|body| body.get_mut("Env")) {
            let assignments = environment
                .as_array_mut()
                .ok_or("Docker Env must be an assignment array")?;
            for assignment in assignments {
                let text = assignment.as_str().ok_or("Docker Env assignment must be text")?;
                let (name, value) = text
                    .split_once('=')
                    .ok_or("Docker Env assignment requires a name and value")?;
                if include_environment_values && authorized_values.iter().any(|authorized| authorized == value) {
                    // Retain the key and remove only an exactly consented assignment value.
                    *assignment = serde_json::Value::String(format!("{name}="));
                }
            }
        }
    }
    for protected in protected_values {
        if json_contains(&redacted, protected) || dependency.is_some_and(|document| json_contains(document, protected))
        {
            return Err("Docker artifact disclosed an unauthorized protected value");
        }
    }
    Ok(())
}

fn docker_privacy_control_plan(environment: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "context": {"kind": "target"},
        "requests": [{
            "method": "POST", "path": "/v1.56/containers/create?name=app",
            "body": {"Env": environment},
        }],
        "prerequisites": [],
    })
}

#[test]
fn docker_environment_artifact_privacy_requires_exact_value_consent() {
    let marker = "protected-marker";
    let protected = vec![marker.to_owned()];
    let authorized = vec![marker.to_owned()];
    let plan = docker_privacy_control_plan(&["TOKEN=protected-marker"]);
    assert!(validate_docker_artifact_privacy(&plan, None, &protected, true, &authorized).is_ok());
    assert_eq!(plan["requests"][0]["body"]["Env"][0], "TOKEN=protected-marker");
    for (include, declared) in [
        (false, authorized.clone()),
        (true, Vec::new()),
        (true, vec!["other-value".to_owned()]),
    ] {
        assert!(validate_docker_artifact_privacy(&plan, None, &protected, include, &declared).is_err());
    }
    for environment in [
        vec!["TOKEN=prefix-protected-marker-suffix"],
        vec!["TOKEN=protected-marker", "SECOND=prefix-protected-marker-suffix"],
        vec!["protected-marker=protected-marker"],
    ] {
        let plan = docker_privacy_control_plan(&environment);
        assert!(validate_docker_artifact_privacy(&plan, None, &protected, true, &authorized).is_err());
    }
    let plan = docker_privacy_control_plan(&["TOKEN=prefix-protected-marker-suffix", "SECOND=value=protected-marker"]);
    let full_values = vec![
        "prefix-protected-marker-suffix".to_owned(),
        "value=protected-marker".to_owned(),
    ];
    assert!(validate_docker_artifact_privacy(&plan, None, &protected, true, &full_values).is_ok());
}

#[test]
fn docker_environment_value_consent_never_covers_keys_other_fields_or_sidecars() -> Result<(), Box<dyn Error>> {
    let marker = "protected-marker";
    let protected = vec![marker.to_owned()];
    let authorized = vec![marker.to_owned()];
    let plan = docker_privacy_control_plan(&["TOKEN=protected-marker"]);
    for (pointer, value) in [
        ("/context", serde_json::json!({"private": marker})),
        ("/context", serde_json::json!({"protected-marker": "public"})),
        (
            "/requests/0/path",
            serde_json::json!("/v1.56/containers/create?name=protected-marker"),
        ),
        (
            "/requests/0/body",
            serde_json::json!({"Env": ["TOKEN=protected-marker"], "Cmd": [marker]}),
        ),
        (
            "/requests/0/body",
            serde_json::json!({"Env": ["TOKEN=protected-marker"], "Labels": {"private": marker}}),
        ),
        (
            "/requests/0/body",
            serde_json::json!({"Env": ["TOKEN=protected-marker"], "Labels": {"protected-marker": "public"}}),
        ),
    ] {
        let mut relocated = plan.clone();
        *relocated
            .pointer_mut(pointer)
            .ok_or("privacy control pointer missing")? = value;
        assert!(
            validate_docker_artifact_privacy(&relocated, None, &protected, true, &authorized).is_err(),
            "environment value consent covered {pointer}",
        );
    }
    for dependency in [
        serde_json::json!({"decisions": [{"body": {"Env": ["TOKEN=protected-marker"]}}]}),
        serde_json::json!({"protected-marker": "public"}),
    ] {
        assert!(validate_docker_artifact_privacy(&plan, Some(&dependency), &protected, true, &authorized).is_err());
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
        "{scenario} command script operation count changed"
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
            "{scenario} command script lacks tmpfs create evidence"
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

fn report_diagnostic_subjects(report: &serde_json::Value) -> Result<Vec<&str>, String> {
    report["diagnostics"]
        .as_array()
        .ok_or_else(|| "report diagnostics array".to_owned())?
        .iter()
        .map(|diagnostic| {
            diagnostic["fields"]
                .as_array()
                .ok_or_else(|| "diagnostic fields array".to_owned())?
                .iter()
                .find(|field| field["name"] == "subject")
                .and_then(|field| field["value"].as_str())
                .map_or(Ok(""), Ok)
        })
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
