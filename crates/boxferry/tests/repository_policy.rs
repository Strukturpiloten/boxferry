//! Executable repository and fixture-contract checks.

mod support;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

const FIXTURE_SUITES: &[&str] = &[
    "model",
    "adapter-contract",
    "conversion",
    "roundtrip",
    "differential",
    "real-world",
];

const FORMAT_ADAPTER_PACKAGES: &[&str] = &["boxferry-compose", "boxferry-podman", "boxferry-quadlet"];

const PUBLISHED_PACKAGES: &[&str] = &[
    "boxferry-model",
    "boxferry-engine",
    "boxferry-compose",
    "boxferry-podman",
    "boxferry-quadlet",
    "boxferry",
];
const CRATES_IO_AUTH_ACTION: &str = "rust-lang/crates-io-auth-action@c6f97d42243bad5fab37ca0427f495c86d5b1a18 # v1.0.5";
const CRATES_IO_BOOTSTRAP_SECRET: &str = "secrets.CRATES_IO_BOOTSTRAP_TOKEN";

#[test]
fn github_actions_are_immutable_and_versioned() -> Result<(), String> {
    support::validate_action_pins(&repository_root())
}

#[test]
fn live_scenario_podman_adapter_preserves_the_action_argument() -> Result<(), String> {
    let validators = fs::read_to_string(repository_root().join("scripts/lib/scenario-validators.sh"))
        .map_err(|error| format!("failed to read live scenario validators: {error}"))?;
    let required = concat!(
        "scenario_podman_socket() {\n",
        "  local socket=${1:?scenario validator must supply socket}\n",
        "  local subcommand=${2:?scenario validator must supply Podman command}\n",
        "  shift 2\n",
        "  podman_socket \"${socket}\" \"validate runtime scenario via Podman ${subcommand}\" \\\n",
        "    \"${subcommand}\" \"$@\"\n",
        "}",
    );
    if !validators.contains(required) {
        return Err(
            "live scenario validators must adapt command-oriented calls to podman_socket's explicit action contract"
                .to_owned(),
        );
    }
    let action_contract_calls = validators.matches("podman_socket \"${socket}\"").count();
    let adapted_calls = validators.matches("scenario_podman_socket \"${socket}\"").count();
    if action_contract_calls != adapted_calls + 1 {
        return Err(
            "live scenario validators must call podman_socket only through their action-preserving adapter".to_owned(),
        );
    }
    if adapted_calls != 23 {
        return Err("every live runtime scenario query must use the action-preserving Podman adapter".to_owned());
    }
    for forbidden in [
        " podman_socket \"${socket}\" info",
        "$(podman_socket \"${socket}\" info",
        " podman_socket \"${socket}\" inspect",
        "$(podman_socket \"${socket}\" inspect",
        " podman_socket \"${socket}\" network",
        "$(podman_socket \"${socket}\" network",
        " podman_socket \"${socket}\" secret",
        "$(podman_socket \"${socket}\" secret",
    ] {
        if validators.contains(forbidden) {
            return Err(format!(
                "live scenario validators must not consume `{forbidden}` as a human action"
            ));
        }
    }
    Ok(())
}

#[test]
fn migration_readiness_restores_privileged_evidence_before_consumers() -> Result<(), String> {
    let workflow = fs::read_to_string(repository_root().join(".github/workflows/migration-readiness.yml"))
        .map_err(|error| format!("failed to read migration-readiness workflow: {error}"))?;
    let trusted = workflow
        .find("      - name: Run trusted tier through shared runner")
        .ok_or("migration-readiness workflow must run its trusted tier")?;
    let restore_contract = concat!(
        "      - name: Restore bounded evidence ownership\n",
        "        if: ${{ always() && env.TIER != 'offline' }}\n",
        "        run: |\n",
        "          if [[ -e target/migration-readiness ]]; then\n",
        "            sudo chown --recursive \"$(id -u):$(id -g)\" target/migration-readiness\n",
        "          fi\n",
    );
    let restore = workflow
        .find(restore_contract)
        .ok_or("migration-readiness workflow must restore only its bounded evidence tree to the runner")?;
    let next_step = workflow[trusted + 1..]
        .find("\n      - name:")
        .map(|offset| trusted + 1 + offset + 1)
        .ok_or("trusted migration-readiness execution must have a following step")?;
    if restore != next_step {
        return Err("evidence ownership restoration must immediately follow privileged execution".to_owned());
    }
    for consumer in [
        "hashFiles('target/migration-readiness/evidence-v1.json')",
        "scripts/migration-readiness.py validate-evidence",
        "actions/upload-artifact@",
    ] {
        let position = workflow
            .find(consumer)
            .ok_or_else(|| format!("migration-readiness workflow must retain `{consumer}`"))?;
        if position <= restore {
            return Err(format!(
                "migration-readiness evidence consumer `{consumer}` must run after ownership restoration"
            ));
        }
    }
    Ok(())
}

#[test]
fn ci_runs_once_per_pull_request_update_and_on_main_pushes() -> Result<(), String> {
    let workflow_path = repository_root().join(".github/workflows/ci.yml");
    let workflow = fs::read_to_string(&workflow_path)
        .map_err(|error| format!("failed to read {}: {error}", workflow_path.display()))?;
    let expected = "on:\n  push:\n    branches:\n      - main\n  pull_request:\n  workflow_dispatch:\n";
    if !workflow.contains(expected) {
        return Err(
            "CI must run for main pushes, pull requests, and manual dispatch without duplicate feature-branch push runs"
                .to_owned(),
        );
    }

    Ok(())
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "keeps the one live-conformance repository contract reviewable in one place"
)]
fn live_podman_conformance_uses_one_checked_in_runner_and_reviewed_matrix() -> Result<(), String> {
    let root = repository_root();
    let runner = fs::read_to_string(root.join("scripts/podman-live-conformance.sh"))
        .map_err(|error| format!("failed to read live Podman runner: {error}"))?;
    let matrix = fs::read_to_string(root.join("fixtures/conformance/podman-live/matrix.tsv"))
        .map_err(|error| format!("failed to read live Podman matrix: {error}"))?;
    let scenarios = fs::read_to_string(root.join("fixtures/conformance/podman-live/scenarios.tsv"))
        .map_err(|error| format!("failed to read live Podman scenarios: {error}"))?;
    let hosted = fs::read_to_string(root.join(".github/workflows/podman-live-conformance.yml"))
        .map_err(|error| format!("failed to read hosted Podman workflow: {error}"))?;
    let limitations = fs::read_to_string(root.join("fixtures/conformance/podman-live/limitations.tsv"))
        .map_err(|error| format!("failed to read live Podman limitations: {error}"))?;
    validate_live_matrix(&matrix, &limitations)?;
    validate_live_scenarios(&scenarios)?;
    let mut runner_contract = runner.clone();
    for module in [
        "scenario-contract.sh",
        "scenario-validators.sh",
        "nextcloud-application.sh",
        "forgejo-application.sh",
        "paperless-application.sh",
        "immich-application.sh",
        "observability-application.sh",
        "supabase-application.sh",
    ] {
        let source = format!("source \"${{script_directory}}/lib/{module}\"");
        if !runner.contains(&source) {
            return Err(format!("live runner must source its shared module: {source}"));
        }
        runner_contract.push_str(
            &fs::read_to_string(root.join("scripts/lib").join(module))
                .map_err(|error| format!("failed to read live scenario module: {error}"))?,
        );
    }
    for fixture in [
        "compose.yaml",
        "frontend.conf",
        "images.tsv",
        "providers.tsv",
        "proxy.conf",
        "second-index.html",
        "published-probe.php",
        "webdav-probe.php",
        "README.md",
    ] {
        runner_contract.push_str(
            &fs::read_to_string(root.join("fixtures/conformance/nextcloud-application").join(fixture))
                .map_err(|error| format!("failed to read Nextcloud application fixture: {error}"))?,
        );
    }
    for fixture in [
        "compose.yaml",
        "peer.compose.yaml",
        "git-probe.sh",
        "images.tsv",
        "providers.tsv",
        "repository-proof.txt",
        "README.md",
    ] {
        runner_contract.push_str(
            &fs::read_to_string(root.join("fixtures/conformance/forgejo-application").join(fixture))
                .map_err(|error| format!("failed to read Forgejo application fixture: {error}"))?,
        );
    }
    for fixture in [
        "compose.yaml",
        "document-probe.py",
        "images.tsv",
        "providers.tsv",
        "README.md",
    ] {
        runner_contract.push_str(
            &fs::read_to_string(
                root.join("fixtures/conformance/paperless-ngx-application")
                    .join(fixture),
            )
            .map_err(|error| format!("failed to read Paperless application fixture: {error}"))?,
        );
    }
    append_immich_fixture_contract(&root, &mut runner_contract)?;
    append_supabase_fixture_contract(&root, &mut runner_contract)?;
    for fixture in [
        "compose.yaml",
        "config.alloy",
        "dashboard.json",
        "grafana-dashboards.yaml",
        "grafana-datasources.yaml",
        "images.tsv",
        "loki.yaml",
        "producer.sh",
        "prometheus.yml",
        "providers.tsv",
        "README.md",
    ] {
        runner_contract.push_str(
            &fs::read_to_string(
                root.join("fixtures/conformance/observability-application")
                    .join(fixture),
            )
            .map_err(|error| format!("failed to read observability fixture: {error}"))?,
        );
    }
    let capture_tool = root.join("fixtures/conformance/podman-live/capture_proxy.py");
    runner_contract.push_str(
        &fs::read_to_string(&capture_tool)
            .map_err(|error| format!("failed to read Paperless capture proxy: {error}"))?,
    );
    let capture_self_test = Command::new("python3")
        .arg(&capture_tool)
        .arg("--self-test")
        .status()
        .map_err(|error| format!("failed to run Paperless capture self-test: {error}"))?;
    if !capture_self_test.success() {
        return Err("Paperless capture proxy self-test failed".to_owned());
    }
    let revalidation_tool = root.join("scripts/lib/podman-revalidation.py");
    let revalidation_self_test = Command::new("python3")
        .arg(&revalidation_tool)
        .arg("--self-test")
        .output()
        .map_err(|error| format!("failed to run Podman revalidation self-test: {error}"))?;
    if !revalidation_self_test.status.success()
        || String::from_utf8_lossy(&revalidation_self_test.stdout).trim()
            != "podman-revalidation hardened self-test: PASS"
    {
        return Err(format!(
            "Podman revalidation self-test failed: {}",
            String::from_utf8_lossy(&revalidation_self_test.stderr).trim()
        ));
    }
    let candidates = fs::read_to_string(root.join("fixtures/conformance/podman-live/candidates.toml"))
        .map_err(|error| format!("failed to read Podman revalidation candidates: {error}"))?;
    let candidate_ids = validate_podman_revalidation_candidates(&candidates, &matrix, &limitations)?;
    let revalidation_workflow = fs::read_to_string(root.join(".github/workflows/podman-limitation-revalidation.yml"))
        .map_err(|error| format!("failed to read Podman revalidation workflow: {error}"))?;
    validate_live_runner(&runner_contract, &matrix)?;
    validate_limitation_revalidation_runner(&runner)?;
    validate_podman_revalidation_workflow(&revalidation_workflow, &candidate_ids)?;
    validate_live_workflow(&hosted)
}

fn append_immich_fixture_contract(root: &Path, runner_contract: &mut String) -> Result<(), String> {
    for fixture in [
        "compose.yaml",
        "media-probe.py",
        "images.tsv",
        "providers.tsv",
        "README.md",
    ] {
        runner_contract.push_str(
            &fs::read_to_string(root.join("fixtures/conformance/immich-application").join(fixture))
                .map_err(|error| format!("failed read Immich application fixture: {error}"))?,
        );
    }
    Ok(())
}

fn append_supabase_fixture_contract(root: &Path, runner_contract: &mut String) -> Result<(), String> {
    for fixture in [
        "README.md",
        "application-probe.mjs",
        "application.tsv",
        "compose.yaml",
        "db-init.sql",
        "functions/main/index.ts",
        "graph.tsv",
        "images.tsv",
        "kong.yml",
        "peer.compose.yaml",
        "postgres-components.tsv",
        "providers.tsv",
        "routes.tsv",
        "success-contract.jq",
    ] {
        runner_contract.push_str(
            &fs::read_to_string(root.join("fixtures/conformance/supabase-application").join(fixture))
                .map_err(|error| format!("failed to read Supabase application fixture: {error}"))?,
        );
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum SupabaseContractMode {
    Validate,
    Diagnostics,
    Fidelity,
}

impl SupabaseContractMode {
    const fn jq_value(self) -> &'static str {
        match self {
            Self::Validate => "false",
            Self::Diagnostics => "true",
            Self::Fidelity => "\"fidelity\"",
        }
    }
}

fn run_supabase_report_contract(
    root: &Path,
    input: &str,
    output: &str,
    selection: &str,
    mode: SupabaseContractMode,
    report: Option<&serde_json::Value>,
) -> Result<Output, String> {
    let mut command = Command::new("jq");
    command.arg("--exit-status");
    if report.is_none() {
        command.arg("--null-input");
    }
    command
        .args(["--arg", "input", input])
        .args(["--arg", "output", output])
        .args(["--arg", "selection", selection])
        .args(["--arg", "resource_prefix", "contract-supabase-"])
        .args(["--argjson", "emit_expected", mode.jq_value()])
        .arg("--from-file")
        .arg(root.join("fixtures/conformance/supabase-application/success-contract.jq"));
    if report.is_some() {
        command.stdin(Stdio::piped());
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("failed to start Supabase jq contract: {error}"))?;
    if let Some(report) = report {
        let mut stdin = child.stdin.take().ok_or("Supabase jq contract stdin was unavailable")?;
        serde_json::to_writer(&mut stdin, report)
            .map_err(|error| format!("failed to serialize Supabase contract report: {error}"))?;
        stdin
            .write_all(b"\n")
            .map_err(|error| format!("failed to finish Supabase contract report: {error}"))?;
    }
    child
        .wait_with_output()
        .map_err(|error| format!("failed to wait for Supabase jq contract: {error}"))
}

fn supabase_contract_diagnostic(tuple: &serde_json::Value) -> serde_json::Value {
    let mut fields = vec![
        serde_json::json!({"name": "subject", "value": tuple["subject"]}),
        serde_json::json!({"name": "decision", "value": tuple["decision"]}),
    ];
    if let Some(reason) = tuple.get("reason") {
        fields.push(serde_json::json!({"name": "reason", "value": reason}));
    }
    serde_json::json!({
        "code": tuple["code"],
        "severity": tuple["severity"],
        "name": "exact Supabase contract example",
        "fields": fields,
    })
}

fn supabase_contract_accepts(
    root: &Path,
    input: &str,
    output: &str,
    selection: &str,
    report: &serde_json::Value,
) -> Result<bool, String> {
    let result = run_supabase_report_contract(
        root,
        input,
        output,
        selection,
        SupabaseContractMode::Validate,
        Some(report),
    )?;
    if !result.status.success() && result.status.code() != Some(1) {
        return Err(format!(
            "Supabase jq contract failed to execute: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    Ok(result.status.success())
}

fn generated_supabase_contract(
    root: &Path,
    input: &str,
    output: &str,
    selection: &str,
    mode: SupabaseContractMode,
) -> Result<serde_json::Value, String> {
    let generated = run_supabase_report_contract(root, input, output, selection, mode, None)?;
    if !generated.status.success() {
        return Err(format!(
            "failed to generate {selection} {input}-to-{output} Supabase contract: {}",
            String::from_utf8_lossy(&generated.stderr).trim()
        ));
    }
    serde_json::from_slice(&generated.stdout)
        .map_err(|error| format!("invalid generated {selection} {input}-to-{output} Supabase contract: {error}"))
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "keeps the complete Supabase report matrix and its counterexamples auditable as one contract"
)]
fn supabase_report_contract_rejects_subject_and_fidelity_counterexamples() -> Result<(), String> {
    let root = repository_root();
    let compose_expected = run_supabase_report_contract(
        &root,
        "compose",
        "podman",
        "storage",
        SupabaseContractMode::Diagnostics,
        None,
    )?;
    if !compose_expected.status.success() {
        return Err(format!(
            "failed to generate Compose rejection contract: {}",
            String::from_utf8_lossy(&compose_expected.stderr).trim()
        ));
    }
    let expected: serde_json::Value = serde_json::from_slice(&compose_expected.stdout)
        .map_err(|error| format!("invalid generated Compose rejection contract: {error}"))?;
    let expected_diagnostics = expected
        .as_array()
        .ok_or("generated Compose rejection contract must be an array")?;
    let mut subjects = expected_diagnostics
        .iter()
        .filter(|diagnostic| diagnostic["code"] == "BFP0008")
        .map(|diagnostic| {
            diagnostic["subject"]
                .as_str()
                .map(str::to_owned)
                .ok_or("generated BFP0008 subject must be a string")
        })
        .collect::<Result<Vec<_>, _>>()?;
    subjects.sort();
    assert_eq!(
        subjects,
        [
            "services.contract-supabase-db.image",
            "services.contract-supabase-imgproxy.image",
            "services.contract-supabase-rest.image",
            "services.contract-supabase-storage.image",
        ]
    );
    assert_eq!(
        subjects.iter().collect::<BTreeSet<_>>().len(),
        subjects.len(),
        "generated rejection subjects must be unique"
    );

    let quadlet_expected = run_supabase_report_contract(
        &root,
        "quadlet",
        "podman",
        "storage",
        SupabaseContractMode::Diagnostics,
        None,
    )?;
    if !quadlet_expected.status.success() {
        return Err(format!(
            "failed to generate Quadlet rejection contract: {}",
            String::from_utf8_lossy(&quadlet_expected.stderr).trim()
        ));
    }
    let quadlet_expected: serde_json::Value = serde_json::from_slice(&quadlet_expected.stdout)
        .map_err(|error| format!("invalid generated Quadlet rejection contract: {error}"))?;
    let quadlet_expected = quadlet_expected
        .as_array()
        .ok_or("generated Quadlet rejection contract must be an array")?;
    let mut quadlet_subjects = quadlet_expected
        .iter()
        .filter(|diagnostic| diagnostic["code"] == "BFP0008")
        .filter_map(|diagnostic| diagnostic["subject"].as_str())
        .collect::<Vec<_>>();
    quadlet_subjects.sort_unstable();
    assert_eq!(quadlet_subjects, subjects);

    for (selection, expected_count) in [("exact", 10), ("label", 11), ("all", 12)] {
        let generated = run_supabase_report_contract(
            &root,
            "compose",
            "podman",
            selection,
            SupabaseContractMode::Diagnostics,
            None,
        )?;
        if !generated.status.success() {
            return Err(format!(
                "failed to generate {selection} rejection contract: {}",
                String::from_utf8_lossy(&generated.stderr).trim()
            ));
        }
        let generated: serde_json::Value = serde_json::from_slice(&generated.stdout)
            .map_err(|error| format!("invalid generated {selection} rejection contract: {error}"))?;
        let generated = generated
            .as_array()
            .ok_or("generated rejection contract must be an array")?;
        let unique_subjects = generated
            .iter()
            .filter(|diagnostic| diagnostic["code"] == "BFP0008")
            .filter_map(|diagnostic| diagnostic["subject"].as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            generated
                .iter()
                .filter(|diagnostic| diagnostic["code"] == "BFP0008")
                .count(),
            expected_count,
            "{selection} image count"
        );
        assert_eq!(unique_subjects.len(), expected_count, "{selection} subjects");
        if selection == "exact" {
            assert!(unique_subjects.contains("services.contract-supabase-realtime.image"));
            assert!(!unique_subjects.contains("services.contract-supabase-supavisor.image"));
        }
    }

    for (input, selection, unsupported, invalid) in [
        ("compose", "exact", 28, 10),
        ("compose", "storage", 12, 4),
        ("compose", "label", 30, 11),
        ("compose", "all", 30, 12),
        ("quadlet", "exact", 52, 10),
        ("quadlet", "storage", 20, 4),
        ("quadlet", "label", 55, 11),
        ("quadlet", "all", 54, 12),
    ] {
        let generated =
            generated_supabase_contract(&root, input, "podman", selection, SupabaseContractMode::Diagnostics)?;
        let generated = generated
            .as_array()
            .ok_or("generated rejection contract must be an array")?;
        assert_eq!(
            generated
                .iter()
                .filter(|diagnostic| diagnostic["code"] == "BFP0007")
                .count(),
            unsupported,
            "{input} {selection} unsupported diagnostics"
        );
        assert_eq!(
            generated
                .iter()
                .filter(|diagnostic| diagnostic["code"] == "BFP0008")
                .count(),
            invalid,
            "{input} {selection} invalid diagnostics"
        );
        assert_eq!(
            generated_supabase_contract(&root, input, "podman", selection, SupabaseContractMode::Fidelity,)?,
            serde_json::json!({
                "approximate": 0,
                "unsupported": unsupported,
                "invalid": invalid,
                "other": 0,
            }),
            "{input} {selection} rejection fidelity"
        );
    }

    let diagnostics = expected_diagnostics
        .iter()
        .map(supabase_contract_diagnostic)
        .collect::<Vec<_>>();
    let mut rejection_report = serde_json::json!({
        "schema_version": 1,
        "status": "failure",
        "exit_category": "input-or-execution",
        "primary_diagnostic_code": "BFP0008",
        "output_artifacts": [],
        "fidelity": {
            "exact": 7,
            "approximate": 0,
            "unsupported": 12,
            "invalid": 4,
            "other": 0,
        },
        "diagnostics": diagnostics,
    });
    assert!(supabase_contract_accepts(
        &root,
        "compose",
        "podman",
        "storage",
        &rejection_report
    )?);
    let quadlet_diagnostics = quadlet_expected
        .iter()
        .map(supabase_contract_diagnostic)
        .collect::<Vec<_>>();
    let quadlet_rejection_report = serde_json::json!({
        "schema_version": 1,
        "status": "failure",
        "exit_category": "input-or-execution",
        "primary_diagnostic_code": "BFP0008",
        "output_artifacts": [],
        "fidelity": {
            "exact": 7,
            "approximate": 0,
            "unsupported": 20,
            "invalid": 4,
            "other": 0,
        },
        "diagnostics": quadlet_diagnostics,
    });
    assert!(supabase_contract_accepts(
        &root,
        "quadlet",
        "podman",
        "storage",
        &quadlet_rejection_report
    )?);

    let mut duplicate = rejection_report.clone();
    let first = duplicate["diagnostics"][0].clone();
    duplicate["diagnostics"][1] = first;
    assert!(!supabase_contract_accepts(
        &root, "compose", "podman", "storage", &duplicate
    )?);

    let mut unseen = rejection_report.clone();
    unseen["diagnostics"][0]["fields"][0]["value"] = serde_json::json!("services.contract-supabase-unseen.image");
    assert!(!supabase_contract_accepts(
        &root, "compose", "podman", "storage", &unseen
    )?);

    let mut missing_target_loss = rejection_report.clone();
    let target_loss_index = missing_target_loss["diagnostics"]
        .as_array()
        .and_then(|diagnostics| {
            diagnostics
                .iter()
                .position(|diagnostic| diagnostic["code"] == "BFP0007")
        })
        .ok_or("synthetic rejection report must contain BFP0007")?;
    missing_target_loss["diagnostics"]
        .as_array_mut()
        .ok_or("synthetic rejection diagnostics must be an array")?
        .remove(target_loss_index);
    assert!(!supabase_contract_accepts(
        &root,
        "compose",
        "podman",
        "storage",
        &missing_target_loss
    )?);

    let mut wrong_target_fidelity = rejection_report.clone();
    wrong_target_fidelity["fidelity"]["unsupported"] = serde_json::json!(15);
    assert!(!supabase_contract_accepts(
        &root,
        "compose",
        "podman",
        "storage",
        &wrong_target_fidelity
    )?);

    let mut emitted_artifact = rejection_report.clone();
    emitted_artifact["output_artifacts"] = serde_json::json!(["podman.json"]);
    assert!(!supabase_contract_accepts(
        &root,
        "compose",
        "podman",
        "storage",
        &emitted_artifact
    )?);

    rejection_report["fidelity"]["invalid"] = serde_json::json!(3);
    assert!(!supabase_contract_accepts(
        &root,
        "compose",
        "podman",
        "storage",
        &rejection_report
    )?);

    let reviewed_fidelity = [
        ("exact", "podman", "compose", 46, 105),
        ("exact", "podman", "quadlet", 36, 81),
        ("exact", "podman", "podman", 36, 118),
        ("exact", "quadlet", "compose", 10, 24),
        ("exact", "compose", "compose", 10, 0),
        ("exact", "compose", "quadlet", 0, 0),
        ("exact", "quadlet", "quadlet", 0, 0),
        ("storage", "podman", "compose", 20, 43),
        ("storage", "podman", "quadlet", 16, 35),
        ("storage", "podman", "podman", 16, 51),
        ("storage", "quadlet", "compose", 4, 8),
        ("storage", "compose", "compose", 4, 0),
        ("storage", "compose", "quadlet", 0, 0),
        ("storage", "quadlet", "quadlet", 0, 0),
        ("label", "podman", "compose", 49, 111),
        ("label", "podman", "quadlet", 38, 86),
        ("label", "podman", "podman", 38, 125),
        ("label", "quadlet", "compose", 11, 25),
        ("label", "compose", "compose", 11, 0),
        ("label", "compose", "quadlet", 0, 0),
        ("label", "quadlet", "quadlet", 0, 0),
        ("all", "podman", "compose", 51, 116),
        ("all", "podman", "quadlet", 39, 90),
        ("all", "podman", "podman", 39, 129),
        ("all", "quadlet", "compose", 12, 25),
        ("all", "compose", "compose", 12, 0),
        ("all", "compose", "quadlet", 0, 0),
        ("all", "quadlet", "quadlet", 0, 0),
    ];
    for (selection, input, output, approximate, unsupported) in reviewed_fidelity {
        let generated = generated_supabase_contract(&root, input, output, selection, SupabaseContractMode::Fidelity)?;
        assert_eq!(
            generated,
            serde_json::json!({
                "approximate": approximate,
                "unsupported": unsupported,
                "invalid": 0,
                "other": 0,
            }),
            "{selection} {input}-to-{output} fidelity"
        );
    }

    let success_expected = run_supabase_report_contract(
        &root,
        "podman",
        "podman",
        "label",
        SupabaseContractMode::Diagnostics,
        None,
    )?;
    if !success_expected.status.success() {
        return Err(format!(
            "failed to generate successful route contract: {}",
            String::from_utf8_lossy(&success_expected.stderr).trim()
        ));
    }
    let success_expected: serde_json::Value = serde_json::from_slice(&success_expected.stdout)
        .map_err(|error| format!("invalid generated successful route contract: {error}"))?;
    let success_expected = success_expected
        .as_array()
        .ok_or("generated successful route contract must be an array")?;
    let authored_podman_losses =
        fs::read_to_string(root.join("fixtures/scenarios/supabase-application/expected.podman-podman.losses.tsv"))
            .map_err(|error| format!("failed to read authored Podman losses: {error}"))?;
    for (authored_subject, generated_subject) in [
        (
            "services.kong.environment.KONG_DATABASE",
            "services.contract-supabase-kong.environment.KONG_DATABASE",
        ),
        (
            "services.kong.environment.KONG_DECLARATIVE_CONFIG",
            "services.contract-supabase-kong.environment.KONG_DECLARATIVE_CONFIG",
        ),
    ] {
        assert!(
            success_expected
                .iter()
                .any(|diagnostic| diagnostic["code"] == "BFP0007" && diagnostic["subject"] == generated_subject)
        );
        assert!(authored_podman_losses.lines().any(|row| {
            let fields = row.split('\t').collect::<Vec<_>>();
            fields.len() == 5 && fields[0] == "BFP0007" && fields[1] == authored_subject
        }));
    }
    let expected_fidelity =
        run_supabase_report_contract(&root, "podman", "podman", "label", SupabaseContractMode::Fidelity, None)?;
    if !expected_fidelity.status.success() {
        return Err(format!(
            "failed to generate successful route fidelity: {}",
            String::from_utf8_lossy(&expected_fidelity.stderr).trim()
        ));
    }
    let mut expected_fidelity: serde_json::Value = serde_json::from_slice(&expected_fidelity.stdout)
        .map_err(|error| format!("invalid generated successful route fidelity: {error}"))?;
    assert_eq!(
        expected_fidelity,
        serde_json::json!({
            "approximate": 38,
            "unsupported": 125,
            "invalid": 0,
            "other": 0,
        })
    );
    expected_fidelity["exact"] = serde_json::json!(11);
    let success_diagnostics = success_expected
        .iter()
        .map(supabase_contract_diagnostic)
        .collect::<Vec<_>>();
    let success_report = serde_json::json!({
        "schema_version": 1,
        "status": "success",
        "fidelity": expected_fidelity,
        "diagnostics": success_diagnostics,
    });
    assert!(supabase_contract_accepts(
        &root,
        "podman",
        "podman",
        "label",
        &success_report
    )?);

    let mut volume_metadata_as_approximate = success_report.clone();
    volume_metadata_as_approximate["fidelity"]["approximate"] = serde_json::json!(50);
    volume_metadata_as_approximate["fidelity"]["unsupported"] = serde_json::json!(113);
    assert!(!supabase_contract_accepts(
        &root,
        "podman",
        "podman",
        "label",
        &volume_metadata_as_approximate
    )?);

    let mut missing_kong_network_outcome = success_report.clone();
    missing_kong_network_outcome["fidelity"]["approximate"] = serde_json::json!(37);
    assert!(!supabase_contract_accepts(
        &root,
        "podman",
        "podman",
        "label",
        &missing_kong_network_outcome
    )?);

    let mut arbitrary_exact_count = success_report.clone();
    arbitrary_exact_count["fidelity"]["exact"] = serde_json::json!(999);
    assert!(supabase_contract_accepts(
        &root,
        "podman",
        "podman",
        "label",
        &arbitrary_exact_count
    )?);
    let mut invalid_exact_count = success_report.clone();
    invalid_exact_count["fidelity"]["exact"] = serde_json::json!(1.5);
    assert!(!supabase_contract_accepts(
        &root,
        "podman",
        "podman",
        "label",
        &invalid_exact_count
    )?);

    let mut missing_category = success_report;
    missing_category["fidelity"]
        .as_object_mut()
        .ok_or("synthetic success fidelity must be an object")?
        .remove("other");
    assert!(!supabase_contract_accepts(
        &root,
        "podman",
        "podman",
        "label",
        &missing_category
    )?);
    Ok(())
}

fn validate_live_matrix(matrix: &str, limitations: &str) -> Result<(), String> {
    let rows = matrix
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .collect::<Vec<_>>();
    if rows.len() != 48 {
        return Err(format!(
            "live Podman matrix must contain 48 cells, found {}",
            rows.len()
        ));
    }
    let mut matrix_ids = BTreeSet::new();
    for row in &rows {
        if row.len() != 7 {
            return Err(format!("live Podman matrix row must have seven fields: {row:?}"));
        }
        if !matrix_ids.insert(row[0]) {
            return Err(format!("live Podman matrix cell ID is duplicated: {}", row[0]));
        }
        let image = row[1];
        let Some((_, digest)) = image.rsplit_once("@sha256:") else {
            return Err(format!("live Podman image is not digest pinned: {image}"));
        };
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(format!("live Podman image has an invalid SHA-256 digest: {image}"));
        }
        if row[6] != "amd64" {
            return Err(format!("live Podman matrix architecture is not reviewed: {}", row[6]));
        }
        if !matches!(row[4], "rootful" | "rootless") {
            return Err(format!("live Podman matrix root mode is not reviewed: {}", row[4]));
        }
        if row[5] != "container" {
            return Err(format!("live Podman matrix lane is not reviewed: {}", row[5]));
        }
    }
    let container_rows = rows.iter().filter(|row| row[5] == "container").count();
    if container_rows != 48 {
        return Err(format!(
            "live Podman matrix must contain 48 container cells, found {container_rows}"
        ));
    }
    let limitation_rows = limitations
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .collect::<Vec<_>>();
    if limitation_rows.len() != 5 {
        return Err(format!(
            "live Podman limitation catalogue must contain five cells, found {}",
            limitation_rows.len()
        ));
    }
    let mut limited_ids = BTreeSet::new();
    for row in &limitation_rows {
        if row.len() != 2 || row[1] != "helper-privilege-collision" {
            return Err(format!("invalid live Podman limitation row: {row:?}"));
        }
        if !limited_ids.insert(row[0]) {
            return Err(format!("live Podman limitation is duplicated: {}", row[0]));
        }
        let matrix_row = rows
            .iter()
            .find(|matrix_row| matrix_row[0] == row[0])
            .ok_or_else(|| format!("limited Podman cell is absent from the matrix: {}", row[0]))?;
        if matrix_row[4] != "rootless" || matrix_row[5] != "container" {
            return Err(format!(
                "limited Podman cell must remain a rootless container row: {}",
                row[0]
            ));
        }
    }

    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "validates the bounded candidate catalogue as one cross-field transaction"
)]
fn validate_podman_revalidation_candidates(
    catalogue: &str,
    matrix: &str,
    limitations: &str,
) -> Result<Vec<String>, String> {
    let document = toml::from_str::<toml::Value>(catalogue)
        .map_err(|error| format!("failed to parse Podman revalidation catalogue: {error}"))?;
    let root = document
        .as_table()
        .ok_or("Podman revalidation catalogue must be a TOML table")?;
    let root_keys = root.keys().map(String::as_str).collect::<BTreeSet<_>>();
    if root_keys != BTreeSet::from(["schema", "candidates"])
        || root.get("schema").and_then(toml::Value::as_integer) != Some(1)
    {
        return Err("Podman revalidation catalogue must have only schema 1 and candidates".to_owned());
    }

    let matrix_rows = matrix
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .collect::<Vec<_>>();
    let matrix_by_id = matrix_rows
        .iter()
        .map(|row| {
            row.first()
                .map(|id| (*id, row.as_slice()))
                .ok_or("Podman revalidation matrix contains an empty row")
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let mut limitation_ids = limitations
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let fields = line.split('\t').collect::<Vec<_>>();
            if fields.as_slice().get(1) != Some(&"helper-privilege-collision") {
                return Err(format!("unexpected Podman limitation row: {line}"));
            }
            Ok(fields[0].to_owned())
        })
        .collect::<Result<Vec<_>, String>>()?;
    limitation_ids.sort();

    let candidates = root
        .get("candidates")
        .and_then(toml::Value::as_array)
        .ok_or("Podman revalidation candidates must be an array")?;
    if candidates.len() > limitation_ids.len() {
        return Err(format!(
            "Podman revalidation catalogue has more candidates than limitations: {}",
            candidates.len()
        ));
    }
    let candidate_keys = BTreeSet::from([
        "id",
        "baseline-image",
        "candidate-image",
        "expected-podman-version",
        "expected-distribution",
        "expected-baseline-observed-distribution",
        "expected-replacement-observed-distribution",
        "expected-mode",
        "expected-lane",
        "expected-architecture",
        "expected-limitation",
        "published-at",
        "source-repository",
        "source-revision",
        "source-license",
        "redistribution",
        "source-files",
    ]);
    let source_file_keys = BTreeSet::from(["role", "path", "sha256"]);
    let mut candidate_ids = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let candidate = candidate
            .as_table()
            .ok_or_else(|| format!("Podman revalidation candidate {index} must be a table"))?;
        if candidate.keys().map(String::as_str).collect::<BTreeSet<_>>() != candidate_keys {
            return Err(format!(
                "Podman revalidation candidate {index} has an inexact field set"
            ));
        }
        let field = |name: &str| {
            candidate
                .get(name)
                .and_then(toml::Value::as_str)
                .ok_or_else(|| format!("Podman revalidation candidate {index}.{name} must be a string"))
        };
        let id = field("id")?;
        let matrix_row = matrix_by_id
            .get(id)
            .ok_or_else(|| format!("Podman revalidation candidate {id} is absent from the matrix"))?;
        if matrix_row.len() != 7 {
            return Err(format!("Podman revalidation matrix row {id} is malformed"));
        }
        let baseline = field("baseline-image")?;
        let replacement = field("candidate-image")?;
        if baseline != matrix_row[1]
            || field("expected-podman-version")? != matrix_row[2]
            || field("expected-distribution")? != matrix_row[3]
            || field("expected-mode")? != matrix_row[4]
            || field("expected-lane")? != matrix_row[5]
            || field("expected-architecture")? != matrix_row[6]
        {
            return Err(format!(
                "Podman revalidation candidate {id} differs from its reviewed matrix row"
            ));
        }
        for (name, expected) in [
            ("expected-mode", "rootless"),
            ("expected-lane", "container"),
            ("expected-architecture", "amd64"),
            ("expected-limitation", "helper-privilege-collision"),
            ("source-repository", "https://github.com/Strukturpiloten/containers"),
            ("source-license", "AGPL-3.0-only"),
            ("redistribution", "transient-test-pull"),
        ] {
            if field(name)? != expected {
                return Err(format!("Podman revalidation candidate {id}.{name} must be {expected}"));
            }
        }
        let (baseline_name, baseline_digest) = baseline
            .rsplit_once("@sha256:")
            .ok_or_else(|| format!("Podman revalidation baseline {id} is not immutable"))?;
        let (replacement_name, replacement_digest) = replacement
            .rsplit_once("@sha256:")
            .ok_or_else(|| format!("Podman revalidation replacement {id} is not immutable"))?;
        if baseline_name != replacement_name
            || baseline_digest == replacement_digest
            || !is_lower_sha256(baseline_digest)
            || !is_lower_sha256(replacement_digest)
        {
            return Err(format!(
                "Podman revalidation candidate {id} must change only its immutable digest"
            ));
        }
        let source_revision = field("source-revision")?;
        if source_revision.len() != 40 || !is_lower_hex(source_revision) {
            return Err(format!(
                "Podman revalidation candidate {id} has an invalid source revision"
            ));
        }
        let published_at = field("published-at")?;
        if published_at.len() != 20 || !published_at.ends_with('Z') {
            return Err(format!(
                "Podman revalidation candidate {id} has an invalid publication timestamp"
            ));
        }

        let distribution = field("expected-distribution")?;
        for name in [
            "expected-baseline-observed-distribution",
            "expected-replacement-observed-distribution",
        ] {
            let observed = field(name)?;
            if !observed_distribution_matches(distribution, observed) {
                return Err(format!(
                    "Podman revalidation candidate {id}.{name} is outside {distribution}"
                ));
            }
        }
        if distribution == "opensuse-tumbleweed"
            && field("expected-baseline-observed-distribution")? == field("expected-replacement-observed-distribution")?
        {
            return Err(format!(
                "Podman revalidation candidate {id} must distinguish Tumbleweed snapshots"
            ));
        }
        let expected_source_paths = BTreeMap::from([
            ("image-definition", format!("images/podman/{id}/container.yaml")),
            (
                "platform-recipe",
                format!("images/podman/platforms/{distribution}/Containerfile"),
            ),
            (
                "runtime-config",
                format!("images/podman/platforms/{distribution}/containers.conf"),
            ),
        ]);
        let source_files = candidate
            .get("source-files")
            .and_then(toml::Value::as_array)
            .ok_or_else(|| format!("Podman revalidation candidate {id} source-files must be an array"))?;
        if source_files.len() != 3 {
            return Err(format!(
                "Podman revalidation candidate {id} must have three source files"
            ));
        }
        let mut roles = BTreeSet::new();
        let mut paths = BTreeSet::new();
        for source_file in source_files {
            let source_file = source_file
                .as_table()
                .ok_or_else(|| format!("Podman revalidation candidate {id} source file must be a table"))?;
            if source_file.keys().map(String::as_str).collect::<BTreeSet<_>>() != source_file_keys {
                return Err(format!(
                    "Podman revalidation candidate {id} source file has an inexact field set"
                ));
            }
            let role = source_file
                .get("role")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| format!("Podman revalidation candidate {id} source role is invalid"))?;
            let path = source_file
                .get("path")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| format!("Podman revalidation candidate {id} source path is invalid"))?;
            let digest = source_file
                .get("sha256")
                .and_then(toml::Value::as_str)
                .ok_or_else(|| format!("Podman revalidation candidate {id} source digest is invalid"))?;
            if expected_source_paths.get(role).map(String::as_str) != Some(path)
                || !roles.insert(role)
                || !paths.insert(path)
                || !is_lower_sha256(digest)
            {
                return Err(format!(
                    "Podman revalidation candidate {id} has invalid or cross-wired source proof"
                ));
            }
        }
        candidate_ids.push(id.to_owned());
    }

    let mut sorted_ids = candidate_ids.clone();
    sorted_ids.sort();
    if candidate_ids != sorted_ids
        || candidate_ids.iter().collect::<BTreeSet<_>>().len() != candidate_ids.len()
        || !candidate_ids.iter().all(|id| limitation_ids.contains(id))
    {
        return Err("Podman revalidation candidates must be unique, sorted active limitations".to_owned());
    }
    Ok(candidate_ids)
}

fn is_lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_lower_sha256(value: &str) -> bool {
    value.len() == 64 && is_lower_hex(value)
}

fn observed_distribution_matches(expected: &str, observed: &str) -> bool {
    match expected {
        "opensuse-leap-16.0" => observed == "opensuse-leap-16.0",
        "opensuse-tumbleweed" => {
            let Some(snapshot) = observed.strip_prefix("opensuse-tumbleweed-") else {
                return false;
            };
            snapshot.len() == 8 && snapshot.bytes().all(|byte| byte.is_ascii_digit())
        }
        "ubi-8" => observed == "ubi-8.10",
        "ubi-9" => observed == "ubi-9.8",
        "ubi-10" => observed == "ubi-10.2",
        _ => false,
    }
}

fn validate_live_scenarios(scenarios: &str) -> Result<(), String> {
    let scenario_rows = scenarios
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .collect::<Vec<_>>();
    if scenario_rows.len() != 31 {
        return Err(format!(
            "live Podman scenario catalogue must contain thirty-one cases, found {}",
            scenario_rows.len()
        ));
    }
    let scenario_ids = scenario_rows
        .iter()
        .map(|row| {
            if row.len() == 5 {
                Ok(row[0])
            } else {
                Err(format!("live Podman scenario row must have five fields: {row:?}"))
            }
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    for required in [
        "exact-small",
        "prefix-large",
        "label-large",
        "all-resources",
        "network-boundary",
        "invalid-literal-glob",
        "socket-discovery",
        "compose-reimports",
        "quadlet-reimports",
        "neutral-projection-equivalence",
        "deterministic-exact-compose",
        "strict-policy-blocks",
        "stopped-and-running",
        "healthy-and-unhealthy",
        "pod-members-and-standalone",
        "image-identities",
        "network-boundaries",
        "mount-matrix",
        "selinux-relabel-promotion",
        "environment-matrix",
        "runtime-policy-matrix",
        "secret-conditional",
        "protected-redaction-support-bundle",
        "malformed-selected-container",
        "disappeared-selected-container",
        "partial-inventory-section",
        "external-apply-reacquire",
        "nextcloud-application-runtime",
        "forgejo-application-runtime",
        "paperless-application-runtime",
        "immich-application-runtime",
    ] {
        if !scenario_ids.contains(required) {
            return Err(format!("live Podman scenario catalogue is missing {required}"));
        }
    }

    Ok(())
}

fn validate_paperless_live_runner(runner: &str) -> Result<(), String> {
    for required in [
        "paperless-ngx/paperless-ngx:3.1.3@sha256:aa810a36942c63d4ee70d00eda7236cd3d6acfb7eb3f7987fb568ed14df8817a",
        "valkey/valkey:9.1.2-alpine@sha256:a0dbf4c1d5708782907c10e2c72deff317518518b5288a58416981d9db95d30b",
        "postgres:18.6-alpine@sha256:d3e1620b530c944afa6e887d22eb899824da68e19c52024bf98f5220c88a65b2",
        "gotenberg/gotenberg:8.34.0@sha256:67097317623a503ba2a6a7e9ae8db6929a1f7e1bbd88077bacf2d325fbdab923",
        "apache/tika:3.3.1.0@sha256:90b7fa1dc018434075fce9e1d9b88b1e3d0ea6979d0cf86e116c79a8073ae973",
        "transient-test-pull",
    ] {
        if !runner.contains(required) {
            return Err(format!("Paperless live runner is missing `{required}`"));
        }
    }
    Ok(())
}

fn validate_immich_live_runner(runner: &str) -> Result<(), String> {
    for required in [
        "immich-app/immich-server:v3.1.0@sha256:b434cb9287eea1471c9974845914d4dd328c9c2d652e446ed4930f99944f0ceb",
        "immich-app/immich-machine-learning:v3.1.0@sha256:5a0839dc5303cd7215bcd2180a26aed3af41675aefb3e75e5157e9f10ad16e6e",
        "valkey/valkey:9@sha256:8e8d64b405ce18f41b8e5ee20aa4687a8ed0022d1298f2ce31cdcf3a76e09411",
        "immich-app/postgres:14-vectorchord0.4.3-pgvectors0.2.0@sha256:bcf63357191b76a916ae5eb93464d65c07511da41e3bf7a8416db519b40b1c23",
        "IMMICH_MIN_CPUS=\"2\"",
        "IMMICH_MIN_MEMORY_KIB=\"8388608\"",
        "IMMICH_MIN_DISK_KIB=\"12582912\"",
        "IMMICH_ARCHIVE_MAX_BYTES=\"2684354560\"",
        "runtime_root:?caller must supply runtime_root",
        "AGPL-3.0-only AND PostgreSQL AND (AGPL-3.0-only OR Elastic-2.0) AND Apache-2.0",
        "run_immich_application_cell()",
        "podman-6.1-rootless-rootless",
        "--pull=never",
        "immich_ingest_phase",
        "immich_verify_asset",
        "immich_assert_database",
        "immich_assert_storage_permissions",
        "vchord:0.4.3",
        "asset_job_status",
        "immich_assert_ml_boundary",
        "'/predict'",
        "BF_IMMICH_URL=http://immich-server:2283",
        "BF_IMMICH_URL=http://127.0.0.1:${IMMICH_HTTP_PORT}",
        "run --rm --pull=never --network host",
        "127.0.0.1:18283:2283",
        "immich-library",
        "immich-model-cache",
        "immich-pgdata",
        "immich-redisdata",
        "immich_expect_collision",
        "immich_run_exports",
        "BOXFERRY_IMMICH_CAPTURE_DIRECTORY",
        "immich_capture_candidate",
        "--application immich",
        "immich-application-6.1.0-rootless.cassette.json",
        "BoxFerry-generated artifacts",
    ] {
        if !runner.contains(required) {
            return Err(format!("Immich live runner is missing `{required}`"));
        }
    }
    Ok(())
}

fn validate_external_apply_reacquire_runner(runner: &str) -> Result<(), String> {
    let target_start = runner
        .find("start_apply_target() {")
        .ok_or("live Podman runner is missing apply-target startup")?;
    let target_tail = &runner[target_start..];
    let target_end = target_tail
        .find("\n}\n\nrun_external_apply_reacquire()")
        .ok_or("live Podman apply-target startup boundary is ambiguous")?;
    if target_tail[..target_end].contains("activate_outer_runtime") {
        return Err("live Podman apply target must not activate its API before direct CLI work finishes".to_owned());
    }

    let start = runner
        .find("run_external_apply_reacquire() {")
        .ok_or("live Podman runner is missing external apply/reacquire")?;
    let tail = &runner[start..];
    let end = tail
        .find("\n}\n\nrun_invalid_glob()")
        .ok_or("live Podman external apply/reacquire boundary is ambiguous")?;
    let contract = &tail[..end];

    for required in [
        "(.external_preconditions | length == 0)",
        "select(.action == \"create\" and .resource.kind == \"network\" and .resource.name == $network)] |\n        length == 1)",
        "select(.action == \"create\" and .resource.kind == \"volume\" and .resource.name == $volume)] |\n        length == 1)",
        "select(.action == \"create\" and .resource.kind == \"container\" and .resource.name == $container)] |\n        length == 1)",
        "select(.action == \"create\") |\n        [.resource.kind, .resource.name]] | sort) ==",
        "([[\"network\", $network], [\"volume\", $volume], [\"container\", $container]] | sort)",
        "Generated apply plan did not promote its named network and volume exactly once.",
        "timed_operation 3m 'execute generated plan inside apply target'",
        "activate_outer_runtime \"${runtime_root}/apply-target\"",
        "cmp --silent \"${current_case}/outputs/apply-source-podman/podman.json\"",
        "cmp --silent \"${current_case}/outputs/apply-source-compose/compose.yaml\"",
        "engine_operation 'remove applied target container through API' \\\n    --url \"unix://${apply_target_socket}\"",
        "engine_operation 'remove applied target network through API' \\\n    --url \"unix://${apply_target_socket}\" network rm",
        "engine_operation 'remove applied target volume through API' \\\n    --url \"unix://${apply_target_socket}\" volume rm",
        "for kind in container network volume; do",
        "\"${engine}\" --url \"unix://${apply_target_socket}\" \"${kind}\" exists \"${name}\"",
    ] {
        if !contract.contains(required) {
            return Err(format!(
                "live Podman external apply/reacquire lacks promoted-resource contract: {required}"
            ));
        }
    }
    for stale in [
        "(.external_preconditions | length == 2)",
        "engine_operation 'create apply-target network'",
        "engine_operation 'create apply-target volume'",
        "exec \"${apply_target_outer}\" podman rm",
        "exec \"${apply_target_outer}\" podman network rm",
        "exec \"${apply_target_outer}\" podman volume rm",
    ] {
        if contract.contains(stale) {
            return Err(format!(
                "live Podman external apply/reacquire retained stale prerequisite setup: {stale}"
            ));
        }
    }
    let execution = contract
        .find("timed_operation 3m 'execute generated plan inside apply target'")
        .ok_or("live Podman external apply/reacquire lacks plan execution")?;
    let inspected = contract
        .find("engine_operation 'inspect applied target container'")
        .ok_or("live Podman external apply/reacquire lacks pre-API inspection")?;
    let activation = contract
        .find("activate_outer_runtime \"${runtime_root}/apply-target\"")
        .ok_or("live Podman external apply/reacquire lacks API activation")?;
    let reacquisition = contract
        .find("run_convert podman \"${apply_target_socket}\" apply-target")
        .ok_or("live Podman external apply/reacquire lacks target reacquisition")?;
    if !(execution < inspected && inspected < activation && activation < reacquisition) {
        return Err("live Podman external apply/reacquire must finish CLI work before API activation".to_owned());
    }

    Ok(())
}

#[test]
fn live_external_apply_reacquire_rejects_stale_prerequisite_contracts() -> Result<(), String> {
    let runner = fs::read_to_string(repository_root().join("scripts/podman-live-conformance.sh"))
        .map_err(|error| format!("failed to read live Podman runner: {error}"))?;
    validate_external_apply_reacquire_runner(&runner)?;

    let mutations = [
        remove_first(
            &runner,
            "select(.action == \"create\" and .resource.kind == \"network\" and .resource.name == $network)",
        )?,
        runner.replacen(
            "(.external_preconditions | length == 0)",
            "(.external_preconditions | length == 2)",
            1,
        ),
        runner.replacen(
            "  start_apply_target\n",
            "  start_apply_target\n  engine_operation 'create apply-target network'\n",
            1,
        ),
        runner.replacen("length == 1)", "length >= 1)", 1),
        runner.replacen(
            "[.resource.kind, .resource.name]] | sort) ==",
            "[.resource.kind, .resource.name]] | sort) !=",
            1,
        ),
        remove_first(
            &runner,
            "cmp --silent \"${current_case}/outputs/apply-source-podman/podman.json\"",
        )?,
        remove_first(
            &runner,
            "cmp --silent \"${current_case}/outputs/apply-source-compose/compose.yaml\"",
        )?,
        remove_first(&runner, "for kind in container network volume; do")?,
        runner
            .replacen(
                "  activate_outer_runtime \"${runtime_root}/apply-target\"\n",
                "",
                1,
            )
            .replacen(
                "  timed_operation 3m 'execute generated plan inside apply target'",
                "  activate_outer_runtime \"${runtime_root}/apply-target\"\n  timed_operation 3m 'execute generated plan inside apply target'",
                1,
            ),
        runner.replacen(
            "  verify_observed_version \"${id}-apply-target\"",
            "  activate_outer_runtime \"${socket_directory}\"\n  verify_observed_version \"${id}-apply-target\"",
            1,
        ),
    ];
    for changed in mutations {
        if validate_external_apply_reacquire_runner(&changed).is_ok() {
            return Err("live Podman apply/reacquire policy accepted a stale prerequisite contract".to_owned());
        }
    }

    Ok(())
}

fn validate_live_runner(runner: &str, matrix: &str) -> Result<(), String> {
    validate_paperless_live_runner(runner)?;
    validate_immich_live_runner(runner)?;
    validate_external_apply_reacquire_runner(runner)?;
    for required in [
        "--podman-resource-prefix",
        "--podman-label",
        "--podman-all",
        "--matrix-start-at",
        "network-boundary",
        "run_invalid_glob",
        "run_reimports",
        "run_external_apply_reacquire",
        "run_discovery",
        "should_run_discovery",
        "run_limited_cell",
        "helper-privilege-collision",
        "resource_coverage",
        "coverage_level=smoke",
        "local -a selections=(exact)",
        "TEST %d/%d START",
        "TEST %d/%d PASS",
        "is_smoke_diagnostics_cell",
        "engine_image_available",
        "engine_operation 'read outer Podman version'",
        "start_clean_acquisition_outer",
        "runtime-canaries.log",
        ">> \"${canary_log}\" 2>&1 < /dev/null",
        "exec > /boxferry-socket/bootstrap.log 2>&1",
        "exec --detach --env \"BF_PREFIX=${prefix}\"",
        "resource-setup.status",
        "wait_for_workload_completion",
        "selected-container-id",
        "smoke-baseline.json",
        "remove previous apply target",
        "timed_operation 5m 'pull digest-pinned workload image'",
        "podman load --input /boxferry-workload.tar",
        "--privileged",
        "--device /dev/fuse",
        "image inspect --format '{{.Digest}}'",
        "cap_setuid=ep",
        "cap_setgid=ep",
        "run_nextcloud_application_cell()",
        "local nested_archive=${5:-${workload_archive}}",
        "run_nextcloud_application_cell \"$@\"",
        "nextcloud_assert_clean_prefix",
        "podman_socket \"${socket}\" \"Nextcloud ${1:-command}\" \"$@\"",
        "nextcloud_webdav_round_trip \"${socket}\" \"${prefix}\" \"${mode}\" false",
        "user: www-data",
        "create --pull=never --user www-data --name \"${prefix}-cloud-init\"",
        "- redis:/data",
        "${prefix}-cloud-redis:/data",
        "pull_policy: never",
        "forgejo/forgejo:16.0.3-rootless@sha256:214f4ae63ee78be1e445e58573c88dc7215e72091210852e0df94eaac1a25685",
        "alpine/git:v2.54.0@sha256:6f3b5029566da8e90b24945933dcd806be866b64b1e706f51828bf84faccf21b",
        "php -r '$$s=json_decode",
        "nextcloud:32.0.10-apache@sha256:611669115cccef3f96aa8eb47bd07c4d57452d894ebcfc1d81f5e8ce368e7d2d",
        "postgres:17.6-alpine@sha256:ef257d85f76e48da1c64832459b59fcaba1a4dac97bf5d7450c77753542eee94",
        "redis:8.2.1-alpine@sha256:987c376c727652f99625c7d205a1cba3cb2c53b92b0b62aade2bd48ee1593232",
        "library/nginx:1.29.1-alpine@sha256:42a516af16b852e33b7682d5ef8acbd5d13fe08fecadc7ed98605ba5e3b26ab8",
        "c57ab918abd5b05ca7e7d0f275875dd1330a695074f309dc9eab1b49efafcd4b",
        "downloaded-test-tool",
        ".versionstring == \"32.0.10\"",
    ] {
        if !runner.contains(required) {
            return Err(format!("live Podman runner is missing `{required}`"));
        }
    }
    for forbidden in [
        "/var/run/docker.sock",
        "/run/podman/podman.sock",
        "--volume ${repository_root}",
    ] {
        if runner.contains(forbidden) {
            return Err(format!("live Podman runner must not expose `{forbidden}` to an image"));
        }
    }
    validate_live_application_cell(runner)?;
    validate_live_forgejo_application_cells(runner)?;
    validate_live_paperless_application_cell(runner)?;
    validate_live_immich_application_cell(runner)?;
    validate_live_observability_application_cell(runner)?;
    validate_live_supabase_application_cell(runner, matrix)?;
    validate_live_target_contracts(runner)?;

    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "keeps the complete failure and result contract auditable as one table"
)]
fn validate_limitation_revalidation_runner(runner: &str) -> Result<(), String> {
    for required in [
        "--profile <smoke|full-container|limitation-revalidation|application|forgejo-application|paperless-application|immich-application|observability-application|supabase-application>",
        "--candidate-cell is required with --profile limitation-revalidation.",
        "--candidate-cell is valid only with --profile limitation-revalidation.",
        "initialize-evidence",
        "binding_arguments=(",
        "--candidate-cell \"${candidate_cell}\"",
        "--repository-commit \"${repository_commit}\"",
        "\"catalogues\": null",
        "write_revalidation_initialization_failure preflight invalid-invocation",
        "write_revalidation_initialization_failure preflight prerequisite-unavailable",
        "write_revalidation_initialization_failure catalogue invalid-catalogue",
        "trap 'handle_revalidation_signal INT 130' INT",
        "trap 'handle_revalidation_signal TERM 143' TERM",
        "revalidation_failure_code=interrupted",
        "record-observation",
        "mark-result",
        "local cleanup_role=${6:-}",
        "replacement) revalidation_candidate_outer=\"${outer}\" ;;",
        "apply-target) revalidation_apply_target_outer=\"${outer}\" ;;",
        "cleanup_role=replacement",
        "cleanup_role=apply-target",
        "\"${workload_archive}\" \"${cleanup_role}\"",
        "record_revalidation_candidate_runtime_observations",
        "ensure-failure-evidence",
        "finalize-evidence",
        "assert_revalidation_required_checks",
        "Duplicate required limitation-revalidation check",
        "Missing mandatory limitation-revalidation check",
        "Unexpected limitation-revalidation check",
        "[[ \"${#revalidation_checks[@]}\" == \"${#revalidation_required_checks[@]}\" ]]",
        "[[ \"${progress_total}\" == \"${#revalidation_required_checks[@]}\" ]]",
        "[[ \"${profile}\" == full-container || \"${profile}\" == limitation-revalidation ||\n    \"${profile}\" == observability-application || \"${profile}\" == supabase-application ]]",
        "for selection in 'exact container' 'prefix selection' 'label selection' 'all resources' 'network boundary'; do",
        "for exporter in compose quadlet podman; do",
        "mark_revalidation_result \"selector_exporters.${selection}.${exporter}\"",
        "Historical limitation %s is stale: rootless Podman initialized.",
        "'.[\"org.opencontainers.image.created\"]'",
        "\"${observed_distribution}\" == \"${candidate_baseline_observed_distribution}\"",
        "\"${observed_distribution}\" == \"${candidate_replacement_observed_distribution}\"",
        "Candidate reused the historical outer runtime identity.",
        "Candidate reused the historical socket namespace.",
        "Candidate reused the historical graph-root identity.",
        "Candidate reused the historical application-name prefix.",
        "Candidate outer runtime survived limitation-revalidation cleanup.",
        "External-apply target survived limitation-revalidation cleanup.",
    ] {
        if !runner.contains(required) {
            return Err(format!("Podman limitation-revalidation runner is missing `{required}`"));
        }
    }
    for forbidden in [
        "image inspect --format '{{.Created}}'",
        "date -u -d \"${observed_timestamp}\"",
    ] {
        if runner.contains(forbidden) {
            return Err(format!(
                "Podman limitation-revalidation runner retains ambiguous timestamp parsing `{forbidden}`"
            ));
        }
    }

    let prepare_start = runner
        .find("prepare_matrix_image() {")
        .ok_or("Podman live runner is missing matrix image preparation")?;
    let prepare_end = runner[prepare_start..]
        .find("\nstart_outer_runtime() {")
        .ok_or("Podman live runner is missing matrix image preparation boundary")?
        + prepare_start;
    require_ordered_contracts(
        &runner[prepare_start..prepare_end],
        "Podman limitation-revalidation digest classification",
        &[
            "local classify_revalidation_digest=${3:-false}",
            "if [[ \"${resolved_digest}\" != \"${expected_digest}\" ]]; then",
            "revalidation_failure_code=\"digest-mismatch\"",
            "Resolved matrix image digest mismatch for %s",
        ],
    )?;
    let outer_start = runner
        .find("start_outer_runtime() {")
        .ok_or("Podman live runner is missing outer runtime entry point")?;
    let outer_end = runner[outer_start..]
        .find("\nstart_outer() {")
        .ok_or("Podman live runner is missing outer runtime boundary")?
        + outer_start;
    require_ordered_contracts(
        &runner[outer_start..outer_end],
        "Podman limitation-revalidation cleanup-role registration",
        &[
            "\"${engine}\" run --detach --rm",
            "replacement) revalidation_candidate_outer=\"${outer}\" ;;",
            "apply-target) revalidation_apply_target_outer=\"${outer}\" ;;",
            "wait for nested runtime evidence",
            "record_revalidation_candidate_runtime_observations \"${id}\" \"${outer}\"",
            "Matrix rootless cell did not report rootless Podman",
        ],
    )?;

    let baseline_start = runner
        .find("run_revalidation_baseline_collision() {")
        .ok_or("Podman limitation-revalidation runner is missing its baseline entry point")?;
    let baseline_end = runner[baseline_start..]
        .find("\nconfigure_revalidation_required_checks() {")
        .ok_or("Podman limitation-revalidation runner is missing its baseline boundary")?
        + baseline_start;
    let baseline = &runner[baseline_start..baseline_end];
    for required in [
        "local expected_digest=\"${image##*@}\"",
        "[[ \"$(< \"${artifact_root}/${id}.digest\")\" == \"${expected_digest}\" ]]",
    ] {
        if !baseline.contains(required) {
            return Err(format!(
                "Podman limitation-revalidation baseline is missing `{required}`"
            ));
        }
    }
    require_ordered_contracts(
        baseline,
        "Podman limitation-revalidation baseline classification",
        &[
            "revalidation_phase=\"baseline-pull\"",
            "revalidation_failure_code=\"pull-failed\"",
            "prepare_matrix_image \"${id}\" \"${image}\" true",
            "revalidation_phase=\"baseline-metadata\"",
            "record_revalidation_observation baseline.observed.distribution",
            "revalidation_phase=\"baseline-collision\"",
            "exec \"${outer}\" podman info",
        ],
    )?;

    for result in [
        "baseline.historical_collision.podman_info_failed",
        "baseline.historical_collision.newuidmap_reported",
        "baseline.historical_collision.permission_denied_reported",
        "baseline.historical_collision.newuidmap_setuid",
        "baseline.historical_collision.newgidmap_setuid",
        "baseline.historical_collision.newuidmap_capability",
        "baseline.historical_collision.newgidmap_capability",
        "fresh_store.distinct_runtime",
        "fresh_store.distinct_socket",
        "fresh_store.distinct_graph_root",
        "fresh_store.distinct_name_prefix",
        "runtime_results.resource_creation",
        "runtime_results.runtime_semantics",
        "runtime_results.deterministic_export",
        "runtime_results.strict_policy",
        "runtime_results.literal_glob_rejection",
        "runtime_results.support_bundle",
        "runtime_results.malformed_response",
        "runtime_results.disappeared_resource",
        "runtime_results.partial_inventory",
        "runtime_results.selinux_intent",
        "reimports.compose",
        "reimports.quadlet",
        "external_apply.performed",
        "external_apply.plan_applied",
        "external_apply.reacquired",
        "diagnostic_privacy.redaction_passed",
        "diagnostic_privacy.raw_outputs_absent",
        "diagnostic_privacy.environment_values_absent",
        "diagnostic_privacy.host_paths_absent",
        "diagnostic_privacy.runtime_identifiers_absent",
        "cleanup.baseline_removed",
        "cleanup.replacement_removed",
        "cleanup.apply_target_removed",
    ] {
        if !runner.contains(result) {
            return Err(format!(
                "Podman limitation-revalidation runner is missing result `{result}`"
            ));
        }
    }

    for (phase, code) in [
        ("catalogue", "invalid-catalogue"),
        ("baseline-pull", "pull-failed"),
        ("baseline-metadata", "baseline-metadata-mismatch"),
        ("baseline-collision", "historical-collision-not-reproduced"),
        ("baseline-cleanup", "cleanup-failed"),
        ("replacement-pull", "pull-failed"),
        ("replacement-provenance", "source-proof-mismatch"),
        ("replacement-runtime", "runtime-metadata-mismatch"),
        ("resource-suite", "resource-contract-failed"),
        ("external-apply", "external-apply-failed"),
        ("cleanup", "cleanup-failed"),
        ("evidence", "evidence-invalid"),
    ] {
        if !has_failure_classification(runner, phase, code) {
            return Err(format!(
                "Podman limitation-revalidation runner is missing failure classification {phase}/{code}"
            ));
        }
    }

    let start = runner
        .find("run_limitation_revalidation() {")
        .ok_or("Podman limitation-revalidation runner is missing its entry point")?;
    require_ordered_contracts(
        &runner[start..],
        "Podman limitation-revalidation runner",
        &[
            "run_revalidation_baseline_collision \"${candidate_cell}\"",
            "revalidation_phase=\"replacement-pull\"",
            "revalidation_failure_code=\"pull-failed\"",
            "prepare_matrix_image \"${candidate_cell}\" \"${candidate_replacement_image}\" true",
            "verify_revalidation_candidate_provenance",
            "run_cell \"${candidate_cell}\" \"${candidate_replacement_image}\"",
            "assert_revalidation_required_checks",
            "revalidation_ready_to_finalize=true",
        ],
    )?;
    let cleanup = runner
        .find("cleanup() {")
        .ok_or("Podman limitation-revalidation runner is missing its EXIT cleanup")?;
    require_ordered_contracts(
        &runner[cleanup..],
        "Podman limitation-revalidation EXIT cleanup",
        &[
            "container exists \"${outer}\"",
            "mark_revalidation_result cleanup.baseline_removed",
            "mark_revalidation_result cleanup.replacement_removed",
            "mark_revalidation_result cleanup.apply_target_removed",
            "\"${revalidation_helper}\" finalize-evidence",
            "\"${revalidation_helper}\" validate-evidence",
            "revalidation_complete=true",
            "\"${revalidation_helper}\" ensure-failure-evidence",
            "trap cleanup EXIT",
        ],
    )?;
    Ok(())
}

fn has_failure_classification(runner: &str, phase: &str, code: &str) -> bool {
    let phase_assignment = format!("revalidation_phase=\"{phase}\"");
    let code_assignment = format!("revalidation_failure_code=\"{code}\"");
    runner.match_indices(&phase_assignment).any(|(offset, _)| {
        let following = &runner[offset + phase_assignment.len()..];
        let boundary = following.find("revalidation_phase=").unwrap_or(following.len());
        following[..boundary].contains(&code_assignment)
    })
}

fn require_ordered_contracts(mut contents: &str, context: &str, contracts: &[&str]) -> Result<(), String> {
    for contract in contracts {
        let offset = contents
            .find(contract)
            .ok_or_else(|| format!("{context} is missing or misorders `{contract}`"))?;
        contents = &contents[offset + contract.len()..];
    }
    Ok(())
}

fn validate_live_target_contracts(runner: &str) -> Result<(), String> {
    for apply_target_contract in [
        "'$1 == \"podman-6.1-rootful\" { print; exit }'",
        "\"${id}\" == podman-6.1-rootful",
        "\"${declared_version}\" == 6.1.0",
    ] {
        if !runner.contains(apply_target_contract) {
            return Err(format!(
                "live Podman runner must pin its external apply target: `{apply_target_contract}`"
            ));
        }
    }
    for default_network_contract in [
        "current_default_podman_network_present",
        "scenario_podman_socket \"${socket}\" network ls --format json",
        "run_reimports \"${declared_version%%+*}\"",
        "assert_default_podman_network_evidence() {",
        "local version=${3:?caller must supply declared Podman version}",
        "[[ \"${present}\" == true && \"${version}\" == 3.0.1 && \"${rootless}\" == false ]]",
        ".code == \"BFP0002\"",
        ".source_code == \"PLN0023\"",
        ".name == \"subject\" and .value == \"network:podman\"",
        "PodmanLens found native response fields without typed portable mappings; path descriptors were retained without values",
        ".name == \"decision\" and .value == \"omitted\"",
        ".name == \"source_engine\" and .value == \"3.0.1\"",
        ".name == \"source_api\" and .value == \"3.0.0\"",
        ".name == \"native_path\" and .value == \"$.CniConfig\"",
        ".name == \"native_value_policy\"",
        "and (any(",
        ".code == \"BFQ0007\"",
        ".name == \"subject\" and .value == \"networks.podman\"",
        ") | not)",
        "default Podman network absent from live inventory; default-network ownership diagnostic is inapplicable",
    ] {
        if !runner.contains(default_network_contract) {
            return Err(format!(
                "live Podman runner must make its default-network Quadlet assertion inventory-aware: `{default_network_contract}`"
            ));
        }
    }

    Ok(())
}

fn validate_live_application_cell(runner: &str) -> Result<(), String> {
    for application_contract in [
        "application) [[ \"${id}\" == podman-6.1-rootless && (-z \"${matrix_cell}\" || \"${id}\" == \"${matrix_cell}\") ]] ;;",
        "[[ \"${id}-${mode}\" == podman-6.1-rootless-rootless ]]",
    ] {
        if !runner.contains(application_contract) {
            return Err(format!(
                "live Podman runner must pin the rootless application cell: `{application_contract}`"
            ));
        }
    }
    if runner.contains("podman-6.1-rootful-rootful ||") {
        return Err("live Podman application profile must not admit the unverified rootful cell".to_owned());
    }
    Ok(())
}

fn validate_live_forgejo_application_cells(runner: &str) -> Result<(), String> {
    for contract in [
        "--profile <smoke|full-container|limitation-revalidation|application|forgejo-application|paperless-application|immich-application|observability-application|supabase-application>",
        "run_forgejo_application_cell()",
        "forgejo_assert_clean_prefix",
        "forgejo_git_probe",
        "forgejo_clear_probe_state",
        "forgejo_expect_collision",
        "forgejo_peer_compose_project",
        "Rootless Podman can remove a container before reporting a netns teardown error.",
        "--project-name \"${prefix}-peer\"",
        "firewall_driver=none drop-in would prevent real HTTP and SSH DNAT",
        "forgejo-application)",
        "\"${id}\" == podman-arch-rootful || \"${id}\" == podman-6.1-rootless",
        "podman-arch-rootful-rootful | podman-6.1-rootless-rootless",
        "prepare rootless Forgejo network configuration",
        "verify rootful Forgejo target uses stock firewall configuration",
        "upstream-source 6.1 rootful target omits nft and cannot install real DNAT.",
    ] {
        if !runner.contains(contract) {
            return Err(format!(
                "live Podman runner must pin both Forgejo application cells: `{contract}`"
            ));
        }
    }
    Ok(())
}

#[test]
fn paperless_probe_cleanup_uses_one_exact_network_isolated_container() -> Result<(), String> {
    let root = repository_root();
    let output = Command::new("bash")
        .args([
            "-c",
            r#"
set -euo pipefail
source "$1"
paperless_remote() {
  printf '%s\0' "$@"
}

paperless_clear_probe_state fixture.sock safe-prefix
"#,
            "paperless-cleanup-contract",
        ])
        .arg(root.join("scripts/lib/paperless-application.sh"))
        .current_dir(&root)
        .output()
        .map_err(|error| format!("failed to exercise Paperless cleanup helper: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Paperless cleanup helper failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let mut expected = [
        "fixture.sock",
        "run",
        "--rm",
        "--pull=never",
        "--network",
        "none",
        "--user",
        "0:0",
        "--volume",
        "/tmp/boxferry-fixture/safe-prefix:/fixture:rw",
        "--entrypoint",
        "/bin/sh",
        "registry.invalid/boxferry-test/paperless-application:paperless",
        "-ceu",
        "rm -rf -- /fixture/probe-state.json /fixture/generated-baseline /fixture/generated-second",
    ]
    .join("\0")
    .into_bytes();
    expected.push(0);
    if output.stdout != expected {
        return Err("Paperless cleanup must pass only the reviewed cleanup-container argv".to_owned());
    }
    Ok(())
}

#[test]
fn immich_probe_cleanup_uses_one_exact_network_isolated_container() -> Result<(), String> {
    let root = repository_root();
    let output = Command::new("bash")
        .args([
            "-c",
            r#"
set -euo pipefail
source "$1"
immich_remote() { printf '%s\0' "$@"; }
immich_clear_probe_state fixture.sock safe-prefix
"#,
            "immich-cleanup-contract",
        ])
        .arg(root.join("scripts/lib/immich-application.sh"))
        .current_dir(&root)
        .output()
        .map_err(|error| format!("failed to exercise Immich cleanup helper: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Immich cleanup helper failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let mut expected = [
        "fixture.sock",
        "run",
        "--rm",
        "--pull=never",
        "--network",
        "none",
        "--user",
        "0:0",
        "--volume",
        "/tmp/boxferry-fixture/safe-prefix:/fixture:rw",
        "--entrypoint",
        "/bin/sh",
        "registry.invalid/boxferry-test/immich-application:machine-learning",
        "-ceu",
        "rm -rf -- /fixture/probe-state.json /fixture/generated-baseline",
    ]
    .join("\0")
    .into_bytes();
    expected.push(0);
    if output.stdout != expected {
        return Err("Immich cleanup must pass only the reviewed cleanup-container argv".to_owned());
    }
    Ok(())
}

#[test]
fn immich_valkey_activity_parser_sums_command_calls() -> Result<(), String> {
    let root = repository_root();
    let output = Command::new("bash")
        .args([
            "-c",
            r#"
set -euo pipefail
source "$1"
immich_remote() {
  printf '%s\n' \
    'cmdstat_get:calls=5,usec=10,usec_per_call=2.00' \
    'cmdstat_set:calls=7,usec=21,usec_per_call=3.00'
}
immich_valkey_commands fixture.sock safe-prefix
"#,
            "immich-valkey-activity-contract",
        ])
        .arg(root.join("scripts/lib/immich-application.sh"))
        .current_dir(&root)
        .output()
        .map_err(|error| format!("failed to exercise Immich Valkey parser: {error}"))?;
    if !output.status.success() || output.stdout != b"12\n" {
        return Err(format!(
            "Immich Valkey parser must sum calls, stdout={:?}, stderr={:?}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

#[test]
fn paperless_compose_startup_and_recreation_are_staged_around_readiness() -> Result<(), String> {
    let root = repository_root();
    let output = Command::new("bash")
        .args([
            "-c",
            r#"
set -euo pipefail
source "$1"
current_case="$(mktemp -d "${TMPDIR:-/tmp}/boxferry-paperless-compose-contract.XXXXXX")"
trap 'rm -rf -- "${current_case}"' EXIT
exec 3>&1
paperless_assert_clean_prefix() {
  printf 'clean\0' >&3
  printf '%s\0' "$@" >&3
}
paperless_compose_project() {
  printf 'compose\0' >&3
  printf '%s\0' "$@" >&3
}
paperless_wait_for() {
  printf 'wait\0' >&3
  printf '%s\0' "$@" >&3
}
paperless_wait_application() {
  printf 'ready\0' >&3
  printf '%s\0' "$@" >&3
}
paperless_provision_compose fixture.sock safe-prefix run-id
paperless_recreate_application compose fixture.sock safe-prefix run-id
"#,
            "paperless-compose-startup-contract",
        ])
        .arg(root.join("scripts/lib/paperless-application.sh"))
        .current_dir(&root)
        .output()
        .map_err(|error| format!("failed to exercise Paperless Compose startup: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "Paperless Compose startup helper failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let expected = concat!(
        "clean\0fixture.sock\0safe-prefix\0",
        "compose\0fixture.sock\0safe-prefix\0run-id\0up\0--detach\0--no-deps\0",
        "--remove-orphans\0db\0broker\0gotenberg\0tika\0",
        "wait\0240\0Docker Compose PostgreSQL readiness\0paperless_remote\0fixture.sock\0",
        "exec\0safe-prefix-paper-db\0pg_isready\0-U\0paperless\0-d\0paperless\0",
        "wait\0240\0Docker Compose Valkey readiness\0paperless_remote\0fixture.sock\0",
        "exec\0safe-prefix-paper-broker\0valkey-cli\0--no-auth-warning\0-a\0",
        "boxferry-public-broker-canary\0ping\0",
        "compose\0fixture.sock\0safe-prefix\0run-id\0up\0--detach\0--no-deps\0webserver\0",
        "compose\0fixture.sock\0safe-prefix\0run-id\0stop\0--timeout\030\0",
        "compose\0fixture.sock\0safe-prefix\0run-id\0rm\0--force\0",
        "compose\0fixture.sock\0safe-prefix\0run-id\0up\0--detach\0--no-deps\0",
        "--remove-orphans\0db\0broker\0gotenberg\0tika\0",
        "wait\0240\0Docker Compose PostgreSQL readiness\0paperless_remote\0fixture.sock\0",
        "exec\0safe-prefix-paper-db\0pg_isready\0-U\0paperless\0-d\0paperless\0",
        "wait\0240\0Docker Compose Valkey readiness\0paperless_remote\0fixture.sock\0",
        "exec\0safe-prefix-paper-broker\0valkey-cli\0--no-auth-warning\0-a\0",
        "boxferry-public-broker-canary\0ping\0",
        "compose\0fixture.sock\0safe-prefix\0run-id\0up\0--detach\0--no-deps\0webserver\0",
        "ready\0fixture.sock\0safe-prefix\0",
    )
    .as_bytes();
    if output.stdout != expected {
        return Err("Paperless Compose startup did not preserve reviewed call order/argv".to_owned());
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    let output = Command::new("shasum").args(["-a", "256"]).arg(path).output();
    #[cfg(not(target_os = "macos"))]
    let output = Command::new("sha256sum").arg(path).output();
    let output = output.map_err(|error| format!("failed to hash {}: {error}", path.display()))?;
    if !output.status.success() {
        return Err(format!("failed to hash {}", path.display()));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| format!("digest for {} was not UTF-8: {error}", path.display()))?
        .split_whitespace()
        .next()
        .map(str::to_owned)
        .ok_or_else(|| format!("digest command returned no digest for {}", path.display()))
}

fn verify_capture_privacy(
    root: &Path,
    capture_path: &Path,
    application: &str,
    display_name: &str,
) -> Result<(), String> {
    let verifier = Command::new("python3")
        .arg(root.join("fixtures/conformance/podman-live/capture_proxy.py"))
        .arg("--application")
        .arg(application)
        .arg("--repository")
        .arg(root)
        .arg("--verify-cassette")
        .arg(capture_path)
        .output()
        .map_err(|error| format!("failed to run captured {display_name} privacy verifier: {error}"))?;
    if verifier.status.success() {
        return Ok(());
    }
    Err(format!(
        "captured {display_name} privacy verification failed: {}",
        String::from_utf8_lossy(&verifier.stderr).trim()
    ))
}

fn inspect_redacted_capture(
    value: &serde_json::Value,
    environment_count: &mut usize,
    display_name: &str,
) -> Result<(), String> {
    match value {
        serde_json::Value::Object(values) => {
            for (key, value) in values {
                if matches!(
                    key.to_ascii_lowercase().as_str(),
                    "authorization" | "cookie" | "set-cookie" | "secretdata"
                ) {
                    return Err(format!("captured {display_name} evidence contains forbidden key {key}"));
                }
                inspect_redacted_capture(value, environment_count, display_name)?;
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                inspect_redacted_capture(value, environment_count, display_name)?;
            }
        }
        serde_json::Value::String(text) => {
            let lowered = text.to_ascii_lowercase();
            for marker in [
                "boxferry-public-admin-canary",
                "boxferry-public-database-canary",
                "boxferry-public-broker-canary",
                "boxferry-public-paperless-secret-canary",
                "boxferry-public-immich-db-password-canary",
                "bearer ",
                "basic ",
                "unix://",
                "tcp://",
                "ssh://",
                "/home/",
                "/root/",
                "/run/user/",
                "/tmp/",
                "/var/lib/containers",
                "/run/containers",
                "/capture-input",
                "/capture-socket",
                "podman.sock",
                "sentinel_private",
            ] {
                if lowered.contains(marker) {
                    return Err(format!(
                        "captured {display_name} evidence retained private marker {marker}"
                    ));
                }
            }
            if matches!(lowered.as_str(), "authorization" | "cookie" | "set-cookie") {
                return Err(format!(
                    "captured {display_name} evidence contains a forbidden header name"
                ));
            }
            if let Some((name, environment_value)) = text.split_once('=') {
                if !name.is_empty()
                    && name
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric() || character == '_')
                {
                    *environment_count += 1;
                    if environment_value != "redacted" {
                        return Err(format!(
                            "captured {display_name} environment value was not redacted: {name}"
                        ));
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

#[test]
fn paperless_captured_native_evidence_is_supplementary_and_redacted() -> Result<(), String> {
    let root = repository_root();
    let scenario_path = root.join("fixtures/scenarios/paperless-ngx-application/scenario.toml");
    let scenario_text = fs::read_to_string(&scenario_path)
        .map_err(|error| format!("failed to read {}: {error}", scenario_path.display()))?;
    let scenario = toml::from_str::<toml::Value>(&scenario_text)
        .map_err(|error| format!("invalid Paperless scenario manifest: {error}"))?;
    let podman_input = scenario
        .get("native-inputs")
        .and_then(toml::Value::as_array)
        .and_then(|inputs| {
            inputs
                .iter()
                .find(|input| input.get("id").and_then(toml::Value::as_str) == Some("podman"))
        })
        .ok_or("Paperless scenario must retain one Podman semantic input")?;
    let semantic_files = podman_input
        .get("files")
        .and_then(toml::Value::as_array)
        .ok_or("Paperless Podman semantic input must list its authored cassette")?
        .iter()
        .map(|value| value.as_str().ok_or("Paperless Podman input file must be a string"))
        .collect::<Result<Vec<_>, _>>()?;
    if semantic_files != ["input-podman.cassette.json"] {
        return Err("captured redacted evidence must not replace the authored Paperless semantic cassette".to_owned());
    }
    if podman_input
        .get("podman")
        .and_then(|podman| podman.get("include-environment-values"))
        .and_then(toml::Value::as_bool)
        != Some(true)
    {
        return Err("authored Paperless input must retain semantic environment values".to_owned());
    }

    let capture_path =
        root.join("fixtures/conformance/paperless-ngx-application/paperless-ngx-6.1.0-rootless.cassette.json");
    let capture_bytes =
        fs::read(&capture_path).map_err(|error| format!("failed to read {}: {error}", capture_path.display()))?;
    let observed_digest = sha256_file(&capture_path)?;
    if observed_digest != "427a86d9e8d798ae8a99e8aeeffb5bfc0ef7c94ce366fd268fbe30eb4a4acaea" {
        return Err(format!("captured Paperless evidence digest drifted: {observed_digest}"));
    }
    verify_capture_privacy(&root, &capture_path, "paperless", "Paperless")?;

    let capture = serde_json::from_slice::<serde_json::Value>(&capture_bytes)
        .map_err(|error| format!("invalid captured Paperless evidence: {error}"))?;
    for (pointer, expected) in [
        (
            "/scenario_id",
            "paperless-ngx-application-podman-6.1.0-rootless-captured",
        ),
        ("/engine_version", "6.1.0"),
        ("/api_version", "6.1.0"),
        ("/execution_context", "rootless"),
        (
            "/provenance/evidence_kind",
            "privacy-review-required-one-off-native-capture",
        ),
        (
            "/provenance/capture/runtime_revision",
            "6ef0b9d6c4c8c2bc5708b7be9de19215d151721c",
        ),
        ("/provenance/capture/runtime_matrix_cell", "podman-6.1-rootless"),
        (
            "/provenance/capture/runtime_matrix_sha256",
            "1ed306f4b368c229bca927697156e2314b922c2ec728c55c2820c69a712bad25",
        ),
        (
            "/provenance/capture/capture_manifest_sha256",
            "4a135307f745905f50de1522ecf4d971b78456b30f88adbd0a5f65f710dc01ea",
        ),
    ] {
        if capture.pointer(pointer).and_then(serde_json::Value::as_str) != Some(expected) {
            return Err(format!("captured Paperless provenance drifted at {pointer}"));
        }
    }
    if capture.get("synthetic").and_then(serde_json::Value::as_bool) != Some(false) {
        return Err("captured Paperless evidence must remain explicitly non-synthetic".to_owned());
    }
    if capture
        .get("interactions")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        != Some(27)
    {
        return Err("captured Paperless evidence must retain all 27 interactions".to_owned());
    }

    let mut environment_count = 0;
    inspect_redacted_capture(&capture, &mut environment_count, "Paperless")?;
    if environment_count != 143 {
        return Err(format!(
            "captured Paperless evidence must retain 143 redacted environment assignments, found {environment_count}"
        ));
    }
    Ok(())
}

#[test]
fn immich_captured_native_evidence_is_supplementary_and_redacted() -> Result<(), String> {
    let root = repository_root();
    let scenario_path = root.join("fixtures/scenarios/immich-application/scenario.toml");
    let scenario_text = fs::read_to_string(&scenario_path)
        .map_err(|error| format!("failed to read {}: {error}", scenario_path.display()))?;
    let scenario = toml::from_str::<toml::Value>(&scenario_text)
        .map_err(|error| format!("invalid Immich scenario manifest: {error}"))?;
    let podman_input = scenario
        .get("native-inputs")
        .and_then(toml::Value::as_array)
        .and_then(|inputs| {
            inputs
                .iter()
                .find(|input| input.get("id").and_then(toml::Value::as_str) == Some("podman"))
        })
        .ok_or("Immich scenario must retain one Podman semantic input")?;
    let semantic_files = podman_input
        .get("files")
        .and_then(toml::Value::as_array)
        .ok_or("Immich Podman semantic input must list its authored cassette")?
        .iter()
        .map(|value| value.as_str().ok_or("Immich Podman input file must be a string"))
        .collect::<Result<Vec<_>, _>>()?;
    if semantic_files != ["input-podman.cassette.json"] {
        return Err("captured redacted evidence must not replace the authored Immich semantic cassette".to_owned());
    }
    if podman_input
        .get("podman")
        .and_then(|podman| podman.get("include-environment-values"))
        .and_then(toml::Value::as_bool)
        != Some(true)
    {
        return Err("authored Immich input must retain semantic environment values".to_owned());
    }

    let capture_path =
        root.join("fixtures/conformance/immich-application/immich-application-6.1.0-rootless.cassette.json");
    let capture_bytes =
        fs::read(&capture_path).map_err(|error| format!("failed to read {}: {error}", capture_path.display()))?;
    let observed_digest = sha256_file(&capture_path)?;
    if observed_digest != "743f7983e64578e6c82068307e1dcbeb7ab64ee3ba0baf7b82aa89d789673a78" {
        return Err(format!("captured Immich evidence digest drifted: {observed_digest}"));
    }
    verify_capture_privacy(&root, &capture_path, "immich", "Immich")?;

    let capture = serde_json::from_slice::<serde_json::Value>(&capture_bytes)
        .map_err(|error| format!("invalid captured Immich evidence: {error}"))?;
    for (pointer, expected) in [
        ("/scenario_id", "immich-application-podman-6.1.0-rootless-captured"),
        ("/engine_version", "6.1.0"),
        ("/api_version", "6.1.0"),
        ("/execution_context", "rootless"),
        (
            "/provenance/evidence_kind",
            "privacy-review-required-one-off-native-capture",
        ),
        (
            "/provenance/capture/runtime_revision",
            "6ef0b9d6c4c8c2bc5708b7be9de19215d151721c",
        ),
        ("/provenance/capture/runtime_matrix_cell", "podman-6.1-rootless"),
        (
            "/provenance/capture/runtime_matrix_sha256",
            "1ed306f4b368c229bca927697156e2314b922c2ec728c55c2820c69a712bad25",
        ),
        (
            "/provenance/capture/capture_manifest_sha256",
            "f04e364c5417fad7f024e9261ca2df110066dd1f094856b350dadc0c975ee6ae",
        ),
    ] {
        if capture.pointer(pointer).and_then(serde_json::Value::as_str) != Some(expected) {
            return Err(format!("captured Immich provenance drifted at {pointer}"));
        }
    }
    if capture.get("synthetic").and_then(serde_json::Value::as_bool) != Some(false) {
        return Err("captured Immich evidence must remain explicitly non-synthetic".to_owned());
    }
    if capture
        .get("interactions")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        != Some(23)
    {
        return Err("captured Immich evidence must retain all 23 interactions".to_owned());
    }

    let mut environment_count = 0;
    inspect_redacted_capture(&capture, &mut environment_count, "Immich")?;
    if environment_count != 149 {
        return Err(format!(
            "captured Immich evidence must retain 149 redacted environment assignments, found {environment_count}"
        ));
    }
    Ok(())
}

fn validate_live_observability_application_cell(runner: &str) -> Result<(), String> {
    for required in [
        "--profile <smoke|full-container|limitation-revalidation|application|forgejo-application|paperless-application|immich-application|observability-application|supabase-application>",
        "observability-application)",
        "run_observability_application_cell \"$@\"",
    ] {
        if !runner.contains(required) {
            return Err(format!(
                "live Podman runner must retain observability entry-point contract: `{required}`"
            ));
        }
    }

    for required in [
        "[[ \"${id}-${mode}\" == podman-6.1-rootless-rootless ]]",
        "OBSERVABILITY_MIN_CPUS=\"2\"",
        "OBSERVABILITY_MIN_MEMORY_KIB=\"4194304\"",
        "OBSERVABILITY_MIN_DISK_KIB=\"8388608\"",
        "OBSERVABILITY_ARCHIVE_MAX_BYTES=\"2147483648\"",
        "OBSERVABILITY_PROVIDER_VERSION=\"5.5.0\"",
        "c57ab918abd5b05ca7e7d0f275875dd1330a695074f309dc9eab1b49efafcd4b",
        "prom/prometheus:v3.14.0@sha256:5ce7540c3c00ef4ab0c9d2c995c6a5b9c421f44b4a115d97a2c7af3b1c21cbb0",
        "grafana/loki:3.7.7@sha256:d70e4659623f3e109af669cae76fe2a5dd5be54e2298fe8aed380d982fbc2500",
        "grafana/grafana:13.2.1@sha256:f772d434e8fab0049deb2b1b30abd43342bcfca1537614aa8d36080232cf4283",
        "grafana/alloy:v1.19.2@sha256:b8ec653c44235fbe910879145dac3597d66b0aaecf60bcbbe82580767771a839",
        "nginx:1.29.1-alpine@sha256:42a516af16b852e33b7682d5ef8acbd5d13fe08fecadc7ed98605ba5e3b26ab8",
        "boxferry_fixture_temperature_celsius{source=\"controlled\"}",
        "query_range?query=%7Bjob%3D%22boxferry_fixture%22%7D%20%7C%3D%20%22boxferry-observability-known-log%22",
        "retention_period: 24h",
        "--storage.tsdb.retention.time=24h",
        "telemetry-logs:/var/log/boxferry:ro",
        "observability_assert_queries_and_grafana",
        "observability_assert_reviewed_diagnostics",
        "observability_write_expected_diagnostics",
        "route=\"${source_kind}-${output}\"",
        "compose-podman | quadlet-podman",
        "compose-compose | compose-quadlet | quadlet-compose | quadlet-quadlet",
        "if (subject ~ /^volumes\\./)",
        "decision = \"approximated\"",
        "policy = \"approximate\"",
        "services.%s%s.mounts[%d]",
        "grafana 3",
        "max_over_time%28boxferry_fixture_temperature_celsius%7Bsource%3D%22controlled%22%7D%5B30m%5D%29",
        "--project-name \"${prefix}-observability\"",
        "\"${selection}\" \"${input}\" \"${output}\" \"${result}\" \"${prefix}\" \"${report}\"",
        "expected.${route}.diagnostics",
        "diff --unified \"${expected}\" \"${observed}\"",
        "field(\"required_loss_policy\")",
        "observability_assert_application_boundaries",
        "observability_assert_storage_ownership",
        "observability_prepare_persistence_sentinels",
        "observability_assert_persistence",
        "observability_run_exports",
        "observability_run_reimports",
        "observability_expect_collision",
        "observability_cleanup_mode",
        "progress_total=35",
    ] {
        if !runner.contains(required) {
            return Err(format!("observability helper and fixtures must retain `{required}`"));
        }
    }

    for forbidden in [
        "--volume /var/run/docker.sock",
        "--volume /run/podman/podman.sock",
        "--volume /var/log/journal",
        "--volume /var/lib/docker/containers",
    ] {
        if runner.contains(forbidden) {
            return Err(format!(
                "observability acceptance must not expose production source `{forbidden}`"
            ));
        }
    }
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "keeps the complete Supabase live contract auditable in one place"
)]
fn validate_live_supabase_application_cell(runner: &str, matrix: &str) -> Result<(), String> {
    let exact_cell = "podman-6.1-rootless\tghcr.io/strukturpiloten/podman-6.1-rootless:v6.1.0@sha256:dd00fadfff6e732728643df565a5db50f6d36dc3ec2d7f23a1fe87e905e08b5e\t6.1.0\tupstream-source\trootless\tcontainer\tamd64";
    if !matrix.lines().any(|line| line == exact_cell) {
        return Err(
            "Supabase application target must remain the exact reviewed Podman 6.1.0 rootless amd64 cell".to_owned(),
        );
    }
    for contract in [
        "--profile <smoke|full-container|limitation-revalidation|application|forgejo-application|paperless-application|immich-application|observability-application|supabase-application>",
        "--profile must be smoke, full-container, limitation-revalidation, application, forgejo-application, paperless-application, immich-application, observability-application, or supabase-application.",
        "supabase-application)\n      [[ \"${id}\" == podman-6.1-rootless &&",
        "run_supabase_application_cell \"$@\"",
        "[[ \"${id}-${mode}\" == podman-6.1-rootless-rootless ]]",
        "[[ \"${architecture}\" == amd64 &&",
        "[[ \"${declared_version}\" =~ ^6\\.1($|\\.) ]]",
    ] {
        if !runner.contains(contract) {
            return Err(format!(
                "live Podman runner must retain Supabase entry-point and target contract: `{contract}`"
            ));
        }
    }

    for contract in [
        "SUPABASE_MIN_CPUS=\"4\"",
        "SUPABASE_MIN_MEMORY_KIB=\"12582912\"",
        "SUPABASE_MIN_DISK_KIB=\"25165824\"",
        "SUPABASE_ARCHIVE_MAX_BYTES=\"5368709120\"",
        "SUPABASE_MAX_CONCURRENCY=\"1\"",
        "SUPABASE_CELL_TIMEOUT=\"90m\"",
        "cpus=\"$(nproc)\"",
        "memory_kib=\"$(awk '$1 == \"MemAvailable:\" { print $2 }' /proc/meminfo)\"",
        "info --format '{{.Store.GraphRoot}}'",
        "for path in \"${runtime_root:?caller must supply runtime_root}\" \"${graph_root}\"; do",
        "progress_total=40",
        "supabase_validate_resource_budget",
        "supabase_prepare_image_archive",
    ] {
        if !runner.contains(contract) {
            return Err(format!("Supabase resource contract is missing `{contract}`"));
        }
    }

    let cell = runner
        .split_once("supabase_run_application_cell_unbounded() {")
        .and_then(|(_, following)| {
            following
                .split_once("\nrun_supabase_application_cell() {")
                .map(|(cell, _)| cell)
        })
        .ok_or("Supabase numbered cell body could not be isolated")?;
    let progress_checks = cell.matches("progress_run '").count();
    if progress_checks != 40 {
        return Err(format!(
            "Supabase application cell must contain 40 numbered checks, found {progress_checks}"
        ));
    }
    for invocation in [
        "supabase_provision_cli \"${socket}\" \"${current_prefix}\" \"${run_id}\"",
        "supabase_probe \"${socket}\" \"${current_prefix}\" seed",
        "supabase_probe \"${socket}\" \"${current_prefix}\" verify",
        "supabase_run_exports cli \"${socket}\" \"${current_prefix}\"",
        "supabase_run_reimports cli \"${current_prefix}\"",
        "supabase_recreate_application cli \"${socket}\" \"${current_prefix}\" \"${run_id}\"",
        "supabase_assert_database_state \"${socket}\" \"${current_prefix}\" 2",
        "supabase_cleanup_mode cli \"${socket}\" \"${current_prefix}\" \"${run_id}\"",
        "supabase_provision_compose \"${socket}\" \"${current_prefix}\" \"${run_id}\"",
        "supabase_run_exports compose \"${socket}\" \"${current_prefix}\"",
        "supabase_run_reimports compose \"${current_prefix}\"",
        "supabase_recreate_application compose \"${socket}\" \"${current_prefix}\" \"${run_id}\"",
        "supabase_cleanup_mode compose \"${socket}\" \"${current_prefix}\" \"${run_id}\"",
        "remove_outer \"${outer}\"",
    ] {
        if !cell.contains(invocation) {
            return Err(format!(
                "Supabase numbered cell is missing executable contract `{invocation}`"
            ));
        }
    }

    for image in [
        "docker.io/supabase/studio:2026.08.03-sha-022b374@sha256:606aca9fdaa753b60968d5c304e2ada83869b76c9043684e63d5885aca9550e8",
        "docker.io/kong/kong:3.9.3@sha256:9a2ae6699a2ce0d60592eb176555d3594a22782c20cc6557a61ff3a7e8b559a3",
        "docker.io/supabase/gotrue:v2.189.0@sha256:385184459f57569c54c25209f51f3b2be99ddd7c4ce9e3555b5d3eea8447b7cf",
        "docker.io/postgrest/postgrest:v14.12@sha256:54000f24847d01a2c2302e0041cf0618b875c57fb48507d743cfa9aaa50bf43c",
        "docker.io/supabase/realtime:v2.102.3@sha256:aa1c92c0cf326007563641730ec9da9c60478caa6853887775365fa2c097a471",
        "docker.io/supabase/storage-api:v1.60.4@sha256:c8eb9858eafec891a97c27125470aaad54703c3f4eb4d55ca7f1bf6c6411febf",
        "docker.io/darthsim/imgproxy:v3.30.1@sha256:3b709e4a0e5e8e0e959b556b7031229202b4b8e7e7d955c517ea7abed68ee34d",
        "docker.io/supabase/postgres-meta:v0.96.6@sha256:a84cc713585eea7b401e4a2561ec4a1e48c87083d1c7ecb4502f204bb4391300",
        "docker.io/supabase/edge-runtime:v1.74.0@sha256:2781daf92394db91f7e94129cc3d04ec474ad16a8fe64b3fbeef6e7d557ab120",
        "docker.io/supabase/postgres:17.6.1.136@sha256:f371b5f3f2ac0a05703f33d6e6134515fb2498cab708fb948a0aeb7481467c00",
        "docker.io/supabase/supavisor:2.9.5@sha256:31c2f05b13b11069660fdfae2f6cfd37b509748d2710aca121cfee8b16cb8b07",
    ] {
        if !runner.contains(image) {
            return Err(format!("Supabase image catalogue is missing exact pin `{image}`"));
        }
    }
    for provider in [
        "SUPABASE_PROVIDER_VERSION=\"5.5.0\"",
        "SUPABASE_PROVIDER_SHA256=\"c57ab918abd5b05ca7e7d0f275875dd1330a695074f309dc9eab1b49efafcd4b\"",
        "https://github.com/docker/compose/releases/download/v5.5.0/docker-compose-linux-x86_64",
    ] {
        if !runner.contains(provider) {
            return Err(format!("Supabase provider contract is missing `{provider}`"));
        }
    }

    for contract in [
        "supabase_provision_cli",
        "supabase_provision_compose",
        "supabase_wait_application",
        "supabase_enable_realtime_table",
        "ALTER PUBLICATION supabase_realtime ADD TABLE public.boxferry_items;",
        "/boxferry-fixture/application-probe.mjs \"${phase}\"",
        "jsonRequest(\"/auth/v1/signup\"",
        "jsonRequest(\"/auth/v1/token?grant_type=password\"",
        "new WebSocket(",
        "jsonRequest(\"/rest/v1/boxferry_items",
        "request(\"/storage/v1/object/authenticated/boxferry/probe.txt\"",
        "jsonRequest(\"/functions/v1/main\"",
        "request(\"/studio/api/platform/profile\")",
        "fetch(`${supavisorBase}/api/health`)",
        "supabase_probe_published_api",
        "http://127.0.0.1:${SUPABASE_HTTP_PORT}/auth/v1/health",
        "supabase_assert_application_boundaries",
        ".[0].HostConfig.PortBindings == {}",
        ".[0].HostConfig.PortBindings[\"8000/tcp\"][0].HostIp == \"127.0.0.1\"",
        "supabase_assert_storage_ownership",
        ".Destination == \"/var/lib/storage\" and .RW == false",
        "supabase_expect_collision",
        "[[ \"${status}\" == 125 ]]",
        "for selection in exact storage label all; do",
        "selection_arguments=(--podman-resource \"container=${prefix}-supabase-kong\")",
        "selection_arguments=(--podman-label io.boxferry.selection=storage)",
        "selection_arguments=(--podman-label \"io.boxferry.application=${prefix}-supabase\")",
        "selection_arguments=(--podman-all)",
        "for output in compose quadlet podman; do",
        "supabase_recreate_application",
        "supabase_assert_report_privacy",
        "--include='*.report.json'",
        "supabase_assert_clean_resources",
        "index($0, prefix) == 1",
    ] {
        if !runner.contains(contract) {
            return Err(format!("Supabase live behavior contract is missing `{contract}`"));
        }
    }

    let privacy = runner
        .split_once("supabase_assert_report_privacy() {")
        .and_then(|(_, following)| {
            following
                .split_once("\nsupabase_remove_cli_application_containers() {")
                .map(|(privacy, _)| privacy)
        })
        .ok_or("Supabase report-privacy body could not be isolated")?;
    for protected_value in [
        "-e \"${SUPABASE_DB_PASSWORD}\"",
        "-e \"${SUPABASE_JWT_SECRET}\"",
        "-e \"${SUPABASE_ANON_KEY}\"",
        "-e \"${SUPABASE_SERVICE_KEY}\"",
        "-e \"${SUPABASE_REALTIME_SECRET}\"",
        "-e \"${SUPABASE_POOLER_SECRET}\"",
        "-e \"${SUPABASE_REALTIME_DB_KEY}\"",
        "-e \"${SUPABASE_META_CRYPTO_KEY}\"",
        "-e \"${SUPABASE_VAULT_ENC_KEY}\"",
        "-e \"${SUPABASE_TEST_PASSWORD}\"",
    ] {
        if !privacy.contains(protected_value) {
            return Err(format!(
                "Supabase report privacy omits protected value `{protected_value}`"
            ));
        }
    }

    for fixture_contract in [
        "9952d6f10fb9a7b2d9d8c3312b279bfab2c4ba96",
        "CREATE TABLE IF NOT EXISTS public.boxferry_items",
        "Deno.serve({ port: 9000 }",
        "crypto.subtle.digest(\"SHA-256\"",
        "Phoenix-WebSocket-PostgreSQL-insert",
        "private-bucket-upload-byte-exact-download",
        "seed-SHA-256-cd2c400852048a021086994cc5f266472d53e72ffddb1c1f5d01a17ddaa27ca4-and-verify-SHA-256-71059a67ee64b2891c41a31b66660b18e366342f765ccf07fd68fb0436eb6638",
        "one-error-per-tag-and-digest-image-no-artifacts",
        ".primary_diagnostic_code == \"BFP0008\"",
        "(.output_artifacts | length) == 0",
        "($actual_subjects | length) == ($actual_subjects | unique | length)",
        ".fidelity.unsupported == $fidelity.unsupported",
        ".fidelity.invalid == $fidelity.invalid",
        "expected_rejection_fidelity",
        "expected_success_fidelity",
        "exact_fidelity_shape",
        "length(seen) != 9 || success != 7 || rejected != 2",
    ] {
        if !runner.contains(fixture_contract) {
            return Err(format!(
                "Supabase fixture and route contract is missing `{fixture_contract}`"
            ));
        }
    }

    for route in [
        "podman\tcompose\tmigration-success\tlive-unperformed\tBFP0002,BFP0003,BFC0007,BFC0009\texact-diagnostic-tuple-multiset-plus-fidelity-v1",
        "podman\tquadlet\tmigration-success\tlive-unperformed\tBFP0002,BFP0003\texact-diagnostic-tuple-multiset-plus-fidelity-v1",
        "podman\tpodman\tmigration-success\tlive-unperformed\tBFP0002,BFP0003,BFP0007\texact-diagnostic-tuple-multiset-plus-fidelity-v1",
        "compose\tcompose\tmigration-success\tlive-unperformed\tBFC0009\texact-diagnostic-tuple-multiset-plus-fidelity-v1",
        "compose\tquadlet\tmigration-success\tlive-unperformed\t-\tzero-loss-zero-diagnostic-reimport",
        "quadlet\tcompose\tmigration-success\tlive-unperformed\tBFC0007,BFC0009\texact-diagnostic-tuple-multiset-plus-fidelity-v1",
        "quadlet\tquadlet\tmigration-success\tlive-unperformed\t-\tzero-loss-zero-diagnostic-reimport",
        "compose\tpodman\texpected-rejection\tlive-unperformed\tBFP0007,BFP0008\tone-error-per-tag-and-digest-image-no-artifacts",
        "quadlet\tpodman\texpected-rejection\tlive-unperformed\tBFP0007,BFP0008\tone-error-per-tag-and-digest-image-no-artifacts",
    ] {
        if !runner.contains(route) {
            return Err(format!("Supabase route catalogue is missing exact row `{route}`"));
        }
    }
    Ok(())
}

fn validate_live_paperless_application_cell(runner: &str) -> Result<(), String> {
    const PORTABLE_AF_UNIX_PATH_BYTES: usize = 104;
    const CAPTURE_RUNTIME_ROOT_TEMPLATE: &str = "/tmp/boxferry-podman-live.XXXXXX";
    const CAPTURE_SOCKET_SUFFIX: &str = "/paperless-capture.sock";

    let capture_socket_template = format!("{CAPTURE_RUNTIME_ROOT_TEMPLATE}{CAPTURE_SOCKET_SUFFIX}");
    if capture_socket_template.len() >= PORTABLE_AF_UNIX_PATH_BYTES {
        return Err(format!(
            "Paperless capture proxy socket template is {} bytes; it must leave room for the AF_UNIX terminator within {PORTABLE_AF_UNIX_PATH_BYTES} bytes",
            capture_socket_template.len()
        ));
    }
    if runner.contains("${current_case}/paperless-capture-proxy.sock") {
        return Err("Paperless capture proxy socket must not inherit the unbounded artifact path".to_owned());
    }

    for contract in [
        "--profile <smoke|full-container|limitation-revalidation|application|forgejo-application|paperless-application|immich-application|observability-application|supabase-application>",
        "paperless-application)",
        "run_paperless_application_cell()",
        "run_paperless_application_cell \"$@\"",
        "[[ \"${id}-${mode}\" == podman-6.1-rootless-rootless ]]",
        "paperless_validate_resource_budget",
        "PAPERLESS_MIN_MEMORY_KIB=\"6291456\"",
        "PAPERLESS_MIN_DISK_KIB=\"12582912\"",
        "PAPERLESS_ARCHIVE_MAX_BYTES=\"2684354560\"",
        "paperless_expect_collision",
        "--detach --no-deps --remove-orphans db broker gotenberg tika",
        "Docker Compose PostgreSQL readiness",
        "Docker Compose Valkey readiness",
        "--detach --no-deps webserver",
        "paperless_ingest_phase",
        "paperless_assert_database",
        "paperless_assert_storage_permissions",
        "BF_PAPERLESS_URL=http://127.0.0.1:${PAPERLESS_HTTP_PORT}",
        "run --rm --pull=never --network host",
        "paperless_verify_documents",
        "paperless_run_exports",
        "BOXFERRY_PAPERLESS_CAPTURE_DIRECTORY",
        "paperless_capture_candidate",
        "runtime_root=\"$(mktemp -d /tmp/boxferry-podman-live.XXXXXX)\"",
        "local proxy_socket=\"${runtime_root}/paperless-capture.sock\"",
        "rm -rf -- \"${runtime_root}\"",
        "trap cleanup EXIT",
        "capture sanitized Podman CLI Paperless evidence candidate",
        "TemporaryDirectory(prefix=\"bfcap-\", dir=\"/tmp\")",
        "listener.settimeout(2)",
        "captured-native-sanitized-candidate",
        "Raw requests and responses were never written",
        "paperless_cleanup_mode",
        "PAPERLESS_TASK_WORKERS: \"1\"",
        "PAPERLESS_THREADS_PER_WORKER: \"1\"",
        "PAPERLESS_WEBSERVER_WORKERS: \"1\"",
        "PAPERLESS_TIKA_ENDPOINT: http://${BF_PREFIX}-paper-tika:9998",
        "PAPERLESS_TIKA_GOTENBERG_ENDPOINT: http://${BF_PREFIX}-paper-gotenberg:3000",
        "127.0.0.1:18000:8000",
        "document generation is not deterministic",
        "converter archive for document",
        "--pull=never",
        "does not execute any BoxFerry-generated artifact",
    ] {
        if !runner.contains(contract) {
            return Err(format!(
                "live Podman runner must retain Paperless application contract: `{contract}`"
            ));
        }
    }
    Ok(())
}

fn validate_live_immich_application_cell(runner: &str) -> Result<(), String> {
    const PORTABLE_AF_UNIX_PATH_BYTES: usize = 104;
    const CAPTURE_RUNTIME_ROOT_TEMPLATE: &str = "/tmp/boxferry-podman-live.XXXXXX";
    const CAPTURE_SOCKET_SUFFIX: &str = "/immich-capture.sock";
    let capture_socket_template = format!("{CAPTURE_RUNTIME_ROOT_TEMPLATE}{CAPTURE_SOCKET_SUFFIX}");
    if capture_socket_template.len() >= PORTABLE_AF_UNIX_PATH_BYTES {
        return Err(format!(
            "Immich capture proxy socket template is {} bytes; leave room for the AF_UNIX terminator within {PORTABLE_AF_UNIX_PATH_BYTES} bytes",
            capture_socket_template.len()
        ));
    }
    if runner.contains("${current_case}/immich-capture-proxy.sock") {
        return Err("Immich capture proxy socket must not use the unbounded artifact path".to_owned());
    }
    for contract in [
        "immich-application)",
        "run_immich_application_cell \"$@\"",
        "immich_validate_resource_budget",
        "--detach --no-deps --remove-orphans database redis immich-machine-learning",
        "Docker Compose PostgreSQL readiness",
        "Docker Compose Valkey readiness",
        "ML /ping readiness",
        "--detach --no-deps immich-server",
        "disable_machine_learning",
        "PNG generation is not byte deterministic",
        "metadata extraction has not created exifInfo",
        "preview changed after recreation",
        "thumbnail changed after recreation",
        "SELECT count(*) FROM asset;",
        "SELECT count(*) FROM asset_job_status;",
        "BOXFERRY_IMMICH_CAPTURE_DIRECTORY",
        "local proxy_socket=\"${runtime_root}/immich-capture.sock\"",
        "capture sanitized Podman CLI Immich evidence candidate",
        "'- DB_DATABASE_NAME=immich'",
        "'published: \"18283\"'",
        "immich_cleanup_mode",
        "does not execute BoxFerry-generated artifacts",
    ] {
        if !runner.contains(contract) {
            return Err(format!(
                "live Podman runner must retain Immich application contract: `{contract}`"
            ));
        }
    }
    let boundary_loop = "for container in database redis machine-learning; do";
    if runner.matches(boundary_loop).count() != 2 {
        return Err("Immich boundary checks must inspect each exact container name twice".to_owned());
    }
    if !runner.contains("machine-learning:/cache:model-cache")
        || runner.contains("immich-machine-learning:/cache:model-cache")
    {
        return Err("Immich storage checks must not duplicate the machine-learning prefix".to_owned());
    }
    for storage_contract in [
        "redis:/data:redisdata:sticky",
        "sticky:1777",
        "all(.[0].Mounts[]?; .Name != $redisdata)",
    ] {
        if !runner.contains(storage_contract) {
            return Err(format!(
                "Immich storage checks must retain the service-specific Valkey contract: `{storage_contract}`"
            ));
        }
    }
    Ok(())
}

fn validate_live_workflow(hosted: &str) -> Result<(), String> {
    let required = "sudo env BOXFERRY_BIN=\"${BOXFERRY_BIN}\" bash scripts/podman-live-conformance.sh --profile \"${PROFILE}\" --matrix-cell \"${MATRIX_CELL}\" --engine podman";
    if !hosted.contains(required) {
        return Err(format!(
            "live Podman workflow must invoke the checked-in runner: `{required}`"
        ));
    }
    if hosted.contains("schedule:") {
        return Err("live Podman workflow must not schedule nightly runs".to_owned());
    }
    if hosted.contains("\n  pull_request:") || !hosted.contains("workflow_dispatch:") {
        return Err("hosted live Podman workflow must be manual-only after tier orchestration".to_owned());
    }
    for required in [
        "build-boxferry:",
        "needs: [matrix, build-boxferry]",
        "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7.0.1",
        "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c # v8.0.1",
        "chmod +x target/debug/boxferry",
        "application:",
        "name: Nextcloud application / podman-6.1-rootless",
        "timeout-minutes: 60",
        "https://github.com/docker/compose/releases/download/v5.5.0/docker-compose-linux-x86_64",
        "c57ab918abd5b05ca7e7d0f275875dd1330a695074f309dc9eab1b49efafcd4b",
        "BOXFERRY_COMPOSE_BIN: ${{ github.workspace }}/target/tools/docker-compose",
        "BOXFERRY_COMPOSE_BIN=\"${BOXFERRY_COMPOSE_BIN}\"",
        "--profile application",
        "--matrix-cell podman-6.1-rootless",
        "forgejo-application:",
        "name: Forgejo application / Podman 6.1 Arch rootful and rootless",
        "timeout-minutes: 30",
        "Run checked-in Forgejo application profile",
        "--profile forgejo-application --engine podman",
        "paperless-application:",
        "immich-application:",
        "name: Immich application / podman-6.1-rootless",
        "Run checked-in Immich application profile",
        "--profile immich-application",
        "immich-capture",
        "needs.matrix.outputs.profile != 'immich-capture'",
        "github.event_name == 'workflow_dispatch' && inputs.profile == 'immich-capture'",
        "BOXFERRY_IMMICH_CAPTURE_DIRECTORY: ${{ github.event_name == 'workflow_dispatch' && inputs.profile == 'immich-capture'",
        "immich-native-capture-candidate",
        "${{ runner.temp }}/immich-capture-parent/candidate",
        "name: Paperless-ngx application / podman-6.1-rootless",
        "Run checked-in Paperless-ngx application profile",
        "--profile paperless-application",
        "- paperless-capture",
        "needs.matrix.outputs.profile != 'paperless-capture'",
        "github.event_name == 'workflow_dispatch' && inputs.profile == 'paperless-capture'",
        "BOXFERRY_PAPERLESS_CAPTURE_DIRECTORY: ${{ github.event_name == 'workflow_dispatch' && inputs.profile == 'paperless-capture'",
        "paperless-native-capture-candidate",
        "path: ${{ runner.temp }}/paperless-capture",
        "github.event_name == 'pull_request' &&",
        "sha256sum --check --strict",
    ] {
        if !hosted.contains(required) {
            return Err(format!("hosted live Podman workflow is missing `{required}`"));
        }
    }
    if hosted.contains("--profile forgejo-application --matrix-cell") {
        return Err("Forgejo application workflow must run both reviewed cells".to_owned());
    }
    if hosted.matches("github.event_name == 'pull_request' &&").count() < 3 {
        return Err("each privileged application job must reject fork-authored code".to_owned());
    }

    for smoke in [
        "podman-5.4-rootless",
        "podman-6.1-rootful",
        "podman-6.1-rootless",
        "podman-debian-11-rootful",
        "podman-debian-11-rootless",
        "podman-debian-12-rootful",
        "podman-ubi-8-rootful",
        "podman-ubuntu-22.04-rootless",
        "podman-ubuntu-24.04-rootless",
    ] {
        if !hosted.contains(&format!("$1 == \"{smoke}\"")) {
            return Err(format!(
                "hosted live Podman workflow smoke selection is missing {smoke}"
            ));
        }
    }
    if !hosted.contains("$6 == \"container\"") {
        return Err("hosted full Podman workflow must select every matrix container row".to_owned());
    }
    if !hosted.contains("sudo apt-get install --yes libcap2-bin podman") {
        return Err("hosted Podman workflow must install the capability inspection tool".to_owned());
    }
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "keeps the manual workflow security contract auditable as one policy"
)]
fn validate_podman_revalidation_workflow(workflow: &str, candidate_ids: &[String]) -> Result<(), String> {
    for forbidden in [
        "\n  push:",
        "\n  pull_request:",
        "\n  pull_request_target:",
        "\n  schedule:",
        "\n  repository_dispatch:",
        "continue-on-error:",
        "contents: write",
        "actions: write",
        "id-token:",
        "packages:",
        "actions/cache@",
    ] {
        if workflow.contains(forbidden) {
            return Err(format!("Podman revalidation workflow must not contain `{forbidden}`"));
        }
    }
    for required in [
        "on:\n  workflow_dispatch:\n    inputs:\n      candidate:",
        "required: true",
        "default: all",
        "type: choice",
        "permissions:\n  contents: read",
        "group: podman-limitation-revalidation",
        "cancel-in-progress: false",
        "[[ \"${GITHUB_REPOSITORY}\" == \"Strukturpiloten/boxferry\" ]]",
        "[[ \"${GITHUB_REF}\" == \"refs/heads/main\" ]]",
        "[[ \"${GITHUB_REF}\" == \"refs/heads/${DEFAULT_BRANCH}\" ]]",
        "DEFAULT_BRANCH: ${{ github.event.repository.default_branch }}",
        "python3 scripts/lib/podman-revalidation.py list-candidates",
        "catalogue_output=\"$(",
        "mapfile -t catalogue <<< \"${catalogue_output}\"",
        "has-candidates: ${{ steps.select.outputs.has-candidates }}",
        "cells='[]'",
        "No active replacement candidates; all reviewed limitations remain retained.",
        "printf 'has-candidates=%s\\n'",
        "candidate: ${{ fromJSON(needs.candidates.outputs.cells) }}",
        "fail-fast: false",
        "max-parallel: 1",
        "timeout-minutes: 140",
        "--profile limitation-revalidation",
        "--candidate-cell \"${CANDIDATE_CELL}\"",
        "sudo chown --recursive \"$(id -u):$(id -g)\" target/podman-revalidation",
        "--expected-repository-commit \"${GITHUB_SHA}\"",
        "test -f target/podman-revalidation/evidence-v1.json",
        "name: podman-revalidation-${{ matrix.candidate }}-${{ github.run_id }}-${{ github.run_attempt }}",
        "path: target/podman-revalidation/evidence-v1.json",
        "if-no-files-found: error",
        "REVALIDATION_STATUS: ${{ steps.execute.outputs.status }}",
        "exit \"${REVALIDATION_STATUS}\"",
    ] {
        if !workflow.contains(required) {
            return Err(format!("Podman revalidation workflow is missing `{required}`"));
        }
    }
    if workflow.matches("persist-credentials: false").count() != 3
        || workflow.matches("ref: ${{ github.sha }}").count() != 3
    {
        return Err("every Podman revalidation checkout must be credential-free and exact-SHA".to_owned());
    }
    if workflow
        .matches("if: needs.candidates.outputs.has-candidates == 'true'")
        .count()
        != 2
    {
        return Err("Podman revalidation build and matrix jobs must skip an empty catalogue".to_owned());
    }
    if workflow.contains("mapfile -t catalogue < <(") {
        return Err("Podman revalidation selection must propagate catalogue-helper failure".to_owned());
    }

    let options_start = workflow
        .find("      options:\n")
        .ok_or("Podman revalidation workflow is missing candidate options")?
        + "      options:\n".len();
    let options_end = workflow[options_start..]
        .find("\n\npermissions:")
        .ok_or("Podman revalidation candidate options are not bounded")?
        + options_start;
    let actual_options = workflow[options_start..options_end]
        .lines()
        .map(|line| {
            line.strip_prefix("          - ")
                .ok_or_else(|| format!("invalid Podman revalidation option line `{line}`"))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let expected_options = std::iter::once("all")
        .chain(candidate_ids.iter().map(String::as_str))
        .collect::<Vec<_>>();
    if actual_options != expected_options {
        return Err(format!(
            "Podman revalidation workflow options differ from catalogue: {actual_options:?}"
        ));
    }

    let validate_start = workflow
        .find("- name: Validate bounded evidence against this run")
        .ok_or("Podman revalidation workflow is missing its evidence-validation step")?;
    let upload_start = workflow[validate_start..]
        .find("- name: Upload bounded revalidation evidence")
        .ok_or("Podman revalidation workflow is missing its evidence-upload step")?
        + validate_start;
    let restore_start = workflow[upload_start..]
        .find("- name: Restore revalidation status")
        .ok_or("Podman revalidation workflow is missing status restoration")?
        + upload_start;
    let validation = &workflow[validate_start..upload_start];
    if !validation.contains("if: ${{ always() }}")
        || !validation.contains("scripts/lib/podman-revalidation.py validate-evidence")
        || !validation.contains("--expected-repository-commit \"${GITHUB_SHA}\"")
    {
        return Err("Podman revalidation evidence must always be validated against the exact run SHA".to_owned());
    }
    let upload = &workflow[upload_start..restore_start];
    if !upload.contains("if: ${{ always() }}")
        || upload.matches("path:").count() != 1
        || !upload.contains("path: target/podman-revalidation/evidence-v1.json")
        || !upload.contains("if-no-files-found: error")
        || !upload.contains("retention-days: 1")
    {
        return Err("Podman revalidation must always upload exactly one bounded evidence document".to_owned());
    }
    Ok(())
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "keeps all mutation cases adjacent to the contract they challenge"
)]
fn podman_limitation_revalidation_policy_rejects_counterfactuals() -> Result<(), String> {
    let root = repository_root();
    let runner = fs::read_to_string(root.join("scripts/podman-live-conformance.sh"))
        .map_err(|error| format!("failed to read live Podman runner: {error}"))?;
    let workflow = fs::read_to_string(root.join(".github/workflows/podman-limitation-revalidation.yml"))
        .map_err(|error| format!("failed to read Podman revalidation workflow: {error}"))?;
    let active_catalogue = fs::read_to_string(root.join("fixtures/conformance/podman-live/candidates.toml"))
        .map_err(|error| format!("failed to read Podman revalidation candidates: {error}"))?;
    let catalogue =
        fs::read_to_string(root.join("fixtures/conformance/podman-live/revalidation/34418537575/candidates.toml"))
            .map_err(|error| format!("failed to read archived Podman revalidation candidates: {error}"))?;
    let matrix = fs::read_to_string(root.join("fixtures/conformance/podman-live/matrix.tsv"))
        .map_err(|error| format!("failed to read live Podman matrix: {error}"))?;
    let limitations = fs::read_to_string(root.join("fixtures/conformance/podman-live/limitations.tsv"))
        .map_err(|error| format!("failed to read live Podman limitations: {error}"))?;
    let candidate_ids = validate_podman_revalidation_candidates(&active_catalogue, &matrix, &limitations)?;
    if !candidate_ids.is_empty() {
        return Err("reviewed Podman revalidation catalogue must have no active candidates".to_owned());
    }
    let archived_candidate_ids = validate_podman_revalidation_candidates(&catalogue, &matrix, &limitations)?;
    if archived_candidate_ids.len() != 5 {
        return Err("archived Podman revalidation decision must bind five candidates".to_owned());
    }
    validate_limitation_revalidation_runner(&runner)?;
    validate_podman_revalidation_workflow(&workflow, &candidate_ids)?;

    let last_candidate = catalogue
        .rfind("\n[[candidates]]")
        .ok_or("Podman revalidation catalogue lacks a removable candidate")?;
    for (description, changed) in [
        (
            "boolean catalogue schema",
            catalogue.replacen("schema = 1", "schema = true", 1),
        ),
        ("missing schema", catalogue[last_candidate + 1..].to_owned()),
        (
            "cross-wired Tumbleweed baseline snapshot",
            catalogue.replacen(
                "expected-baseline-observed-distribution = \"opensuse-tumbleweed-20260821\"",
                "expected-baseline-observed-distribution = \"opensuse-tumbleweed-20260904\"",
                1,
            ),
        ),
        (
            "observed distribution outside declared family",
            catalogue.replacen(
                "expected-baseline-observed-distribution = \"ubi-8.10\"",
                "expected-baseline-observed-distribution = \"ubi-9.8\"",
                1,
            ),
        ),
        (
            "cross-wired source path",
            catalogue.replacen(
                "images/podman/platforms/opensuse-leap-16.0/Containerfile",
                "images/podman/platforms/ubi-8/Containerfile",
                1,
            ),
        ),
    ] {
        if validate_podman_revalidation_candidates(&changed, &matrix, &limitations).is_ok() {
            return Err(format!("Podman revalidation candidate policy accepted {description}"));
        }
    }

    let runner_mutations = [
        ("missing runtime result", "runtime_results.resource_creation"),
        ("missing selector", " 'network boundary'"),
        ("missing re-import", "reimports.quadlet"),
        ("missing apply result", "external_apply.reacquired"),
        ("missing cleanup result", "cleanup.apply_target_removed"),
        (
            "missing replacement cleanup registration",
            "replacement) revalidation_candidate_outer=\"${outer}\" ;;",
        ),
        (
            "missing apply-target cleanup registration",
            "apply-target) revalidation_apply_target_outer=\"${outer}\" ;;",
        ),
        (
            "missing failed root-mode observation",
            "record_revalidation_candidate_runtime_observations",
        ),
        ("missing failure evidence", "ensure-failure-evidence"),
        (
            "mismatched baseline digest representation",
            "[[ \"$(< \"${artifact_root}/${id}.digest\")\" == \"${expected_digest}\" ]]",
        ),
        (
            "weakened full-resource admission",
            " || \"${profile}\" == limitation-revalidation",
        ),
    ];
    for (description, needle) in runner_mutations {
        let changed = remove_first(&runner, needle)?;
        if validate_limitation_revalidation_runner(&changed).is_ok() {
            return Err(format!("Podman revalidation runner policy accepted {description}"));
        }
    }
    let changed = replace_first(
        &runner,
        "local baseline_socket_namespace=\"${runtime_root}/revalidation-baseline/${id}\"\n  local expected_digest=\"${image##*@}\"",
        "local baseline_socket_namespace=\"${runtime_root}/revalidation-baseline/${id}\"\n  local expected_digest=\"${image##*@sha256:}\"",
    )?;
    if validate_limitation_revalidation_runner(&changed).is_ok() {
        return Err("Podman revalidation runner policy accepted a prefix-stripped baseline digest".to_owned());
    }

    let changed = remove_first(&runner, "\"${revalidation_helper}\" validate-evidence")?;
    if validate_limitation_revalidation_runner(&changed).is_ok() {
        return Err("Podman revalidation runner policy accepted missing final validation".to_owned());
    }
    let changed = swap_once(
        &runner,
        "run_revalidation_baseline_collision \"${candidate_cell}\"",
        "run_cell \"${candidate_cell}\" \"${candidate_replacement_image}\"",
    )?;
    if validate_limitation_revalidation_runner(&changed).is_ok() {
        return Err("Podman revalidation runner policy accepted reversed baseline order".to_owned());
    }

    let workflow_mutations = [
        (
            "scheduled execution",
            "workflow_dispatch:",
            "workflow_dispatch:\n  schedule:",
        ),
        ("parallel candidates", "max-parallel: 1", "max-parallel: 2"),
        ("string input", "type: choice", "type: string"),
        ("wrong repository", "Strukturpiloten/boxferry", "attacker/boxferry"),
        ("writable token", "contents: read", "contents: write"),
        (
            "masked run failure",
            "id: execute",
            "id: execute\n      continue-on-error: true",
        ),
        ("non-exact checkout", "ref: ${{ github.sha }}", "ref: main"),
        (
            "swallowed catalogue resolver failure",
            "catalogue_output=\"$(",
            "mapfile -t catalogue < <(",
        ),
        (
            "missing zero-candidate build guard",
            "if: needs.candidates.outputs.has-candidates == 'true'",
            "if: ${{ always() }}",
        ),
        (
            "missing evidence ownership restoration",
            "sudo chown --recursive \"$(id -u):$(id -g)\" target/podman-revalidation",
            "true # evidence remains root-owned",
        ),
        (
            "broad evidence upload",
            "path: target/podman-revalidation/evidence-v1.json",
            "path: target/",
        ),
        (
            "missing always validation",
            "if: ${{ always() }}",
            "if: ${{ success() }}",
        ),
    ];
    for (description, from, to) in workflow_mutations {
        let changed = replace_first(&workflow, from, to)?;
        if validate_podman_revalidation_workflow(&changed, &candidate_ids).is_ok() {
            return Err(format!("Podman revalidation workflow policy accepted {description}"));
        }
    }
    if let Some(candidate_id) = candidate_ids.first() {
        let missing_option = format!("          - {candidate_id}\n");
        let changed = remove_first(&workflow, &missing_option)?;
        if validate_podman_revalidation_workflow(&changed, &candidate_ids).is_ok() {
            return Err("Podman revalidation workflow policy accepted a missing candidate".to_owned());
        }
    }
    Ok(())
}

fn replace_first(contents: &str, from: &str, to: &str) -> Result<String, String> {
    if !contents.contains(from) {
        return Err(format!("counterfactual source is missing `{from}`"));
    }
    Ok(contents.replacen(from, to, 1))
}

fn remove_first(contents: &str, needle: &str) -> Result<String, String> {
    replace_first(contents, needle, "")
}

fn swap_once(contents: &str, first: &str, second: &str) -> Result<String, String> {
    let first_offset = contents
        .find(first)
        .ok_or_else(|| format!("counterfactual source is missing `{first}`"))?;
    let second_offset = contents
        .find(second)
        .ok_or_else(|| format!("counterfactual source is missing `{second}`"))?;
    if first_offset >= second_offset {
        return Err("counterfactual source already has reversed ordering".to_owned());
    }
    let mut changed = String::with_capacity(contents.len());
    changed.push_str(&contents[..first_offset]);
    changed.push_str(second);
    changed.push_str(&contents[first_offset + first.len()..second_offset]);
    changed.push_str(first);
    changed.push_str(&contents[second_offset + second.len()..]);
    Ok(changed)
}

#[test]
fn vscode_workspace_configuration_covers_local_development() -> Result<(), String> {
    let root = repository_root();
    for (path, required) in [
        (
            ".vscode/settings.json",
            &[
                "rust-analyzer.check.command",
                "rust-analyzer.cargo.features",
                "editor.formatOnSave",
            ][..],
        ),
        (
            ".vscode/extensions.json",
            &[
                "DavidAnson.vscode-markdownlint",
                "esbenp.prettier-vscode",
                "exiasr.hadolint",
                "mkhl.shfmt",
                "ms-vscode-remote.remote-containers",
                "rust-lang.rust-analyzer",
                "tombi-toml.tombi",
                "timonwong.shellcheck",
                "vadimcn.vscode-lldb",
            ][..],
        ),
        (".vscode/launch.json", &["BoxFerry: CLI help", "lldb", "boxferry"][..]),
        (
            ".vscode/tasks.json",
            &[
                "BoxFerry: Format, lint, and test all",
                "scripts/check-all.sh",
                "BoxFerry: Required Rust checks",
                "BoxFerry: Build workspace",
                "BoxFerry: Test",
                "BoxFerry: Configure GitHub CLI authentication",
                "scripts/configure-github-cli.sh",
                "cargo",
            ][..],
        ),
    ] {
        let text = fs::read_to_string(root.join(path)).map_err(|error| format!("failed to read {path}: {error}"))?;
        for expected in required {
            if !text.contains(expected) {
                return Err(format!("{path} must contain `{expected}`"));
            }
        }
    }

    Ok(())
}

#[test]
fn github_cli_authentication_is_interactive_container_scoped_and_does_not_replace_git_credentials() -> Result<(), String>
{
    let script_path = repository_root().join("scripts/configure-github-cli.sh");
    let script = fs::read_to_string(&script_path)
        .map_err(|error| format!("failed to read {}: {error}", script_path.display()))?;

    for required in [
        r#"expected_config_directory="/workspaces/.boxferry-gh""#,
        "[[ ! -t 0 ]]",
        "IFS= read -r -s -p 'GitHub token: '",
        "gh auth login --hostname \"${github_host}\" --with-token --insecure-storage",
        r#"auth_file="${GH_CONFIG_DIR}/hosts.yml""#,
        "chmod 0600 \"${auth_file}\"",
        "gh auth status --hostname",
        "The existing Git credential helpers were not changed.",
    ] {
        if !script.contains(required) {
            return Err(format!("GitHub CLI authentication script is missing `{required}`"));
        }
    }

    for forbidden in ["gh auth setup-git", "--show-token", "github_token=$1"] {
        if script.contains(forbidden) {
            return Err(format!(
                "GitHub CLI authentication script must not contain `{forbidden}`"
            ));
        }
    }

    Ok(())
}

#[test]
fn local_validation_runner_covers_deterministic_repository_checks() -> Result<(), String> {
    let script_path = repository_root().join("scripts/check-all.sh");
    let script = fs::read_to_string(&script_path)
        .map_err(|error| format!("failed to read {}: {error}", script_path.display()))?;

    for required in [
        "list_existing_files",
        "cargo fmt --all",
        "bash scripts/check-files.sh --fix",
        "bash scripts/validate-release-metadata.sh",
        "git --no-pager diff --check",
        "actionlint",
        "zizmor .github/workflows",
        "cargo ci-check",
        "cargo ci-core",
        "cargo ci-compose",
        "cargo ci-podman",
        "cargo ci-quadlet",
        "cargo ci-policy",
        "cargo ci-clippy",
        "cargo ci-test",
        "cargo ci-doctest",
        "RUSTDOCFLAGS=\"-D warnings\" cargo ci-doc",
        "cargo llvm-cov clean --locked",
        "cargo llvm-cov --locked --no-clean --workspace --all-features",
        "--fail-under-regions 82 --fail-under-functions 87",
        "cargo \"+${msrv}\" ci-check",
        "cargo \"+${msrv}\" ci-policy",
        "cargo deny --all-features check",
        "lychee --config lychee.toml --root-dir . --offline",
        "validation_storage_root",
        "coverage_target_dir",
        "semver_cargo_home",
        "semver_target_dir",
        "${CARGO_TARGET_DIR:-${repository_root}/target}/check-all/boxferry",
        "${validation_storage_root}/coverage",
        "${validation_storage_root}/cargo-home",
        "${validation_storage_root}/cargo-semver-checks-target",
        "env CARGO_TARGET_DIR=\"${coverage_target_dir}\"",
        "env CARGO_HOME=\"${semver_cargo_home}\"",
        "CARGO_TARGET_DIR=\"${semver_target_dir}\"",
        "BOXFERRY_SEMVER_RELEASE_TYPE",
        "\"\" | major | minor | patch",
        "semver_check=(cargo semver-checks check-release --workspace --all-features)",
        "semver_check+=(--release-type \"${semver_release_type}\")",
        "\"${semver_check[@]}\"",
    ] {
        if !script.contains(required) {
            return Err(format!("local validation runner missing `{required}`"));
        }
    }

    for opt_in in ["ci-real-world-compose"] {
        if script.contains(opt_in) {
            return Err(format!(
                "local validation runner must not invoke opt-in tier `{opt_in}`"
            ));
        }
    }

    if script.contains("lychee --config lychee.toml --root-dir . --cache") {
        return Err("local validation runner must not perform cached external link checks".to_owned());
    }

    Ok(())
}

#[test]
fn issue_to_pr_workflow_requires_primary_ownership_and_the_complete_local_gate() -> Result<(), String> {
    let root = repository_root();
    for (path, required) in [
        (
            "AGENTS.md",
            &[
                "## GitHub issue-to-PR workflow",
                "Run `./scripts/check-all.sh`",
                "hard gate against commit, push",
                "primary agent runs this workflow",
                "high reasoning effort",
                "Worker subagents",
                "never execute the Git or GitHub",
                "remains the primary agent's responsibility",
            ][..],
        ),
        (
            "docs/development-environment.md",
            &[
                "## Issue-to-PR contribution workflow",
                "./scripts/check-all.sh",
                "All steps must pass before the change is committed, pushed, or submitted",
                "primary agent uses high reasoning effort",
                "Worker agents",
                "never perform Git or GitHub writes",
                "the primary agent's final responsibility",
            ][..],
        ),
    ] {
        let contents =
            fs::read_to_string(root.join(path)).map_err(|error| format!("failed to read {path}: {error}"))?;
        for value in required {
            if !contents.contains(value) {
                return Err(format!("{path} is missing `{value}`"));
            }
        }
    }

    Ok(())
}

const REVIEWED_PRETTIER_EXCLUSIONS: &[&str] = &[
    "/CHANGELOG.md",
    "fixtures/**/expected-podman.json",
    "fixtures/**/expected-*-podman.json",
    "fixtures/differential/podman-lens-complex-corpus/*.cassette.json",
    "fixtures/scenarios/real-world-compose-*/input.compose.yaml",
    "fixtures/conformance/paperless-ngx-application/paperless-ngx-6.1.0-rootless.cassette.json",
    "fixtures/conformance/immich-application/immich-application-6.1.0-rootless.cassette.json",
];

#[test]
fn non_rust_file_runner_covers_owned_formats_without_recursive_workspace_globs() -> Result<(), String> {
    let root = repository_root();
    let script_path = root.join("scripts/check-files.sh");
    let script = fs::read_to_string(&script_path)
        .map_err(|error| format!("failed to read {}: {error}", script_path.display()))?;

    for required in [
        "git ls-files --cached --others --exclude-standard",
        "list_existing_files",
        "markdownlint-cli2 --fix",
        "prettier --write",
        "prettier --check",
        "check_yaml_document_markers",
        "fixtures/scenarios/real-world-compose-*/input.compose.yaml) continue ;;",
        "tombi format --check --offline",
        "tombi lint --error-on-warnings --offline",
        "shfmt -w",
        "shellcheck --",
        "hadolint",
    ] {
        if !script.contains(required) {
            return Err(format!("non-Rust file runner missing `{required}`"));
        }
    }
    if script.contains("**/*.md") {
        return Err("non-Rust file runner must not traverse sibling or generated Markdown trees".to_owned());
    }

    let tombi =
        fs::read_to_string(root.join("tombi.toml")).map_err(|error| format!("failed to read tombi.toml: {error}"))?;
    for required in [
        "dotted-keys-out-of-order = \"error\"",
        "key-empty = \"error\"",
        "tables-out-of-order = \"error\"",
        "docs/schemas/tombi-cargo-offline.schema.json",
        "include = [\"Cargo.toml\", \"**/Cargo.toml\"]",
        "fixtures/**/*.toml",
        "tools/**/*.toml",
    ] {
        if !tombi.contains(required) {
            return Err(format!("tombi.toml must contain `{required}`"));
        }
    }

    let cargo_schema = fs::read_to_string(root.join("docs/schemas/tombi-cargo-offline.schema.json"))
        .map_err(|error| format!("failed to read the offline Cargo schema: {error}"))?;
    for required in [r#""type": "object""#, r#""additionalProperties": true"#] {
        if !cargo_schema.contains(required) {
            return Err(format!("offline Cargo schema must contain `{required}`"));
        }
    }

    let prettier_ignore = fs::read_to_string(root.join(".prettierignore"))
        .map_err(|error| format!("failed to read .prettierignore: {error}"))?;
    if prettier_ignore
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .collect::<Vec<_>>()
        != REVIEWED_PRETTIER_EXCLUSIONS
    {
        return Err(
            "Prettier exclusions must remain limited to reviewed generated or immutable evidence inputs".to_owned(),
        );
    }
    let markdown_format = script
        .find(r#"prettier --write --ignore-path .prettierignore --ignore-unknown "${markdown_files[@]}""#)
        .ok_or("non-Rust file runner must format Markdown with Prettier")?;
    let markdown_fix = script
        .find(r#"markdownlint-cli2 --fix "${markdown_literals[@]}""#)
        .ok_or("non-Rust file runner must apply fixable Markdown lint rules")?;
    let markdown_lint = script
        .find(r#"markdownlint-cli2 "${markdown_literals[@]}""#)
        .ok_or("non-Rust file runner must lint Markdown after formatting")?;
    let markdown_check = script
        .find(r#"prettier --check --ignore-path .prettierignore --ignore-unknown "${markdown_files[@]}""#)
        .ok_or("non-Rust file runner must verify Markdown formatting")?;
    if !(markdown_format < markdown_fix && markdown_fix < markdown_lint && markdown_lint < markdown_check) {
        return Err("non-Rust file runner must format, fix, lint, then check Markdown in that order".to_owned());
    }

    let ci_path = root.join(".github/workflows/ci.yml");
    let ci = fs::read_to_string(&ci_path).map_err(|error| format!("failed to read {}: {error}", ci_path.display()))?;
    let release_path = root.join(".github/workflows/release.yml");
    let release = fs::read_to_string(&release_path)
        .map_err(|error| format!("failed to read {}: {error}", release_path.display()))?;
    for (path, workflow) in [(ci_path, ci), (release_path, release)] {
        for required in [
            "npm ci --ignore-scripts",
            "bash scripts/install-file-tools.sh /usr/local/bin",
            "bash scripts/check-files.sh --check",
        ] {
            if !workflow.contains(required) {
                return Err(format!(
                    "{} must enforce the non-Rust file contract `{required}`",
                    path.display()
                ));
            }
        }
    }

    let lock_path = root.join("package-lock.json");
    let lock =
        fs::read_to_string(&lock_path).map_err(|error| format!("failed to read {}: {error}", lock_path.display()))?;
    for package in ["markdownlint-cli2", "prettier"] {
        if !lock.contains(&format!("\"{package}\"")) {
            return Err(format!("{} must lock `{package}`", lock_path.display()));
        }
    }

    Ok(())
}

#[test]
fn complete_yaml_documents_use_explicit_start_markers() -> Result<(), String> {
    let root = repository_root();
    let output = Command::new("git")
        .args(["ls-files", "-z", "--", "*.yaml", "*.yml"])
        .current_dir(&root)
        .output()
        .map_err(|error| format!("failed to list YAML documents: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let deleted_output = Command::new("git")
        .args(["ls-files", "--deleted", "-z", "--", "*.yaml", "*.yml"])
        .current_dir(&root)
        .output()
        .map_err(|error| format!("failed to list deleted YAML documents: {error}"))?;
    if !deleted_output.status.success() {
        return Err(format!(
            "git ls-files --deleted failed: {}",
            String::from_utf8_lossy(&deleted_output.stderr)
        ));
    }
    let deleted = deleted_output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            std::str::from_utf8(path)
                .map(PathBuf::from)
                .map_err(|error| error.to_string())
        })
        .collect::<Result<BTreeSet<_>, _>>()?;

    for path in output.stdout.split(|byte| *byte == 0).filter(|path| !path.is_empty()) {
        let path = Path::new(std::str::from_utf8(path).map_err(|error| error.to_string())?);
        let absolute = root.join(path);
        if deleted.contains(path) {
            continue;
        }
        let immutable_upstream_compose = path.file_name().and_then(|name| name.to_str()) == Some("input.compose.yaml")
            && path
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .is_some_and(|directory| directory.starts_with("real-world-compose-"))
            && path.parent().and_then(Path::parent) == Some(Path::new("fixtures/scenarios"));
        if immutable_upstream_compose {
            continue;
        }
        let contents =
            fs::read_to_string(&absolute).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        if contents.lines().next() != Some("---") {
            return Err(format!("{} must start with `---`", path.display()));
        }
    }

    Ok(())
}

#[test]
fn immutable_upstream_whitespace_exceptions_are_narrow() -> Result<(), String> {
    let attributes = fs::read_to_string(repository_root().join(".gitattributes"))
        .map_err(|error| format!("failed to read .gitattributes: {error}"))?;
    let rules = attributes
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(
        rules,
        [
            "fixtures/scenarios/real-world-compose-*/UPSTREAM-LICENSE -whitespace",
            "fixtures/scenarios/real-world-compose-*/UPSTREAM-NOTICE -whitespace",
            "fixtures/scenarios/real-world-compose-*/input.compose.yaml -whitespace",
        ],
        "Git whitespace exceptions must remain limited to immutable upstream evidence"
    );
    Ok(())
}

#[test]
fn documentation_link_checks_separate_local_validity_from_external_health() -> Result<(), String> {
    let root = repository_root();
    let ci_path = root.join(".github/workflows/ci.yml");
    let ci = fs::read_to_string(&ci_path).map_err(|error| format!("failed to read {}: {error}", ci_path.display()))?;
    let external_path = root.join(".github/workflows/documentation-links.yml");
    let external = fs::read_to_string(&external_path)
        .map_err(|error| format!("failed to read {}: {error}", external_path.display()))?;
    let config_path = root.join("lychee.toml");
    let config = fs::read_to_string(&config_path)
        .map_err(|error| format!("failed to read {}: {error}", config_path.display()))?;

    for required in ["--config lychee.toml", "--offline"] {
        if !ci.contains(required) {
            return Err(format!("CI local-link check is missing `{required}`"));
        }
    }
    if ci.contains("--cache") {
        return Err("pull-request CI must not perform cached external link checks".to_owned());
    }

    for required in [
        "schedule:",
        "workflow_dispatch:",
        "path: .lycheecache",
        "--config lychee.toml",
        "--cache",
    ] {
        if !external.contains(required) {
            return Err(format!("external link-health workflow is missing `{required}`"));
        }
    }
    if external.contains("--offline") {
        return Err("external link-health workflow must not use offline mode".to_owned());
    }

    for required in [
        "max_cache_age = \"14d\"",
        "cache_exclude_status = \"400..=599\"",
        "host_concurrency = 2",
        "host_request_interval = \"500ms\"",
    ] {
        if !config.contains(required) {
            return Err(format!("Lychee policy is missing `{required}`"));
        }
    }

    Ok(())
}

#[test]
fn multi_root_workspace_uses_boxferry_as_the_container_owner() -> Result<(), String> {
    let root = repository_root();
    let workspace_path = root.join("boxferry-lenses.code-workspace");
    let workspace = fs::read_to_string(&workspace_path)
        .map_err(|error| format!("failed to read {}: {error}", workspace_path.display()))?;
    let devcontainer_path = root.join(".devcontainer/devcontainer.json");
    let devcontainer = fs::read_to_string(&devcontainer_path)
        .map_err(|error| format!("failed to read {}: {error}", devcontainer_path.display()))?;

    for required in [
        "\"name\": \"BoxFerry\"",
        "\"path\": \".\"",
        "\"name\": \"ComposeLens\"",
        "\"path\": \".boxferry-workspace/compose-lens\"",
        "\"name\": \"PodmanLens\"",
        "\"path\": \".boxferry-workspace/podman-lens\"",
        "\"name\": \"QuadletLens\"",
        "\"path\": \".boxferry-workspace/quadlet-lens\"",
        "\"label\": \"Workspace: Format, lint, and test all repositories\"",
        "\"dependsOrder\": \"sequence\"",
        "\"label\": \"Workspace: Check BoxFerry\"",
        "\"label\": \"Workspace: Check ComposeLens\"",
        "\"label\": \"Workspace: Check QuadletLens\"",
        "${workspaceFolder:BoxFerry}/scripts/check-all.sh",
        "${workspaceFolder:ComposeLens}/scripts/check-all.sh",
        "${workspaceFolder:QuadletLens}/scripts/check-all.sh",
    ] {
        if !workspace.contains(required) {
            return Err(format!("multi-root workspace is missing `{required}`"));
        }
    }
    for required in [
        "\"workspaceMount\": \"source=${localWorkspaceFolder},target=/workspaces/boxferry,type=bind\"",
        "\"workspaceFolder\": \"/workspaces/boxferry\"",
        "\"CARGO_HOME\": \"/workspaces/.boxferry-cargo\"",
        "\"GH_CONFIG_DIR\": \"/workspaces/.boxferry-gh\"",
        "source=boxferry-cargo-${devcontainerId},target=/workspaces/.boxferry-cargo,type=volume",
        "source=boxferry-gh-${devcontainerId},target=/workspaces/.boxferry-gh,type=volume",
        "source=${localWorkspaceFolder}/../compose-lens,target=/workspaces/boxferry/.boxferry-workspace/compose-lens,type=bind",
        "source=${localWorkspaceFolder}/../podman-lens,target=/workspaces/boxferry/.boxferry-workspace/podman-lens,type=bind",
        "source=${localWorkspaceFolder}/../quadlet-lens,target=/workspaces/boxferry/.boxferry-workspace/quadlet-lens,type=bind",
    ] {
        if !devcontainer.contains(required) {
            return Err(format!("BoxFerry Dev Container is missing sibling mount `{required}`"));
        }
    }

    for (label, text, ordered) in [
        (
            "workspace folders",
            workspace.as_str(),
            [
                "\"path\": \".boxferry-workspace/compose-lens\"",
                "\"path\": \".boxferry-workspace/podman-lens\"",
                "\"path\": \".boxferry-workspace/quadlet-lens\"",
            ],
        ),
        (
            "Dev Container sibling mounts",
            devcontainer.as_str(),
            [
                "../compose-lens,target=/workspaces/boxferry/.boxferry-workspace/compose-lens",
                "../podman-lens,target=/workspaces/boxferry/.boxferry-workspace/podman-lens",
                "../quadlet-lens,target=/workspaces/boxferry/.boxferry-workspace/quadlet-lens",
            ],
        ),
    ] {
        let positions = ordered
            .map(|entry| text.find(entry).ok_or_else(|| format!("{label} is missing `{entry}`")))
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
        if !positions.windows(2).all(|pair| pair[0] < pair[1]) {
            return Err(format!("{label} must order ComposeLens, PodmanLens, and QuadletLens"));
        }
    }

    Ok(())
}

#[test]
fn workspace_toolchain_includes_all_devcontainer_rust_components() -> Result<(), String> {
    let path = repository_root().join("rust-toolchain.toml");
    let text = fs::read_to_string(&path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let config =
        toml::from_str::<toml::Value>(&text).map_err(|error| format!("invalid workspace toolchain: {error}"))?;
    let components = config
        .get("toolchain")
        .and_then(|toolchain| toolchain.get("components"))
        .and_then(toml::Value::as_array)
        .ok_or("workspace toolchain must declare its required components")?;
    for required in ["clippy", "llvm-tools-preview", "rustfmt"] {
        if !components.iter().any(|component| component.as_str() == Some(required)) {
            return Err(format!(
                "workspace toolchain must request `{required}`; the image default toolchain's components do not apply to workspace toolchain updates"
            ));
        }
    }
    Ok(())
}

#[test]
fn devcontainer_preinstalls_the_workspace_toolchain_from_its_canonical_file() -> Result<(), String> {
    let path = repository_root().join(".devcontainer/Dockerfile");
    let dockerfile =
        fs::read_to_string(&path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    for required in [
        "COPY rust-toolchain.toml /opt/boxferry-toolchain/rust-toolchain.toml",
        "WORKDIR /opt/boxferry-toolchain\nRUN rustup show active-toolchain\nWORKDIR /",
    ] {
        if !dockerfile.contains(required) {
            return Err(format!(
                "Dev Container must preinstall the workspace toolchain: `{required}`"
            ));
        }
    }
    Ok(())
}

#[test]
fn devcontainer_lifecycle_check_uses_the_remote_user_without_sudo_user_switching() -> Result<(), String> {
    let script_path = repository_root().join(".devcontainer/verify-tools.sh");
    let script = fs::read_to_string(&script_path)
        .map_err(|error| format!("failed to read {}: {error}", script_path.display()))?;

    for required in [
        "CARGO_HOME",
        "GH_CONFIG_DIR",
        "[[ ! -w \"${persistent_directory}\" ]]",
        "sudo chown -R",
        "chmod 0700 \"${GH_CONFIG_DIR}\"",
        "installed_components=\"$(rustup component list --installed)\"",
        "$(rustup show active-toolchain)",
        "From the BoxFerry repository root, run: rustup component add llvm-tools-preview",
        "for repository in compose-lens podman-lens quadlet-lens boxferry-website; do",
    ] {
        if !script.contains(required) {
            return Err(format!("Dev Container lifecycle check is missing `{required}`"));
        }
    }
    for forbidden in ["CARGO_TARGET_DIR", "sudo -u"] {
        if script.contains(forbidden) {
            return Err(format!("Dev Container lifecycle check must not contain `{forbidden}`"));
        }
    }

    Ok(())
}

#[test]
fn devcontainer_uses_cargo_default_workspace_target_directory() -> Result<(), String> {
    let root = repository_root();
    let config_path = root.join(".devcontainer/devcontainer.json");
    let config = fs::read_to_string(&config_path)
        .map_err(|error| format!("failed to read {}: {error}", config_path.display()))?;

    for forbidden in ["CARGO_TARGET_DIR", "boxferry-target", "/workspaces/.boxferry-target"] {
        if config.contains(forbidden) {
            return Err(format!(
                "Dev Container must use Cargo's default workspace target directory; found `{forbidden}`"
            ));
        }
    }

    let guide_path = root.join("docs/development-environment.md");
    let guide =
        fs::read_to_string(&guide_path).map_err(|error| format!("failed to read {}: {error}", guide_path.display()))?;
    for required in [
        "Cargo uses its default workspace target directory",
        "cargo build --release --locked --package boxferry",
        "./target/release/boxferry --version",
        "Dev Containers: Rebuild Container",
        "unset CARGO_TARGET_DIR",
    ] {
        if !guide.contains(required) {
            return Err(format!("development environment guide is missing `{required}`"));
        }
    }

    Ok(())
}

#[test]
fn ci_workflow_enforces_coverage_portability_and_pr_gate_contract() -> Result<(), String> {
    let dockerfile = fs::read_to_string(repository_root().join(".devcontainer/Dockerfile"))
        .map_err(|error| format!("failed to read Dev Container Dockerfile: {error}"))?;
    let expected_version = pinned_cargo_llvm_cov_version(&dockerfile, ".devcontainer/Dockerfile")?;

    let workflow_path = repository_root().join(".github/workflows/ci.yml");
    let workflow = fs::read_to_string(&workflow_path)
        .map_err(|error| format!("failed to read {}: {error}", workflow_path.display()))?;

    let workflow_version = pinned_cargo_llvm_cov_version(&workflow, ".github/workflows/ci.yml")?;
    if workflow_version != expected_version {
        return Err(format!(
            "CI pins cargo-llvm-cov {workflow_version}, but the Dev Container pins {expected_version}"
        ));
    }

    for required in [
        "  coverage:\n    name: Coverage ratchet",
        "rustup component add llvm-tools-preview",
        "cargo llvm-cov clean --locked",
        "cargo llvm-cov --locked --no-clean --workspace --all-features --all-targets --summary-only\n          --fail-under-regions 82 --fail-under-functions 87 --fail-under-lines 82",
        "  portability:\n    name: Portability (macOS)",
        "runs-on: macos-14",
        "run: cargo ci-check",
        "run: cargo ci-test",
        "  release-metadata:\n    name: Release metadata and changelog",
        "run: bash scripts/validate-release-metadata.sh",
        "  pr-gate:\n    name: PR gate\n    if: always()",
        "needs:\n      [\n        rust,\n        msrv,\n        dependencies,\n        documentation,\n        release-metadata,\n        semver-release-type,\n        semver,\n        coverage,\n        portability,\n        migration-readiness,\n      ]",
    ] {
        if !workflow.contains(required) {
            return Err(format!("CI workflow is missing contract `{required}`"));
        }
    }

    for job in [
        ("Rust quality", "RUST_RESULT", "rust"),
        ("MSRV", "MSRV_RESULT", "msrv"),
        ("Dependency and license policy", "DEPENDENCIES_RESULT", "dependencies"),
        ("Documentation", "DOCUMENTATION_RESULT", "documentation"),
        (
            "Release metadata and changelog",
            "RELEASE_METADATA_RESULT",
            "release-metadata",
        ),
        (
            "SemVer release type",
            "SEMVER_RELEASE_TYPE_RESULT",
            "semver-release-type",
        ),
        ("SemVer", "SEMVER_RESULT", "semver"),
        ("Coverage ratchet", "COVERAGE_RESULT", "coverage"),
        ("macOS portability", "PORTABILITY_RESULT", "portability"),
        (
            "Offline migration readiness",
            "MIGRATION_READINESS_RESULT",
            "migration-readiness",
        ),
    ] {
        let (job_name, result_variable, needs_job) = job;
        let required = format!("{result_variable}: ${{{{ needs.{needs_job}.result }}}}");
        if !workflow.contains(&required) {
            return Err(format!("PR gate does not expose a result variable for `{job_name}`"));
        }
    }

    for required in [
        "printf '| Job | Result |\\n'",
        "printf \"| %s | \\`%s\\` |\\n\" \"${name}\" \"${result}\" >> \"${GITHUB_STEP_SUMMARY}\"",
        "::error title=Required PR job did not succeed::${name} concluded ${result}.",
        "Required PR job did not succeed: ${name} concluded ${result}.",
        "if (( failures != 0 )); then",
        "One or more required PR jobs did not succeed; see the result table and annotations above.",
    ] {
        if !workflow.contains(required) {
            return Err(format!("PR gate is missing actionable failure diagnostic `{required}`"));
        }
    }
    if workflow.contains("test \"${{ needs.") {
        return Err("PR gate must not use opaque success test predicates".to_owned());
    }
    if workflow.contains("windows-") {
        return Err("CI must not claim unsupported native Windows portability".to_owned());
    }

    Ok(())
}

#[test]
fn release_workflow_rechecks_coverage_and_msrv_contracts() -> Result<(), String> {
    let dockerfile = fs::read_to_string(repository_root().join(".devcontainer/Dockerfile"))
        .map_err(|error| format!("failed to read Dev Container Dockerfile: {error}"))?;
    let expected_version = pinned_cargo_llvm_cov_version(&dockerfile, ".devcontainer/Dockerfile")?;

    let workflow_path = repository_root().join(".github/workflows/release.yml");
    let workflow = fs::read_to_string(&workflow_path)
        .map_err(|error| format!("failed to read {}: {error}", workflow_path.display()))?;

    let workflow_version = pinned_cargo_llvm_cov_version(&workflow, ".github/workflows/release.yml")?;
    if workflow_version != expected_version {
        return Err(format!(
            "release workflow pins cargo-llvm-cov {workflow_version}, but the Dev Container pins {expected_version}"
        ));
    }

    for required in [
        "bash scripts/validate-release-metadata.sh",
        "rustup component add llvm-tools-preview",
        "cargo llvm-cov clean --locked",
        "cargo llvm-cov --locked --no-clean --workspace --all-features --all-targets --summary-only\n          --fail-under-regions 82 --fail-under-functions 87 --fail-under-lines 82",
        "- name: Read the workspace MSRV",
        "rustup toolchain install \"${RUST_MSRV}\" --profile minimal",
        "cargo \"+${RUST_MSRV}\" ci-check",
        "cargo \"+${RUST_MSRV}\" ci-policy",
    ] {
        if !workflow.contains(required) {
            return Err(format!("release workflow is missing validation guard `{required}`"));
        }
    }

    Ok(())
}

fn pinned_cargo_llvm_cov_version(document: &str, source: &str) -> Result<String, String> {
    const WORKFLOW_PREFIX: &str = "run: cargo install --locked --version ";
    const WORKFLOW_SUFFIX: &str = " cargo-llvm-cov";
    const DEVCONTAINER_PREFIX: &str = "ARG CARGO_LLVM_COV_VERSION=";

    let versions = document
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            line.strip_prefix(WORKFLOW_PREFIX)
                .and_then(|value| value.strip_suffix(WORKFLOW_SUFFIX))
                .or_else(|| line.strip_prefix(DEVCONTAINER_PREFIX))
        })
        .collect::<Vec<_>>();

    if versions.len() != 1 {
        return Err(format!(
            "{source} must contain exactly one cargo-llvm-cov version pin, found {}",
            versions.len()
        ));
    }

    let version = versions[0];
    let components = version.split('.').collect::<Vec<_>>();
    if components.len() != 3
        || components
            .iter()
            .any(|component| component.is_empty() || !component.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(format!(
            "{source} must pin cargo-llvm-cov to an exact major.minor.patch version, found `{version}`"
        ));
    }

    Ok(version.to_owned())
}

#[test]
fn release_metadata_and_changelog_validation_is_shared() -> Result<(), String> {
    let root = repository_root();
    let script_path = root.join("scripts/validate-release-metadata.sh");
    let script = fs::read_to_string(&script_path)
        .map_err(|error| format!("failed to read {}: {error}", script_path.display()))?;

    for required in [
        "cargo metadata --locked --no-deps --format-version 1",
        "workspace packages must use one version",
        "select(.publish == [])",
        "bash scripts/extract-release-notes.sh",
        "exactly one Unreleased section",
        "Newest CHANGELOG.md release",
        "Unreleased must be empty while preparing BoxFerry",
        "Release metadata and changelog are valid",
    ] {
        if !script.contains(required) {
            return Err(format!("shared release validator is missing {required}"));
        }
    }
    for package in PUBLISHED_PACKAGES {
        if !script.contains(package) {
            return Err(format!("shared release validator is missing package {package}"));
        }
    }

    for (path, required) in [
        (
            "scripts/check-all.sh",
            "run_step \"Test release metadata policy\" bash scripts/test-release-metadata.sh",
        ),
        (
            ".github/workflows/ci.yml",
            "  release-metadata:\n    name: Release metadata and changelog\n    runs-on: ubuntu-24.04\n    timeout-minutes: 5\n    steps:\n      - name: Check out repository with release history",
        ),
        (
            ".github/workflows/release.yml",
            "bash scripts/validate-release-metadata.sh",
        ),
    ] {
        let text = fs::read_to_string(root.join(path)).map_err(|error| format!("failed to read {path}: {error}"))?;
        if text.matches("bash scripts/validate-release-metadata.sh").count() != 1 || !text.contains(required) {
            return Err(format!("{path} must invoke the shared release validator exactly once"));
        }
    }

    let ci = fs::read_to_string(root.join(".github/workflows/ci.yml"))
        .map_err(|error| format!("failed to read CI workflow: {error}"))?;
    let release_metadata_job = ci
        .split_once("  release-metadata:\n")
        .and_then(|(_, remainder)| remainder.split_once("\n  semver-release-type:"))
        .map(|(job, _)| job)
        .ok_or_else(|| "CI release-metadata job boundary is missing".to_owned())?;
    if !release_metadata_job.contains("fetch-depth: 0") {
        return Err("CI release-metadata job must fetch complete tag history".to_owned());
    }

    let release = fs::read_to_string(root.join(".github/workflows/release.yml"))
        .map_err(|error| format!("failed to read release workflow: {error}"))?;
    if release.contains("metadata=\"$(cargo metadata --locked") {
        return Err("release workflow must not duplicate shared release metadata validation".to_owned());
    }

    let tests = fs::read_to_string(root.join("scripts/test-release-metadata.sh"))
        .map_err(|error| format!("failed to read release metadata policy tests: {error}"))?;
    for required in [
        "non-empty Unreleased during release preparation",
        "newest release differs from workspace version",
        "duplicate current-version release section",
        "duplicate Unreleased section",
    ] {
        if !tests.contains(required) {
            return Err(format!("release metadata policy tests are missing {required}"));
        }
    }

    for (path, required) in [
        (
            "docs/releasing.md",
            "one dated, usable numbered section matching the workspace version",
        ),
        ("docs/testing.md", "dedicated job required by the aggregate gate"),
    ] {
        let contents =
            fs::read_to_string(root.join(path)).map_err(|error| format!("failed to read {path}: {error}"))?;
        if !contents.contains(required) {
            return Err(format!("{path} is missing shared release-validation documentation"));
        }
    }

    Ok(())
}

#[test]
fn platform_support_contract_requires_wsl_for_windows_cli() -> Result<(), String> {
    let root = repository_root();
    let cli = fs::read_to_string(root.join("crates/boxferry/src/main.rs"))
        .map_err(|error| format!("failed to read CLI source: {error}"))?;
    let platform_support = fs::read_to_string(root.join("docs/platform-support.md"))
        .map_err(|error| format!("failed to read platform support: {error}"))?;

    for required in [
        "#[cfg(target_os = \"windows\")]",
        "the native Windows BoxFerry CLI is unsupported; install and run BoxFerry inside WSL2",
    ] {
        if !cli.contains(required) {
            return Err(format!("CLI is missing native-Windows guard `{required}`"));
        }
    }
    for required in [
        "The BoxFerry CLI is supported on Linux.",
        "Windows users must install and run the Linux CLI inside",
        "Such compilation is incidental unless that platform appears in the supported CI",
    ] {
        if !platform_support.contains(required) {
            return Err(format!("platform documentation is missing contract `{required}`"));
        }
    }

    Ok(())
}

#[test]
fn repository_supply_chain_has_single_sources_and_immutable_pins() -> Result<(), String> {
    support::validate_repository_supply_chain(&repository_root())
}

#[test]
fn release_packages_are_publishable_and_lockstep() -> Result<(), String> {
    let root = repository_root();
    let workspace_text = fs::read_to_string(root.join("Cargo.toml"))
        .map_err(|error| format!("failed to read workspace manifest: {error}"))?;
    let workspace = toml::from_str::<toml::Value>(&workspace_text)
        .map_err(|error| format!("failed to parse workspace manifest: {error}"))?;
    let version = workspace["workspace"]["package"]["version"]
        .as_str()
        .ok_or_else(|| "workspace version must be a string".to_owned())?;

    for package in PUBLISHED_PACKAGES {
        let crate_root = root.join("crates").join(package);
        let manifest_path = crate_root.join("Cargo.toml");
        let manifest_text = fs::read_to_string(&manifest_path)
            .map_err(|error| format!("failed to read {}: {error}", manifest_path.display()))?;
        let manifest = toml::from_str::<toml::Value>(&manifest_text)
            .map_err(|error| format!("failed to parse {}: {error}", manifest_path.display()))?;

        if manifest["package"]["publish"].as_bool() != Some(true) {
            return Err(format!("{package} must be explicitly publishable"));
        }
        if manifest["package"]["version"]["workspace"].as_bool() != Some(true) {
            return Err(format!("{package} must inherit the lockstep workspace version"));
        }
        let readme = manifest["package"]["readme"]
            .as_str()
            .ok_or_else(|| format!("{package} must declare a package README"))?;
        if !crate_root.join(readme).is_file() {
            return Err(format!("{package} package README does not exist"));
        }
        if *package == "boxferry"
            && manifest["package"]["metadata"]["docs"]["rs"]["all-features"].as_bool() != Some(true)
        {
            return Err("boxferry docs.rs builds must include every public feature".to_owned());
        }

        if let Some(dependencies) = manifest.get("dependencies").and_then(toml::Value::as_table) {
            for (dependency, requirement) in dependencies {
                if dependency.starts_with("boxferry-") && requirement["version"].as_str() != Some(version) {
                    return Err(format!(
                        "{package} dependency {dependency} must require workspace version {version}"
                    ));
                }
            }
        }
    }

    Ok(())
}

#[test]
fn release_plz_preparation_runs_only_for_reviewed_release_paths() -> Result<(), String> {
    let root = repository_root();
    let workflow_path = root.join(".github/workflows/release-plz.yml");
    let workflow = fs::read_to_string(&workflow_path)
        .map_err(|error| format!("failed to read {}: {error}", workflow_path.display()))?;
    let push_start = workflow
        .find("  push:\n")
        .ok_or_else(|| "release-plz workflow must have a push trigger".to_owned())?;
    let dispatch_start = workflow
        .find("  workflow_dispatch:\n")
        .ok_or_else(|| "release-plz workflow must retain manual dispatch".to_owned())?;
    let push = &workflow[push_start..dispatch_start];

    for required in [
        "    paths:\n",
        r#"      - ".cargo/**""#,
        r#"      - ".github/scripts/publish-crate.sh""#,
        r#"      - ".github/workflows/release-plz.yml""#,
        r#"      - ".github/workflows/release.yml""#,
        r#"      - "Cargo.lock""#,
        r#"      - "Cargo.toml""#,
        r#"      - "LICENSE""#,
        r#"      - "crates/**/Cargo.toml""#,
        r#"      - "crates/**/*.rs""#,
        r#"      - "release-plz.toml""#,
        r#"      - "rust-toolchain.toml""#,
    ] {
        if !push.contains(required) {
            return Err(format!("release-plz push trigger is missing `{required}`"));
        }
    }
    for forbidden in [r#"      - "docs/**""#, r#"      - "**/*.md""#, r#"      - "crates/**""#] {
        if push.contains(forbidden) {
            return Err(format!(
                "release-plz push trigger must not include documentation-only path `{forbidden}`"
            ));
        }
    }

    Ok(())
}

#[test]
fn public_api_compatibility_runs_in_ci_and_release() -> Result<(), String> {
    const ACTION: &str = "obi1kenobi/cargo-semver-checks-action@6b69fcf40e9b5fb17adeb57e4b6ecd020649a239 # v2.9";

    for workflow_name in ["ci.yml", "release.yml"] {
        let workflow_path = repository_root().join(".github/workflows").join(workflow_name);
        let workflow = fs::read_to_string(&workflow_path)
            .map_err(|error| format!("failed to read {}: {error}", workflow_path.display()))?;

        if workflow.matches(ACTION).count() != 1
            || !workflow.contains("package: ${{ matrix.package }}\n          feature-group: all-features")
        {
            return Err(format!(
                "{workflow_name} must run the pinned SemVer check for the package matrix"
            ));
        }
        for package in PUBLISHED_PACKAGES {
            if !workflow.contains(&format!("          - {package}")) {
                return Err(format!("{workflow_name} SemVer matrix is missing {package}"));
            }
        }
    }

    Ok(())
}

#[test]
fn intentional_public_break_semver_path_is_explicit_and_narrow() -> Result<(), String> {
    let root = repository_root();
    let local_script = fs::read_to_string(root.join("scripts/check-all.sh"))
        .map_err(|error| format!("failed to read local validation runner: {error}"))?;
    for required in [
        "semver_release_type=\"${BOXFERRY_SEMVER_RELEASE_TYPE:-}\"",
        "case \"${semver_release_type}\" in\n  \"\" | major | minor | patch) ;;",
        "BOXFERRY_SEMVER_RELEASE_TYPE must be empty, major, minor, or patch",
        "semver_check+=(--release-type \"${semver_release_type}\")",
    ] {
        if !local_script.contains(required) {
            return Err(format!("local SemVer override contract is missing {required}"));
        }
    }

    let ci_path = root.join(".github/workflows/ci.yml");
    let ci = fs::read_to_string(&ci_path).map_err(|error| format!("failed to read {}: {error}", ci_path.display()))?;
    for required in [
        "semver-release-type:",
        "release_type: ${{ steps.classify.outputs.release_type }}",
        "SEMVER_CHANGE_SUBJECT: ${{ github.event.pull_request.title }}",
        "push | workflow_dispatch) subject=\"$(git log -1 --format=%s)\" ;;",
        "subject=\"${subject%%$'\\n'*}\"",
        "'^(feat|fix|perf|refactor|revert)(\\([^)]+\\))?!: .+$'",
        "printf 'release_type=major\\n' >> \"${GITHUB_OUTPUT}\"",
        "printf 'release_type=\\n' >> \"${GITHUB_OUTPUT}\"",
        "needs: semver-release-type",
        "release-type: ${{ needs.semver-release-type.outputs.release_type }}",
    ] {
        if !ci.contains(required) {
            return Err(format!("CI SemVer break classifier is missing {required}"));
        }
    }
    for forbidden in [
        "contains(github.event.pull_request.title, '!')",
        "contains(\"${subject}\", '!')",
    ] {
        if ci.contains(forbidden) {
            return Err(format!("CI SemVer break classifier must not use {forbidden}"));
        }
    }

    let release_plz = fs::read_to_string(root.join("release-plz.toml"))
        .map_err(|error| format!("failed to read release-plz.toml: {error}"))?;
    if !release_plz.contains("semver_check = true") {
        return Err("release-plz must retain semver_check = true".to_owned());
    }

    Ok(())
}

#[test]
fn published_format_adapters_do_not_depend_on_sibling_adapters() -> Result<(), String> {
    let root = repository_root();

    for adapter in FORMAT_ADAPTER_PACKAGES {
        let manifest_path = root.join("crates").join(adapter).join("Cargo.toml");
        let manifest_text = fs::read_to_string(&manifest_path)
            .map_err(|error| format!("failed to read {}: {error}", manifest_path.display()))?;
        let manifest = toml::from_str::<toml::Value>(&manifest_text)
            .map_err(|error| format!("failed to parse {}: {error}", manifest_path.display()))?;
        let mut dependency_tables = Vec::new();

        for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
            if let Some(dependencies) = manifest.get(section).and_then(toml::Value::as_table) {
                dependency_tables.push((section.to_owned(), dependencies));
            }
        }
        if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
            for (target, configuration) in targets {
                for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
                    if let Some(dependencies) = configuration.get(section).and_then(toml::Value::as_table) {
                        dependency_tables.push((format!("target.{target}.{section}"), dependencies));
                    }
                }
            }
        }

        for (section, dependencies) in dependency_tables {
            for (dependency_alias, specification) in dependencies {
                let dependency = specification
                    .get("package")
                    .and_then(toml::Value::as_str)
                    .unwrap_or(dependency_alias);
                if FORMAT_ADAPTER_PACKAGES.contains(&dependency) {
                    return Err(format!(
                        "published adapter {adapter} must not have sibling adapter {dependency} in {section}"
                    ));
                }
            }
        }
    }

    Ok(())
}

#[test]
fn release_package_order_follows_internal_manifest_dependencies() -> Result<(), String> {
    let root = repository_root();
    let positions = PUBLISHED_PACKAGES
        .iter()
        .enumerate()
        .map(|(position, package)| (*package, position))
        .collect::<BTreeMap<_, _>>();

    for dependent in PUBLISHED_PACKAGES {
        let manifest_path = root.join("crates").join(dependent).join("Cargo.toml");
        let manifest_text = fs::read_to_string(&manifest_path)
            .map_err(|error| format!("failed to read {}: {error}", manifest_path.display()))?;
        let manifest = toml::from_str::<toml::Value>(&manifest_text)
            .map_err(|error| format!("failed to parse {}: {error}", manifest_path.display()))?;
        let dependent_position = positions[dependent];
        let mut dependency_tables = Vec::new();

        for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
            if let Some(dependencies) = manifest.get(section).and_then(toml::Value::as_table) {
                dependency_tables.push((section.to_owned(), dependencies));
            }
        }
        if let Some(targets) = manifest.get("target").and_then(toml::Value::as_table) {
            for (target, configuration) in targets {
                for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
                    if let Some(dependencies) = configuration.get(section).and_then(toml::Value::as_table) {
                        dependency_tables.push((format!("target.{target}.{section}"), dependencies));
                    }
                }
            }
        }

        for (section, dependencies) in dependency_tables {
            for (dependency_alias, specification) in dependencies {
                let dependency = specification
                    .get("package")
                    .and_then(toml::Value::as_str)
                    .unwrap_or(dependency_alias);
                let Some(dependency_position) = positions.get(dependency) else {
                    continue;
                };
                if *dependency_position >= dependent_position {
                    return Err(format!(
                        "{dependent} {section} dependency {dependency} must precede its dependent in PUBLISHED_PACKAGES"
                    ));
                }
            }
        }
    }

    Ok(())
}

#[test]
fn release_workflow_uses_ordered_trusted_publishing() -> Result<(), String> {
    let root = repository_root();
    let workflow_path = root.join(".github/workflows/release.yml");
    let workflow = fs::read_to_string(&workflow_path)
        .map_err(|error| format!("failed to read {}: {error}", workflow_path.display()))?;
    let script_path = root.join(".github/scripts/publish-crate.sh");
    let script = fs::read_to_string(&script_path)
        .map_err(|error| format!("failed to read {}: {error}", script_path.display()))?;

    for forbidden in ["CRATES_IO_API_TOKEN", "cargo login", "--token"] {
        if workflow.contains(forbidden) || script.contains(forbidden) {
            return Err(format!(
                "release automation contains forbidden credential path `{forbidden}`"
            ));
        }
    }

    let bootstrap_references = workflow.matches(CRATES_IO_BOOTSTRAP_SECRET).count();
    let secret_references = workflow.matches("secrets.").count();
    if bootstrap_references != PUBLISHED_PACKAGES.len() + 1
        || secret_references != bootstrap_references
        || script.contains("CRATES_IO_BOOTSTRAP_TOKEN")
        || script.contains("secrets.")
    {
        return Err("release bootstrap must use only the exact scoped GitHub environment secret".to_owned());
    }
    if workflow
        .matches("if: steps.auth_mode.outputs.bootstrap != 'true'")
        .count()
        != PUBLISHED_PACKAGES.len()
        || workflow.matches("|| secrets.CRATES_IO_BOOTSTRAP_TOKEN").count() != PUBLISHED_PACKAGES.len()
        || !workflow.contains("if [[ \"${VERSION}\" != \"0.1.1\" ]]")
    {
        return Err("release bootstrap must be limited to 0.1.1 and preserve trusted publishing".to_owned());
    }

    if workflow.matches(CRATES_IO_AUTH_ACTION).count() != PUBLISHED_PACKAGES.len() {
        return Err("every published crate must declare a fresh trusted-publishing token path".to_owned());
    }
    if script.matches("cargo publish --locked --package").count() != 1 {
        return Err("the publication helper must contain one locked package command".to_owned());
    }

    let mut previous = 0;
    for package in PUBLISHED_PACKAGES {
        let command = format!("bash .github/scripts/publish-crate.sh {package} \"${{VERSION}}\"");
        let position = workflow
            .find(&command)
            .ok_or_else(|| format!("release workflow is missing {package}"))?;
        if position < previous {
            return Err(format!("release workflow publishes {package} out of dependency order"));
        }
        previous = position;
    }

    for required in [
        "environment: release",
        "id-token: write",
        "target/release-artifacts/SHA256SUMS",
        "uses: actions/attest@",
        "Create or verify annotated release tag",
        "Publish immutable GitHub release",
        "Could not read the numeric ID of the existing draft release",
        "GitHub did not return a numeric release ID after creating draft",
        "GitHub did not return a valid release asset upload URL",
        "Release asset is missing:",
        "GitHub did not confirm upload of release asset",
        "GitHub did not confirm publication of release ID",
    ] {
        if !workflow.contains(required) {
            return Err(format!("release workflow is missing guard `{required}`"));
        }
    }

    Ok(())
}

#[test]
fn release_plz_prepares_only_guarded_lockstep_releases() -> Result<(), String> {
    let root = repository_root();
    if root.join("docs/releases").exists() {
        return Err("CHANGELOG.md must remain the only release-history source".to_owned());
    }
    let config_text = fs::read_to_string(root.join("release-plz.toml"))
        .map_err(|error| format!("failed to read release-plz.toml: {error}"))?;
    let config = toml::from_str::<toml::Value>(&config_text)
        .map_err(|error| format!("failed to parse release-plz.toml: {error}"))?;
    let workspace = config["workspace"]
        .as_table()
        .ok_or_else(|| "release-plz.toml must contain [workspace]".to_owned())?;

    for (name, expected) in [
        ("allow_dirty", false),
        ("changelog_update", false),
        ("dependencies_update", false),
        ("git_release_enable", false),
        ("git_tag_enable", false),
        ("publish", false),
        ("release_always", false),
        ("semver_check", true),
    ] {
        if workspace.get(name).and_then(toml::Value::as_bool) != Some(expected) {
            return Err(format!("release-plz workspace setting {name} must be {expected}"));
        }
    }
    if workspace.get("pr_branch_prefix").and_then(toml::Value::as_str) != Some("release-plz-") {
        return Err("release-plz branches must use the guarded release-plz- prefix".to_owned());
    }
    if workspace.contains_key("release_commits") {
        return Err("release-plz must not filter commits before version-group bookkeeping".to_owned());
    }

    validate_release_plz_changelog(&config)?;

    let packages = config["package"]
        .as_array()
        .ok_or_else(|| "release-plz.toml must configure every published package".to_owned())?;
    if packages.len() != PUBLISHED_PACKAGES.len() {
        return Err("release-plz.toml must configure all six lockstep packages".to_owned());
    }
    for package in PUBLISHED_PACKAGES {
        let configured = packages.iter().find(|entry| entry["name"].as_str() == Some(package));
        let configured = configured.ok_or_else(|| format!("release-plz.toml is missing {package}"))?;
        if configured["version_group"].as_str() != Some("boxferry") {
            return Err(format!("{package} must belong to the boxferry version group"));
        }
    }
    let facade = packages
        .iter()
        .find(|entry| entry["name"].as_str() == Some("boxferry"))
        .ok_or_else(|| "release-plz.toml is missing the facade package".to_owned())?;
    if facade["changelog_update"].as_bool() != Some(true)
        || facade["changelog_path"].as_str() != Some("CHANGELOG.md")
        || facade["changelog_include"].as_array().map(Vec::len) != Some(5)
    {
        return Err("the facade must aggregate all component changes into the root changelog".to_owned());
    }

    validate_release_plz_workflow(&root, "Strukturpiloten/boxferry")?;

    let release = fs::read_to_string(root.join(".github/workflows/release.yml"))
        .map_err(|error| format!("failed to read release workflow: {error}"))?;
    if release.contains("docs/releases/${version}.md") || !release.contains("bash scripts/extract-release-notes.sh") {
        return Err("protected publication must derive release notes from CHANGELOG.md".to_owned());
    }

    Ok(())
}

fn validate_release_plz_changelog(config: &toml::Value) -> Result<(), String> {
    let changelog = config["changelog"]
        .as_table()
        .ok_or_else(|| "release-plz.toml must contain [changelog]".to_owned())?;
    if changelog.get("protect_breaking_commits").and_then(toml::Value::as_bool) != Some(true) {
        return Err("release-plz must preserve breaking commits in generated changelogs".to_owned());
    }

    let body = changelog
        .get("body")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| "release-plz must configure a changelog body".to_owned())?;
    if !body.contains("## [{{ version }}]")
        || !body.contains("{{ release_link }}")
        || !body.contains("%Y-%m-%d")
        || body.contains("commits")
        || body.contains("group_by")
    {
        return Err("release-plz must add only a dated release heading above reviewed Unreleased notes".to_owned());
    }

    let parsers = changelog
        .get("commit_parsers")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "release-plz must configure changelog commit parsers".to_owned())?;
    let expected = [
        ("^feat", Some("Added"), false),
        ("^fix", Some("Fixed"), false),
        ("^perf", Some("Performance"), false),
        ("^refactor", Some("Changed"), false),
        ("^revert", Some("Reverted"), false),
        ("^.*", None, true),
    ];
    if parsers.len() != expected.len() {
        return Err("release-plz must configure the exact code-only changelog parser set".to_owned());
    }
    for (parser, (message, group, skip)) in parsers.iter().zip(expected) {
        if parser["message"].as_str() != Some(message)
            || parser.get("group").and_then(toml::Value::as_str) != group
            || parser.get("skip").and_then(toml::Value::as_bool).unwrap_or(false) != skip
        {
            return Err(format!("release-plz changelog parser for {message} is invalid"));
        }
    }

    let releasing = fs::read_to_string(repository_root().join("docs/releasing.md"))
        .map_err(|error| format!("failed to read release documentation: {error}"))?;
    for required in [
        "classification contract",
        "`feat`, `fix`, `perf`, `refactor`, or `revert`",
        "`docs`, `test`, `ci`, `build`, `style`, or `chore`",
    ] {
        if !releasing.contains(required) {
            return Err(format!("release documentation is missing `{required}`"));
        }
    }

    Ok(())
}

#[test]
fn release_note_extraction_is_strict_and_bounded() -> Result<(), String> {
    let root = repository_root();
    let directory = std::env::temp_dir().join(format!("boxferry-release-notes-{}", std::process::id()));
    let changelog = directory.join("CHANGELOG.md");
    fs::create_dir_all(&directory).map_err(|error| format!("failed to create {}: {error}", directory.display()))?;

    fs::write(
        &changelog,
        "# Changelog\n\n## [Unreleased]\n\n## [1.2.3](https://example.invalid/v1.2.3) - 2026-08-17\n\n### Added\n\n- Useful change.\n\n## [1.2.2] - 2026-08-16\n\n- Older change.\n",
    )
    .map_err(|error| format!("failed to write {}: {error}", changelog.display()))?;
    let valid = run_release_notes_script(&root, "1.2.3", &changelog)?;
    let valid_stdout = String::from_utf8(valid.stdout).map_err(|error| error.to_string())?;
    if !valid.status.success() || !valid_stdout.contains("Useful change") || valid_stdout.contains("Older change") {
        return Err("valid release notes were not extracted as one bounded section".to_owned());
    }

    let missing = run_release_notes_script(&root, "9.9.9", &changelog)?;
    if missing.status.success() || !String::from_utf8_lossy(&missing.stderr).contains("no release section") {
        return Err("a missing release section must fail with an actionable diagnostic".to_owned());
    }
    let malformed_version = run_release_notes_script(&root, "v1.2.3", &changelog)?;
    if malformed_version.status.success()
        || !String::from_utf8_lossy(&malformed_version.stderr).contains("major.minor.patch")
    {
        return Err("a malformed release version must fail before extraction".to_owned());
    }

    fs::write(
        &changelog,
        "# Changelog\n\n## [1.2.3] - 2026-08-17\n\n## [1.2.2] - 2026-08-16\n",
    )
    .map_err(|error| format!("failed to write {}: {error}", changelog.display()))?;
    let empty = run_release_notes_script(&root, "1.2.3", &changelog)?;
    if empty.status.success() || !String::from_utf8_lossy(&empty.stderr).contains("is empty") {
        return Err("an empty release section must fail".to_owned());
    }

    fs::write(&changelog, "# Changelog\n\n## [1.2.3] - not-a-date\n\n- Change.\n")
        .map_err(|error| format!("failed to write {}: {error}", changelog.display()))?;
    let malformed_heading = run_release_notes_script(&root, "1.2.3", &changelog)?;
    if malformed_heading.status.success() || !String::from_utf8_lossy(&malformed_heading.stderr).contains("YYYY-MM-DD")
    {
        return Err("a malformed release heading must fail".to_owned());
    }

    fs::remove_dir_all(&directory).map_err(|error| format!("failed to remove {}: {error}", directory.display()))?;
    Ok(())
}

fn validate_release_plz_workflow(root: &Path, repository: &str) -> Result<(), String> {
    let workflow = fs::read_to_string(root.join(".github/workflows/release-plz.yml"))
        .map_err(|error| format!("failed to read release-plz workflow: {error}"))?;
    for required in [
        repository,
        "vars.RELEASE_PLZ_APP_CLIENT_ID",
        "client-id:",
        "secrets.RELEASE_PLZ_APP_PRIVATE_KEY",
        "permission-contents: write",
        "permission-pull-requests: write",
        "continue-on-error: true",
        "steps.app-token.outcome == 'failure'",
        "approve the updated permissions for the App installation",
        "command: release-pr",
        "renovate: datasource=crate depName=release-plz",
        "version: \"0.3.160\"",
        "release-plz/action@2eb1d8bcb770b4c48ccfaad919734b38b51958c9 # v0.5.131",
        "actions/create-github-app-token@bcd2ba49218906704ab6c1aa796996da409d3eb1 # v3.2.0",
        "(.head.ref | startswith(\"release-plz-\"))",
        "actions/workflows/release.yml/dispatches",
        "actions: write",
        "No release was dispatched.",
    ] {
        if !workflow.contains(required) {
            return Err(format!("release-plz workflow is missing `{required}`"));
        }
    }
    for forbidden in [
        "secrets.RELEASE_PLZ_APP_ID",
        "app-id:",
        "command: release\n",
        "cargo publish",
        "git tag",
        "gh release create",
    ] {
        if workflow.contains(forbidden) {
            return Err(format!("release-plz workflow must not contain `{forbidden}`"));
        }
    }
    Ok(())
}

#[test]
fn renovate_tracks_every_directly_pinned_development_tool() -> Result<(), String> {
    let root = repository_root();
    let renovate = fs::read_to_string(root.join(".github/renovate.json"))
        .map_err(|error| format!("failed to read Renovate configuration: {error}"))?;
    for required in [
        "Update versioned Dev Container tools",
        "Signal updates for checksum-pinned file-quality tools",
        "Update directly pinned workflow tool versions",
        "Update the documented Dev Container CLI",
        "Update the GitHub CLI installed in the Dev Container",
        r#""matchManagers": ["cargo"]"#,
        r#""matchManagers": ["npm"]"#,
        r#""matchManagers": ["github-actions"]"#,
        r#""matchManagers": ["devcontainer"]"#,
        r#""matchManagers": ["rust-toolchain"]"#,
        "Automerge tested non-major dependency updates",
        "Do not delay BoxFerry and Lens releases",
        r#""minimumReleaseAge": "0 days""#,
        r#""platformAutomerge": false"#,
        r#""boxferry-model""#,
        r#""compose-lens""#,
        r#""podman-lens""#,
        r#""quadlet-lens""#,
    ] {
        if !renovate.contains(required) {
            return Err(format!("Renovate configuration is missing `{required}`"));
        }
    }

    if renovate.matches(r#""automerge": false"#).count() != 2 {
        return Err("Renovate must keep Dev Container features and checksum-pinned tools manual".to_owned());
    }

    for workflow_name in ["ci.yml", "release.yml"] {
        let workflow = fs::read_to_string(root.join(".github/workflows").join(workflow_name))
            .map_err(|error| format!("failed to read {workflow_name}: {error}"))?;
        for required in [
            "renovate: datasource=crate depName=cargo-llvm-cov",
            "renovate: datasource=node-version depName=node",
        ] {
            if !workflow.contains(required) {
                return Err(format!("{workflow_name} is missing Renovate marker `{required}`"));
            }
        }
    }

    Ok(())
}

#[test]
fn renovate_keeps_zip_compatible_with_the_workspace_msrv() -> Result<(), String> {
    let path = repository_root().join(".github/renovate.json");
    let renovate = fs::read_to_string(&path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let renovate: serde_json::Value =
        serde_json::from_str(&renovate).map_err(|error| format!("failed to parse {}: {error}", path.display()))?;
    let rules = renovate["packageRules"]
        .as_array()
        .ok_or_else(|| "Renovate packageRules must be an array".to_owned())?;
    let zip_rule = rules
        .iter()
        .find(|rule| {
            rule["matchManagers"]
                .as_array()
                .is_some_and(|managers| managers.iter().any(|manager| manager == "cargo"))
                && rule["matchPackageNames"]
                    .as_array()
                    .is_some_and(|packages| packages.iter().any(|package| package == "zip"))
        })
        .ok_or_else(|| "Renovate must define a Cargo package rule for zip".to_owned())?;

    if zip_rule["allowedVersions"] != "<8.0.0" {
        return Err("Renovate must keep zip below its Rust 1.88 release line".to_owned());
    }

    Ok(())
}

fn run_release_notes_script(root: &Path, version: &str, changelog: &Path) -> Result<Output, String> {
    Command::new("bash")
        .arg(root.join("scripts/extract-release-notes.sh"))
        .arg(version)
        .arg(changelog)
        .current_dir(root)
        .output()
        .map_err(|error| format!("failed to run release-note extractor: {error}"))
}

#[test]
fn fixture_manifests_follow_the_common_contract() -> Result<(), String> {
    support::validate_fixture_tree(&repository_root(), FIXTURE_SUITES)
}

#[test]
fn maintainer_documentation_is_small_and_has_one_current_inventory() -> Result<(), String> {
    let root = repository_root();
    let docs = root.join("docs");
    let expected = BTreeSet::from([
        "README.md",
        "api-stability.md",
        "architecture.md",
        "dependency-policy.md",
        "development-environment.md",
        "platform-support.md",
        "releasing.md",
        "testing.md",
    ])
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    let actual = fs::read_dir(&docs)
        .map_err(|error| format!("failed to read {}: {error}", docs.display()))?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("md"))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(format!(
            "top-level maintainer guide inventory drifted: expected {expected:?}, found {actual:?}"
        ));
    }

    for (path, maximum_words) in [
        ("README.md", 800),
        ("AGENTS.md", 1_300),
        ("docs/README.md", 500),
        ("docs/api-stability.md", 800),
        ("docs/architecture.md", 1_500),
        ("docs/dependency-policy.md", 1_000),
        ("docs/development-environment.md", 800),
        ("docs/platform-support.md", 400),
        ("docs/releasing.md", 700),
        ("docs/testing.md", 1_300),
        ("fixtures/README.md", 1_000),
    ] {
        let text = fs::read_to_string(root.join(path)).map_err(|error| format!("failed to read {path}: {error}"))?;
        let words = text.split_whitespace().count();
        if words > maximum_words {
            return Err(format!(
                "{path} contains {words} words; current guidance is limited to {maximum_words}"
            ));
        }
    }

    for removed in [
        "docs/agent-workflow.md",
        "docs/fixture-format.md",
        "docs/implementation-plan.md",
        "docs/library-api.md",
        "docs/project-structure.md",
        "docs/research/podlet-compose-spec-rs-issues-2026-08-01.md",
    ] {
        if root.join(removed).exists() {
            return Err(format!("obsolete or duplicate documentation returned at {removed}"));
        }
    }

    Ok(())
}

#[test]
fn deferred_docker_and_runtime_namespaces_are_not_published_by_the_current_product() {
    for rule in boxferry_engine::RULES {
        assert!(
            !matches!(rule.code().get(..3), Some("BFD" | "BFR")),
            "deferred Docker or runtime rule namespace must not remain published: {}",
            rule.code()
        );
    }
}

#[test]
fn real_world_compose_catalog_is_immutable_and_reviewed() -> Result<(), String> {
    support::validate_real_world_compose_catalog(&repository_root())
}

#[test]
fn fixture_contract_accepts_authored_metadata() {
    let errors = support::validate_fixture_manifest_text(
        "valid fixture",
        r#"
schema = 1
id = "minimal-conversion"
suite = "conversion"
description = "Protects a minimal conversion."
secrets_reviewed = true
files = ["compose.yaml", "expected.container"]

[provenance]
source = "authored"
license = "MPL-2.0"
redistribution = "allowed"
modifications = "none"

[environment]
description = "No runtime environment is provided."

[expectations]
summary = "The workload converts without loss."
"#,
        FIXTURE_SUITES,
    );

    assert!(errors.is_empty(), "{errors:#?}");
}

#[test]
fn fixture_contract_rejects_unsafe_external_metadata() {
    let errors = support::validate_fixture_manifest_text(
        "invalid fixture",
        r#"
schema = 1
id = "external-project"
suite = "real-world"
description = "An incomplete external fixture."
secrets_reviewed = false
files = ["../secret.env"]

[provenance]
source = "external"
license = "unknown"
redistribution = "allowed"
modifications = "none"

[environment]
description = "Unknown."

[expectations]
summary = "Must not be accepted."
"#,
        FIXTURE_SUITES,
    );

    assert!(
        errors.iter().any(|error| error.contains("secrets_reviewed")),
        "{errors:#?}"
    );
    assert!(
        errors.iter().any(|error| error.contains("unsafe fixture path")),
        "{errors:#?}"
    );
    assert!(errors.iter().any(|error| error.contains("`url`")), "{errors:#?}");
    assert!(errors.iter().any(|error| error.contains("`revision`")), "{errors:#?}");
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn agent_roles_are_explicit() -> Result<(), Box<dyn std::error::Error>> {
    let root = repository_root();
    let config = fs::read_to_string(root.join(".codex/config.toml"))?;
    for required in [
        "model = \"gpt-5.6-sol\"",
        "model_reasoning_effort = \"xhigh\"",
        "max_concurrent_threads_per_session = 3",
        "default_subagent_model = \"gpt-5.6-terra\"",
        "default_subagent_reasoning_effort = \"medium\"",
    ] {
        assert!(config.contains(required), "missing agent default: {required}");
    }
    for (role, model, effort, sandbox) in [
        ("implementation-worker", "gpt-5.6-terra", "high", "workspace-write"),
        ("specification-researcher", "gpt-5.6-terra", "high", "read-only"),
        ("reviewer", "gpt-5.6-sol", "high", "read-only"),
        ("verifier", "gpt-5.6-terra", "medium", "workspace-write"),
    ] {
        let text = fs::read_to_string(root.join(format!(".codex/agents/{role}.toml")))?;
        for (key, value) in [
            ("model", model),
            ("model_reasoning_effort", effort),
            ("sandbox_mode", sandbox),
        ] {
            assert!(
                text.contains(&format!("{key} = \"{value}\"")),
                "{role}: incorrect {key}"
            );
        }
    }
    let reviewer = fs::read_to_string(root.join(".codex/agents/reviewer.toml"))?;
    assert!(reviewer.contains("original user requirements"));
    assert!(reviewer.contains("independent expected results"));
    let verifier = fs::read_to_string(root.join(".codex/agents/verifier.toml"))?;
    assert!(verifier.contains("./scripts/check-all.sh --check"));
    assert!(verifier.contains("never run the default formatting gate"));
    let instructions = fs::read_to_string(root.join("AGENTS.md"))?;
    assert!(!instructions.contains("Sol") && !instructions.contains("Terra") && !instructions.contains("Astra"));
    Ok(())
}

// The full shell gate targets the Linux Dev Container, not the macOS portability lane.
// Keep configuration assertions above platform-independent.
#[cfg(target_os = "linux")]
#[test]
fn linux_gate_modes_and_failure_propagation_are_correct() -> Result<(), Box<dyn std::error::Error>> {
    let root = repository_root();
    let result = Command::new("bash")
        .arg("scripts/test-check-all.sh")
        .current_dir(root)
        .output()?;
    assert!(
        result.status.success(),
        "gate mode regression failed:\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(())
}
