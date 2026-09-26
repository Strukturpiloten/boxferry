//! Executable repository and fixture-contract checks.

mod support;

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Output, Stdio},
    thread,
    time::{Duration, Instant},
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
const CRATES_IO_AUTH_ACTION: &str = "uses: rust-lang/crates-io-auth-action@";
const CRATES_IO_BOOTSTRAP_SECRET: &str = "secrets.CRATES_IO_BOOTSTRAP_TOKEN";

struct KillAndReapChild {
    child: Option<Child>,
}

impl KillAndReapChild {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child
            .as_mut()
            .ok_or_else(|| std::io::Error::other("bounded child already consumed"))?
            .try_wait()
    }

    fn kill(&mut self) -> std::io::Result<()> {
        self.child
            .as_mut()
            .ok_or_else(|| std::io::Error::other("bounded child already consumed"))?
            .kill()
    }

    fn wait_with_output(mut self) -> std::io::Result<Output> {
        self.child
            .take()
            .ok_or_else(|| std::io::Error::other("bounded child already consumed"))?
            .wait_with_output()
    }
}

impl Drop for KillAndReapChild {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn run_bounded_command(command: &mut Command, timeout: Duration, description: &str) -> Result<Output, String> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = KillAndReapChild::new(
        command
            .spawn()
            .map_err(|error| format!("failed to start {description}: {error}"))?,
    );
    let started = Instant::now();

    loop {
        if child
            .try_wait()
            .map_err(|error| format!("failed to poll {description}: {error}"))?
            .is_some()
        {
            return child
                .wait_with_output()
                .map_err(|error| format!("failed to collect {description} output: {error}"));
        }
        if started.elapsed() >= timeout {
            if let Err(error) = child.kill() {
                if child
                    .try_wait()
                    .map_err(|poll_error| format!("failed to poll timed-out {description}: {poll_error}"))?
                    .is_none()
                {
                    return Err(format!("failed to stop timed-out {description}: {error}"));
                }
            }
            let output = child
                .wait_with_output()
                .map_err(|error| format!("failed to reap timed-out {description}: {error}"))?;
            return Err(format!(
                "{description} timed out after {:.3} seconds; stdout: {}; stderr: {}",
                timeout.as_secs_f64(),
                String::from_utf8_lossy(&output.stdout).trim(),
                String::from_utf8_lossy(&output.stderr).trim(),
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn bounded_repository_policy_children_are_killed_and_reaped() -> Result<(), String> {
    let Err(error) = run_bounded_command(
        Command::new("python3").args(["-c", "import os, time; print(os.getpid(), flush=True); time.sleep(60)"]),
        Duration::from_secs(1),
        "synthetic sleeping child",
    ) else {
        return Err("sleeping child did not exceed its repository-policy deadline".to_owned());
    };
    if !error.contains("synthetic sleeping child timed out after 1.000 seconds") {
        return Err(format!("bounded child reported unexpected failure: {error}"));
    }
    let pid = error
        .split("; stdout: ")
        .nth(1)
        .and_then(|tail| tail.split(';').next())
        .filter(|pid| !pid.is_empty() && pid.bytes().all(|byte| byte.is_ascii_digit()))
        .ok_or_else(|| format!("bounded child did not report its PID: {error}"))?;
    let child_is_gone = Command::new("python3")
        .args([
            "-c",
            "import os, sys\ntry:\n os.kill(int(sys.argv[1]), 0)\nexcept ProcessLookupError:\n sys.exit(0)\nexcept PermissionError:\n sys.exit(2)\nelse:\n sys.exit(1)",
            pid,
        ])
        .status()
        .map_err(|check_error| format!("failed to check bounded child PID {pid}: {check_error}"))?;
    if !child_is_gone.success() {
        return Err(format!("bounded child PID {pid} still exists after timeout cleanup"));
    }
    Ok(())
}

fn shell_quoted_value<'a>(contents: &'a str, declaration: &str) -> Option<&'a str> {
    let prefix = format!("{declaration}=\"");
    contents.lines().find_map(|line| {
        line.trim()
            .strip_prefix(&prefix)
            .and_then(|value| value.strip_suffix('"'))
    })
}

fn validate_workflow_renovate_pins(workflow_name: &str, workflow: &str) -> Result<(), String> {
    let lines = workflow.lines().collect::<Vec<_>>();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let yaml_value = trimmed.strip_prefix("- ").unwrap_or(trimmed);
        let previous = index
            .checked_sub(1)
            .and_then(|previous| lines.get(previous))
            .map_or("", |line| line.trim());

        if let Some(runner) = trimmed.strip_prefix("runs-on:").map(str::trim) {
            let looks_hosted = ["ubuntu", "macos", "windows"]
                .iter()
                .any(|platform| runner.starts_with(platform) || runner.contains(&format!("{platform}-")));
            if !runner.contains("${{") && looks_hosted {
                let Some((platform, version)) = runner.split_once('-') else {
                    return Err(format!(
                        "{workflow_name}:{} malformed GitHub-hosted runner label `{runner}`",
                        index + 1
                    ));
                };
                let mut version_parts = version.split('-');
                let numeric_version = version_parts.next().unwrap_or_default();
                if !matches!(platform, "ubuntu" | "macos" | "windows")
                    || numeric_version.is_empty()
                    || !numeric_version
                        .split('.')
                        .all(|component| !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit()))
                    || version_parts.any(|suffix| {
                        suffix.is_empty()
                            || !suffix
                                .bytes()
                                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
                    })
                {
                    return Err(format!(
                        "{workflow_name}:{} fixed GitHub-hosted runner `{runner}` is not Renovate-managed",
                        index + 1
                    ));
                }
            }
        }

        if trimmed.starts_with("node-version:")
            && !trimmed.contains("${{")
            && previous != "# renovate: datasource=node-version depName=node"
        {
            return Err(format!(
                "{workflow_name}:{} has a literal Node.js version without its adjacent Renovate marker",
                index + 1
            ));
        }

        if trimmed.starts_with("version:")
            && !trimmed.contains("${{")
            && !previous.starts_with("# renovate: datasource=")
        {
            return Err(format!(
                "{workflow_name}:{} has a literal tool version without an adjacent Renovate marker",
                index + 1
            ));
        }

        if trimmed.contains("cargo install")
            && trimmed.contains("--version ")
            && !trimmed.contains("--version ${")
            && !previous.starts_with("# renovate: datasource=crate depName=")
        {
            return Err(format!(
                "{workflow_name}:{} installs a literal Cargo tool version without an adjacent Renovate marker",
                index + 1
            ));
        }

        if trimmed.contains("/releases/download/") {
            return Err(format!(
                "{workflow_name}:{} embeds a release download; use a Renovate-managed shared installer",
                index + 1
            ));
        }

        if (yaml_value.starts_with("image:") && !yaml_value.contains("${{"))
            || yaml_value.starts_with("uses: docker://")
            || ["docker pull ", "docker run ", "podman pull ", "podman run "]
                .iter()
                .any(|command| trimmed.contains(command))
        {
            return Err(format!(
                "{workflow_name}:{} embeds a container image; use a Renovate-managed script or catalogue",
                index + 1
            ));
        }
    }
    Ok(())
}

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
        .find("      - name: Run bounded serial evidence")
        .ok_or("migration-readiness workflow must run its bounded serial tier")?;
    let restore_contract = concat!(
        "      - name: Restore bounded evidence ownership\n",
        "        if: always() && steps.selection.outputs.privileged == 'true'\n",
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
        "- name: Validate serial evidence",
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
fn migration_readiness_protects_focused_task_evidence() -> Result<(), String> {
    let workflow = fs::read_to_string(repository_root().join(".github/workflows/migration-readiness.yml"))
        .map_err(|error| format!("failed to read migration-readiness workflow: {error}"))?;

    for required in [
        "task:\n        description: Optional exact task ID",
        "TASK: ${{ inputs.task }}",
        "workflow_call:\n    inputs:\n      tier:",
        "artifact=\"migration-readiness-focused-${TIER}-${GITHUB_SHA}-${GITHUB_RUN_ID}-${TASK}\"",
        "name: ${{ steps.selection.outputs.artifact }}",
        "(inputs.tier != 'pre-release' || inputs.task != '')",
        "printf 'name=migration-readiness-pre-release-%s-%s\\n'",
    ] {
        if !workflow.contains(required) {
            return Err(format!(
                "migration-readiness workflow must retain focused selection contract `{required}`"
            ));
        }
    }

    if workflow.matches("task_arguments=(--task \"${TASK}\")").count() != 3
        || workflow.matches("\"${task_arguments[@]}\"").count() != 3
    {
        return Err(
            "migration-readiness workflow must pass one quoted task selection to plan, runner, and evidence validation"
                .to_owned(),
        );
    }

    Ok(())
}

#[test]
fn migration_readiness_parallel_workers_are_bounded_and_exactly_bound() -> Result<(), String> {
    let workflow = fs::read_to_string(repository_root().join(".github/workflows/migration-readiness.yml"))
        .map_err(|error| format!("failed to read migration-readiness workflow: {error}"))?;
    for required in [
        "fail-fast: false",
        "max-parallel: 4",
        "matrix: ${{ fromJSON(needs.plan.outputs.workers) }}",
        "python3 scripts/migration-readiness.py coordinator-id",
        "COORDINATOR_ID: ${{ needs.plan.outputs.coordinator }}",
        "WORKER_ID: ${{ matrix.task }}",
        "BOXFERRY_BINARY_SHA256: ${{ needs.build.outputs.binary_sha256 }}",
        "printf 'name=migration-readiness-boxferry-%s-%s\\n'",
        "migration-readiness-worker-${{ github.sha }}-${{ github.run_id }}-${{ matrix.task }}",
        "migration-readiness-worker-${{ github.sha }}-${{ github.run_id }}-*",
        "overwrite: true",
        "--coordinator-id \"${COORDINATOR_ID}\"",
        "--worker-id \"${WORKER_ID}\"",
        "--boxferry-binary-sha256 \"${BOXFERRY_BINARY_SHA256}\"",
        "sha256sum --check --strict boxferry.sha256",
        "chmod 0755 boxferry",
        "uses: actions/download-artifact@",
        "if: always() && matrix.privileged",
        "sudo chown --recursive \"$(id -u):$(id -g)\" target/migration-readiness",
        "name: Upload complete aggregate evidence",
        "path: target/migration-readiness/evidence-v2.json",
    ] {
        if !workflow.contains(required) {
            return Err(format!("parallel migration-readiness workflow is missing `{required}`"));
        }
    }
    if workflow.contains("uuidgen") {
        return Err("migration-readiness workflow must use its portable UUID helper".to_owned());
    }
    if workflow.matches("max-parallel:").count() != 1 {
        return Err("migration-readiness must have one global worker concurrency cap".to_owned());
    }
    Ok(())
}

#[test]
fn privileged_migration_readiness_cargo_keeps_the_runner_toolchain() -> Result<(), String> {
    let root = repository_root();
    let workflow = fs::read_to_string(root.join(".github/workflows/migration-readiness.yml"))
        .map_err(|error| format!("failed to read migration-readiness workflow: {error}"))?;
    let catalogue = fs::read_to_string(root.join("fixtures/conformance/migration-readiness/tiers.toml"))
        .map_err(|error| format!("failed to read migration-readiness catalogue: {error}"))?;
    let quadlet = catalogue
        .split("id = \"quadlet-lens-candidate\"")
        .nth(1)
        .ok_or("catalogue must retain the QuadletLens candidate")?;
    let quadlet = quadlet.split("[[tasks]]").next().unwrap_or(quadlet);
    for required in ["privileged = true", "required-tools = [\"cargo\", \"git\", \"podman\"]"] {
        if !quadlet.contains(required) {
            return Err(format!("privileged QuadletLens candidate must retain `{required}`"));
        }
    }
    for required in [
        "CARGO_HOME=\"/home/runner/.cargo\"",
        "RUSTUP_HOME=\"/home/runner/.rustup\"",
    ] {
        if workflow.matches(required).count() != 2 {
            return Err(format!(
                "both privileged migration-readiness runners must retain `{required}`"
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
    let expected = r"on:
  workflow_call:
  push:
    branches:
      - main
  pull_request:
  workflow_dispatch:";
    if !workflow.contains(expected) {
        return Err(
            "CI must run for one pull-request event, main pushes, manual dispatch, and reusable calls".to_owned(),
        );
    }
    for forbidden in ["types: [opened", "types: [synchronize", "branches-ignore:"] {
        if workflow.contains(forbidden) {
            return Err(format!(
                "CI trigger must not narrow pull-request updates with `{forbidden}`"
            ));
        }
    }
    Ok(())
}

#[test]
fn supabase_success_contract_mismatch_summary_is_bounded_and_privacy_safe() -> Result<(), String> {
    let root = repository_root();
    let diagnostics =
        generated_supabase_contract(&root, "compose", "podman", "storage", SupabaseContractMode::Diagnostics)?;
    let fidelity = generated_supabase_contract(&root, "compose", "podman", "storage", SupabaseContractMode::Fidelity)?;
    let mut fidelity = fidelity
        .as_object()
        .cloned()
        .ok_or("generated Supabase fidelity must be an object")?;
    fidelity.insert("exact".to_owned(), serde_json::json!(7));
    let report = serde_json::json!({
        "schema_version": 1,
        "status": "success",
        "fidelity": fidelity,
        "diagnostics": diagnostics
            .as_array()
            .ok_or("generated Supabase diagnostics must be an array")?
            .iter()
            .map(supabase_contract_diagnostic)
            .collect::<Vec<_>>(),
    });
    assert!(supabase_contract_mismatches(&root, "compose", "podman", "storage", &report)?.is_empty());

    let cases = [
        ("schema-version", serde_json::json!(2)),
        ("status", serde_json::json!("private-status-value")),
    ];
    for (label, value) in cases {
        let mut mutated = report.clone();
        if label == "schema-version" {
            mutated["schema_version"] = value;
        } else {
            mutated["status"] = value;
        }
        assert_eq!(
            supabase_contract_mismatches(&root, "compose", "podman", "storage", &mutated)?,
            vec![label],
            "{label} mismatch label"
        );
    }
    assert_supabase_fidelity_mismatch_summaries(&root, &report)?;
    let mut empty_diagnostic_name = report.clone();
    empty_diagnostic_name["diagnostics"][0]["name"] = serde_json::json!("");
    assert_eq!(
        supabase_contract_mismatches(&root, "compose", "podman", "storage", &empty_diagnostic_name,)?,
        vec!["diagnostic-names"]
    );
    assert_eq!(
        supabase_contract_mismatch_summary(&root, "compose", "podman", "storage", &empty_diagnostic_name,)?["invalid_diagnostic_names"],
        serde_json::json!(1),
        "empty diagnostic names retain only their bounded count"
    );
    let mut protected_diagnostic_name = report.clone();
    protected_diagnostic_name["diagnostics"][0]["name"] = serde_json::json!({"secret": "private-diagnostic-value"});
    let protected_name_summary =
        supabase_contract_mismatch_summary(&root, "compose", "podman", "storage", &protected_diagnostic_name)?;
    assert_eq!(protected_name_summary["invalid_diagnostic_names"], serde_json::json!(1));
    assert!(
        !protected_name_summary.to_string().contains("private-diagnostic-value"),
        "mismatch summary exposed an arbitrary invalid diagnostic name"
    );
    let mut malformed_diagnostics = report;
    malformed_diagnostics["diagnostics"] = serde_json::json!({"name": "private-diagnostic-value"});
    assert_eq!(
        supabase_contract_mismatches(&root, "compose", "podman", "storage", &malformed_diagnostics,)?,
        vec!["diagnostic-names"]
    );
    let serialized = serde_json::to_string(&supabase_contract_mismatch_summary(
        &root,
        "compose",
        "podman",
        "storage",
        &serde_json::json!({
            "schema_version": 2,
            "status": "private-status-value",
            "fidelity": "private-fidelity-value",
            "diagnostics": [{"name": {"secret": "private-diagnostic-value"}}],
        }),
    )?)
    .map_err(|error| format!("failed to serialize bounded mismatch summary: {error}"))?;
    assert!(
        serialized.len() <= 4096,
        "mismatch summary exceeded the existing output bound"
    );
    assert!(
        !serialized.contains("private-status-value"),
        "mismatch summary exposed an arbitrary report value"
    );
    assert!(
        !serialized.contains("private-fidelity-value") && !serialized.contains("private-diagnostic-value"),
        "mismatch summary exposed malformed protected values"
    );
    Ok(())
}

fn assert_supabase_fidelity_mismatch_summaries(root: &Path, report: &serde_json::Value) -> Result<(), String> {
    let mut malformed_fidelity = report.clone();
    malformed_fidelity["fidelity"] = serde_json::json!({"exact": 7});
    assert_eq!(
        supabase_contract_mismatches(root, "compose", "podman", "storage", &malformed_fidelity)?,
        vec!["fidelity-shape"]
    );
    let mut non_object_fidelity = report.clone();
    non_object_fidelity["fidelity"] = serde_json::json!("private-fidelity-value");
    assert_eq!(
        supabase_contract_mismatches(root, "compose", "podman", "storage", &non_object_fidelity)?,
        vec!["fidelity-shape"]
    );
    for label in [
        "fidelity-approximate",
        "fidelity-unsupported",
        "fidelity-invalid",
        "fidelity-other",
    ] {
        let mut mutated = report.clone();
        let category = label.strip_prefix("fidelity-").ok_or("invalid test label")?;
        let expected = mutated["fidelity"][category].clone();
        mutated["fidelity"][category] = serde_json::json!(999_999);
        assert_eq!(
            supabase_contract_mismatches(root, "compose", "podman", "storage", &mutated)?,
            vec![label],
            "{label} mismatch label"
        );
        let summary = supabase_contract_mismatch_summary(root, "compose", "podman", "storage", &mutated)?;
        assert_eq!(
            summary["fidelity_counters"][category],
            serde_json::json!({"expected": expected, "actual": 999_999}),
            "{category} mismatch retains only its reviewed integer counters"
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
        "podman-live-outer-storage.sh",
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
    let capture_self_test = run_bounded_command(
        Command::new("python3").arg(&capture_tool).arg("--self-test"),
        Duration::from_secs(180),
        "Paperless capture proxy self-test",
    )?;
    if !capture_self_test.status.success() {
        return Err(format!(
            "Paperless capture proxy self-test failed: {}",
            String::from_utf8_lossy(&capture_self_test.stderr).trim()
        ));
    }
    let revalidation_tool = root.join("scripts/lib/podman-revalidation.py");
    let revalidation_self_test = run_bounded_command(
        Command::new("python3").arg(&revalidation_tool).arg("--self-test"),
        Duration::from_secs(30),
        "Podman revalidation self-test",
    )?;
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
        "realtime-readiness.mjs",
        "realtime-websocket.mjs",
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
    Mismatches,
}

impl SupabaseContractMode {
    const fn jq_value(self) -> &'static str {
        match self {
            Self::Validate => "false",
            Self::Diagnostics => "true",
            Self::Fidelity => "\"fidelity\"",
            Self::Mismatches => "\"mismatches\"",
        }
    }
}

#[derive(Clone, Copy)]
struct SupabaseContractContext<'a> {
    podman_acquisition: &'a str,
    provisioner_mode: &'a str,
    include_system_network: bool,
}

fn run_supabase_report_contract(
    root: &Path,
    input: &str,
    output: &str,
    selection: &str,
    context: SupabaseContractContext<'_>,
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
        .args(["--arg", "podman_acquisition", context.podman_acquisition])
        .args(["--arg", "provisioner_mode", context.provisioner_mode])
        .args([
            "--argjson",
            "include_system_network",
            if context.include_system_network {
                "true"
            } else {
                "false"
            },
        ])
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

fn default_supabase_contract_acquisition(input: &str) -> &'static str {
    if input == "podman" { "cli" } else { "not-podman" }
}

fn default_supabase_contract_provisioner(podman_acquisition: &str) -> &'static str {
    match podman_acquisition {
        "compose" => "compose",
        _ => "cli",
    }
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
    supabase_contract_accepts_with_acquisition(
        root,
        input,
        output,
        selection,
        default_supabase_contract_acquisition(input),
        report,
    )
}

fn supabase_contract_accepts_with_acquisition(
    root: &Path,
    input: &str,
    output: &str,
    selection: &str,
    podman_acquisition: &str,
    report: &serde_json::Value,
) -> Result<bool, String> {
    let result = run_supabase_report_contract(
        root,
        input,
        output,
        selection,
        SupabaseContractContext {
            podman_acquisition,
            provisioner_mode: default_supabase_contract_provisioner(podman_acquisition),
            include_system_network: false,
        },
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

fn supabase_contract_mismatch_summary(
    root: &Path,
    input: &str,
    output: &str,
    selection: &str,
    report: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let result = run_supabase_report_contract(
        root,
        input,
        output,
        selection,
        SupabaseContractContext {
            podman_acquisition: default_supabase_contract_acquisition(input),
            provisioner_mode: default_supabase_contract_provisioner(default_supabase_contract_acquisition(input)),
            include_system_network: false,
        },
        SupabaseContractMode::Mismatches,
        Some(report),
    )?;
    if !result.status.success() {
        return Err(format!(
            "failed to produce bounded Supabase mismatch summary: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    serde_json::from_slice(&result.stdout)
        .map_err(|error| format!("invalid bounded Supabase mismatch summary: {error}"))
}

fn supabase_contract_mismatches(
    root: &Path,
    input: &str,
    output: &str,
    selection: &str,
    report: &serde_json::Value,
) -> Result<Vec<String>, String> {
    let summary = supabase_contract_mismatch_summary(root, input, output, selection, report)?;
    summary["failed_predicates"]
        .as_array()
        .ok_or("bounded Supabase mismatch summary lacks failed predicate labels")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or("bounded Supabase mismatch label is not a string".to_owned())
        })
        .collect()
}

fn supabase_contract_accepts_including_system_network(
    root: &Path,
    input: &str,
    output: &str,
    selection: &str,
    report: &serde_json::Value,
) -> Result<bool, String> {
    supabase_contract_accepts_including_system_network_with_acquisition(
        root,
        input,
        output,
        selection,
        default_supabase_contract_acquisition(input),
        report,
    )
}

fn supabase_contract_accepts_including_system_network_with_acquisition(
    root: &Path,
    input: &str,
    output: &str,
    selection: &str,
    podman_acquisition: &str,
    report: &serde_json::Value,
) -> Result<bool, String> {
    let result = run_supabase_report_contract(
        root,
        input,
        output,
        selection,
        SupabaseContractContext {
            podman_acquisition,
            provisioner_mode: default_supabase_contract_provisioner(podman_acquisition),
            include_system_network: true,
        },
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
    generated_supabase_contract_with_acquisition(
        root,
        input,
        output,
        selection,
        default_supabase_contract_acquisition(input),
        mode,
    )
}

fn generated_supabase_contract_with_acquisition(
    root: &Path,
    input: &str,
    output: &str,
    selection: &str,
    podman_acquisition: &str,
    mode: SupabaseContractMode,
) -> Result<serde_json::Value, String> {
    let generated = run_supabase_report_contract(
        root,
        input,
        output,
        selection,
        SupabaseContractContext {
            podman_acquisition,
            provisioner_mode: default_supabase_contract_provisioner(podman_acquisition),
            include_system_network: false,
        },
        mode,
        None,
    )?;
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
fn supabase_provider_origin_controls_exact_kong_podman_losses() -> Result<(), String> {
    let root = repository_root();
    let kong_subjects = BTreeSet::from([
        "services.contract-supabase-kong.environment.KONG_NGINX_PROXY_PROXY_BUFFER_SIZE".to_owned(),
        "services.contract-supabase-kong.environment.KONG_NGINX_PROXY_PROXY_BUFFERS".to_owned(),
    ]);

    for selection in ["exact", "storage", "label", "all"] {
        for (input, cli_acquisition, compose_acquisition) in [
            ("podman", "cli", "compose"),
            ("compose", "not-podman", "not-podman"),
            ("quadlet", "not-podman", "not-podman"),
        ] {
            let expected = |podman_acquisition, provisioner_mode, mode| {
                run_supabase_report_contract(
                    &root,
                    input,
                    "podman",
                    selection,
                    SupabaseContractContext {
                        podman_acquisition,
                        provisioner_mode,
                        include_system_network: false,
                    },
                    mode,
                    None,
                )
            };
            let cli_diagnostics = expected(cli_acquisition, "cli", SupabaseContractMode::Diagnostics)?;
            let compose_diagnostics = expected(compose_acquisition, "compose", SupabaseContractMode::Diagnostics)?;
            assert!(cli_diagnostics.status.success());
            assert!(compose_diagnostics.status.success());
            let subjects = |output: &Output| -> Result<BTreeSet<String>, String> {
                let diagnostics: serde_json::Value = serde_json::from_slice(&output.stdout)
                    .map_err(|error| format!("invalid Supabase diagnostics: {error}"))?;
                Ok(diagnostics
                    .as_array()
                    .ok_or("Supabase diagnostics must be an array")?
                    .iter()
                    .filter(|diagnostic| diagnostic["code"] == "BFP0007")
                    .filter_map(|diagnostic| diagnostic["subject"].as_str())
                    .filter(|subject| subject.contains("services.contract-supabase-kong.environment."))
                    .map(str::to_owned)
                    .collect())
            };
            let cli_subjects = subjects(&cli_diagnostics)?;
            let compose_subjects = subjects(&compose_diagnostics)?;
            assert!(kong_subjects.is_disjoint(&cli_subjects));
            assert!(kong_subjects.is_subset(&compose_subjects));
            assert_eq!(
                compose_subjects.difference(&cli_subjects).collect::<BTreeSet<_>>(),
                kong_subjects.iter().collect::<BTreeSet<_>>()
            );

            let cli_fidelity: serde_json::Value =
                serde_json::from_slice(&expected(cli_acquisition, "cli", SupabaseContractMode::Fidelity)?.stdout)
                    .map_err(|error| format!("invalid CLI Supabase fidelity: {error}"))?;
            let compose_fidelity: serde_json::Value = serde_json::from_slice(
                &expected(compose_acquisition, "compose", SupabaseContractMode::Fidelity)?.stdout,
            )
            .map_err(|error| format!("invalid Compose Supabase fidelity: {error}"))?;
            if input == "podman" {
                // Native Compose acquisition has separately reviewed creation-evidence
                // accounting. Its exact counter nevertheless includes only these two
                // additional Kong environment omissions.
                let expected = if selection == "all" { 1_619 } else { 1_507 };
                assert_eq!(compose_fidelity["unsupported"].as_u64(), Some(expected));
            } else if input == "quadlet" {
                // Compose origin drops 16 native dependencies and adds two Kong omissions.
                assert_eq!(
                    compose_fidelity["unsupported"].as_u64(),
                    cli_fidelity["unsupported"].as_u64().map(|value| value - 14),
                    "Quadlet provider provenance changes dependency and Kong evidence"
                );
            } else {
                assert_eq!(
                    compose_fidelity["unsupported"].as_u64(),
                    cli_fidelity["unsupported"].as_u64().map(|value| value + 2),
                    "{input} Compose-provider Kong environment evidence adds exactly two omissions"
                );
            }
        }
    }

    for (input, acquisition, provisioner) in [
        ("podman", "cli", "compose"),
        ("podman", "compose", "cli"),
        ("podman", "unknown", "cli"),
        ("compose", "cli", "cli"),
        ("quadlet", "compose", "compose"),
        ("compose", "not-podman", "unknown"),
    ] {
        let invalid = run_supabase_report_contract(
            &root,
            input,
            "podman",
            "exact",
            SupabaseContractContext {
                podman_acquisition: acquisition,
                provisioner_mode: provisioner,
                include_system_network: false,
            },
            SupabaseContractMode::Diagnostics,
            None,
        )?;
        assert_eq!((invalid.status.code(), &*invalid.stdout), (Some(1), &b"null\n"[..]));
    }
    Ok(())
}

#[test]
#[allow(
    clippy::too_many_lines,
    reason = "keeps the complete Supabase report matrix and its counterexamples auditable as one contract"
)]
fn supabase_report_contract_rejects_subject_and_fidelity_counterexamples() -> Result<(), String> {
    let root = repository_root();

    for (input, selection, unsupported) in [
        ("compose", "exact", 198),
        ("compose", "storage", 198),
        ("compose", "label", 198),
        ("compose", "all", 212),
        ("quadlet", "exact", 223),
        ("quadlet", "storage", 223),
        ("quadlet", "label", 223),
        ("quadlet", "all", 237),
    ] {
        let generated =
            generated_supabase_contract(&root, input, "podman", selection, SupabaseContractMode::Diagnostics)?;
        let diagnostics = generated
            .as_array()
            .ok_or("generated successful Podman contract must be an array")?;
        assert_eq!(
            diagnostics.len(),
            unsupported,
            "{input} {selection} target-loss diagnostic count"
        );
        assert!(
            diagnostics.iter().all(|diagnostic| {
                diagnostic["code"] == "BFP0007"
                    && diagnostic["severity"] == "warning"
                    && diagnostic["decision"] == "omitted"
            }),
            "{input} {selection} generated a non-target-loss diagnostic"
        );
        let unique_tuples = diagnostics
            .iter()
            .map(|diagnostic| {
                format!(
                    "{}\t{}\t{}",
                    diagnostic["code"], diagnostic["subject"], diagnostic["decision"]
                )
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            unique_tuples.len(),
            diagnostics.len(),
            "{input} {selection} diagnostic tuples must be unique"
        );
        assert_eq!(
            generated_supabase_contract(&root, input, "podman", selection, SupabaseContractMode::Fidelity,)?,
            serde_json::json!({
                "approximate": 0,
                "unsupported": unsupported,
                "invalid": 0,
                "other": 0,
            }),
            "{input} {selection} successful-route fidelity"
        );
    }

    for (input, unsupported) in [("compose", 198), ("quadlet", 223)] {
        let expected =
            generated_supabase_contract(&root, input, "podman", "storage", SupabaseContractMode::Diagnostics)?;
        let expected = expected
            .as_array()
            .ok_or("generated storage Podman contract must be an array")?;
        let report = serde_json::json!({
            "schema_version": 1,
            "status": "success",
            "fidelity": {
                "exact": 7,
                "approximate": 0,
                "unsupported": unsupported,
                "invalid": 0,
                "other": 0,
            },
            "diagnostics": expected
                .iter()
                .map(supabase_contract_diagnostic)
                .collect::<Vec<_>>(),
        });
        assert!(
            supabase_contract_accepts(&root, input, "podman", "storage", &report)?,
            "{input} successful Podman route"
        );
    }

    let expected =
        generated_supabase_contract(&root, "compose", "podman", "storage", SupabaseContractMode::Diagnostics)?;
    let expected = expected
        .as_array()
        .ok_or("generated Compose-to-Podman contract must be an array")?;
    let success_report = serde_json::json!({
        "schema_version": 1,
        "status": "success",
        "fidelity": {
            "exact": 7,
            "approximate": 0,
            "unsupported": 198,
            "invalid": 0,
            "other": 0,
        },
        "diagnostics": expected
            .iter()
            .map(supabase_contract_diagnostic)
            .collect::<Vec<_>>(),
    });

    let mut duplicate = success_report.clone();
    let first = duplicate["diagnostics"][0].clone();
    duplicate["diagnostics"][1] = first;
    assert!(!supabase_contract_accepts(
        &root, "compose", "podman", "storage", &duplicate
    )?);

    let mut unseen = success_report.clone();
    unseen["diagnostics"][0]["fields"][0]["value"] = serde_json::json!("networks.contract-supabase-unseen.internal");
    assert!(!supabase_contract_accepts(
        &root, "compose", "podman", "storage", &unseen
    )?);

    let mut missing_target_loss = success_report.clone();
    missing_target_loss["diagnostics"]
        .as_array_mut()
        .ok_or("synthetic success diagnostics must be an array")?
        .remove(0);
    assert!(!supabase_contract_accepts(
        &root,
        "compose",
        "podman",
        "storage",
        &missing_target_loss
    )?);

    let mut wrong_target_fidelity = success_report.clone();
    wrong_target_fidelity["fidelity"]["unsupported"] = serde_json::json!(199);
    assert!(!supabase_contract_accepts(
        &root,
        "compose",
        "podman",
        "storage",
        &wrong_target_fidelity
    )?);

    let mut invalid_fidelity = success_report.clone();
    invalid_fidelity["fidelity"]["invalid"] = serde_json::json!(1);
    assert!(!supabase_contract_accepts(
        &root,
        "compose",
        "podman",
        "storage",
        &invalid_fidelity
    )?);

    let mut failure_status = success_report;
    failure_status["status"] = serde_json::json!("failure");
    assert!(!supabase_contract_accepts(
        &root,
        "compose",
        "podman",
        "storage",
        &failure_status
    )?);

    for (input, podman_acquisition) in [
        ("podman", "not-podman"),
        ("podman", "future"),
        ("podman", ""),
        ("compose", "cli"),
        ("compose", "compose"),
        ("compose", "future"),
        ("quadlet", "cli"),
        ("quadlet", "compose"),
        ("quadlet", "future"),
    ] {
        for mode in [SupabaseContractMode::Diagnostics, SupabaseContractMode::Fidelity] {
            let invalid = run_supabase_report_contract(
                &root,
                input,
                "compose",
                "storage",
                SupabaseContractContext {
                    podman_acquisition,
                    provisioner_mode: "cli",
                    include_system_network: false,
                },
                mode,
                None,
            )?;
            assert_eq!(
                invalid.status.code(),
                Some(1),
                "{input} input unexpectedly admitted {podman_acquisition:?} acquisition"
            );
            assert_eq!(invalid.stdout, b"null\n");
        }
    }

    let expected_quadlet_compose_subjects = BTreeSet::from([
        "networks.contract-supabase-backend.ipam.config",
        "services.contract-supabase-auth.dependencies[0]",
        "services.contract-supabase-auth.healthcheck",
        "services.contract-supabase-db.healthcheck",
        "services.contract-supabase-functions.dependencies[0]",
        "services.contract-supabase-functions.healthcheck",
        "services.contract-supabase-imgproxy.healthcheck",
        "services.contract-supabase-kong.dependencies[0]",
        "services.contract-supabase-kong.dependencies[1]",
        "services.contract-supabase-kong.dependencies[2]",
        "services.contract-supabase-kong.dependencies[3]",
        "services.contract-supabase-kong.dependencies[4]",
        "services.contract-supabase-kong.dependencies[5]",
        "services.contract-supabase-kong.healthcheck",
        "services.contract-supabase-meta.dependencies[0]",
        "services.contract-supabase-realtime.dependencies[0]",
        "services.contract-supabase-realtime.healthcheck",
        "services.contract-supabase-rest.dependencies[0]",
        "services.contract-supabase-rest.healthcheck",
        "services.contract-supabase-storage.dependencies[0]",
        "services.contract-supabase-storage.dependencies[1]",
        "services.contract-supabase-storage.dependencies[2]",
        "services.contract-supabase-storage.healthcheck",
        "services.contract-supabase-studio.dependencies[0]",
        "services.contract-supabase-studio.healthcheck",
        "services.contract-supabase-supavisor.dependencies[0]",
    ]);
    let expected_compose_authored_podman_compose_subjects = BTreeSet::from([
        "networks.contract-supabase-backend.ipam.config",
        "networks.contract-supabase-edge.ipam.config",
        "networks.contract-supabase-edge.internal",
        "networks.contract-supabase-edge.labels",
        "services.contract-supabase-auth.healthcheck",
        "services.contract-supabase-db.healthcheck",
        "services.contract-supabase-functions.healthcheck",
        "services.contract-supabase-imgproxy.healthcheck",
        "services.contract-supabase-kong.healthcheck",
        "services.contract-supabase-realtime.healthcheck",
        "services.contract-supabase-rest.healthcheck",
        "services.contract-supabase-storage.healthcheck",
        "services.contract-supabase-studio.healthcheck",
    ]);
    let application_creation_evidence_subjects = BTreeSet::from([
        "services.contract-supabase-auth.creation_evidence",
        "services.contract-supabase-db.creation_evidence",
        "services.contract-supabase-functions.creation_evidence",
        "services.contract-supabase-imgproxy.creation_evidence",
        "services.contract-supabase-kong.creation_evidence",
        "services.contract-supabase-meta.creation_evidence",
        "services.contract-supabase-realtime.creation_evidence",
        "services.contract-supabase-rest.creation_evidence",
        "services.contract-supabase-storage.creation_evidence",
        "services.contract-supabase-studio.creation_evidence",
        "services.contract-supabase-supavisor.creation_evidence",
    ]);
    let mut expected_cli_authored_podman_compose_subjects = expected_quadlet_compose_subjects.clone();
    expected_cli_authored_podman_compose_subjects.extend([
        "networks.contract-supabase-edge.ipam.config",
        "networks.contract-supabase-edge.internal",
        "networks.contract-supabase-edge.labels",
    ]);
    for selection in ["exact", "storage", "label", "all"] {
        let cli_authored_diagnostics =
            generated_supabase_contract(&root, "podman", "compose", selection, SupabaseContractMode::Diagnostics)?;
        let cli_authored_subjects = cli_authored_diagnostics
            .as_array()
            .ok_or("Podman-to-Compose diagnostics must be an array")?
            .iter()
            .filter(|diagnostic| diagnostic["code"] == "BFC0007")
            .map(|diagnostic| {
                diagnostic["subject"]
                    .as_str()
                    .ok_or("Podman-to-Compose target-loss subject must be a string")
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        assert_eq!(
            cli_authored_subjects, expected_cli_authored_podman_compose_subjects,
            "{selection} CLI-authored Podman-to-Compose independent loss subjects"
        );
        let compose_authored_diagnostics = generated_supabase_contract_with_acquisition(
            &root,
            "podman",
            "compose",
            selection,
            "compose",
            SupabaseContractMode::Diagnostics,
        )?;
        let compose_authored_subjects = compose_authored_diagnostics
            .as_array()
            .ok_or("Compose-authored Podman-to-Compose diagnostics must be an array")?
            .iter()
            .filter(|diagnostic| diagnostic["code"] == "BFC0007")
            .map(|diagnostic| {
                diagnostic["subject"]
                    .as_str()
                    .ok_or("Compose-authored Podman-to-Compose target-loss subject must be a string")
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        assert_eq!(
            compose_authored_subjects, expected_compose_authored_podman_compose_subjects,
            "{selection} Compose-authored Podman-to-Compose must not invent absent source intent"
        );
        assert!(
            compose_authored_subjects
                .iter()
                .all(|subject| !subject.contains(".dependencies[")),
            "{selection} Compose-authored Podman-to-Compose must retain authored dependencies"
        );
        let cli_creation_evidence_subjects = cli_authored_diagnostics
            .as_array()
            .ok_or("CLI-authored Podman-to-Compose diagnostics must be an array")?
            .iter()
            .filter(|diagnostic| diagnostic["code"] == "BFP0002")
            .filter_map(|diagnostic| diagnostic["subject"].as_str())
            .filter(|subject| subject.ends_with(".creation_evidence"))
            .collect::<BTreeSet<_>>();
        let compose_creation_evidence_subjects = compose_authored_diagnostics
            .as_array()
            .ok_or("Compose-authored Podman-to-Compose diagnostics must be an array")?
            .iter()
            .filter(|diagnostic| diagnostic["code"] == "BFP0002")
            .filter_map(|diagnostic| diagnostic["subject"].as_str())
            .filter(|subject| subject.ends_with(".creation_evidence"))
            .collect::<BTreeSet<_>>();
        let mut expected_cli_creation_evidence_subjects = application_creation_evidence_subjects.clone();
        let expected_compose_creation_evidence_subjects = BTreeSet::new();
        if selection == "all" {
            expected_cli_creation_evidence_subjects
                .insert("services.contract-supabase-boundary-peer.creation_evidence");
        }
        assert_eq!(
            cli_creation_evidence_subjects, expected_cli_creation_evidence_subjects,
            "{selection} CLI acquisition must retain all observed creation evidence"
        );
        assert_eq!(
            compose_creation_evidence_subjects, expected_compose_creation_evidence_subjects,
            "{selection} Compose acquisition must omit all creation evidence"
        );
        let (approximate, cli_unsupported, compose_unsupported): (usize, usize, usize) = if selection == "all" {
            (67, 1_446, 1_406)
        } else {
            (63, 1_346, 1_308)
        };
        assert_eq!(
            cli_unsupported - compose_unsupported,
            16 + (expected_cli_creation_evidence_subjects.len() * 2),
            "{selection} origin delta must be sixteen dependency outcomes plus two occurrences for each CLI-only creation-evidence record"
        );
        let cli_authored_report = serde_json::json!({
            "schema_version": 1,
            "status": "success",
            "fidelity": {
                "exact": 0,
                "approximate": approximate,
                "unsupported": cli_unsupported,
                "invalid": 0,
                "other": 0,
            },
            "diagnostics": cli_authored_diagnostics
                .as_array()
                .ok_or("CLI-authored Podman-to-Compose diagnostics must be an array")?
                .iter()
                .map(supabase_contract_diagnostic)
                .collect::<Vec<_>>(),
        });
        let compose_authored_report = serde_json::json!({
            "schema_version": 1,
            "status": "success",
            "fidelity": {
                "exact": 0,
                "approximate": approximate,
                "unsupported": compose_unsupported,
                "invalid": 0,
                "other": 0,
            },
            "diagnostics": compose_authored_diagnostics
                .as_array()
                .ok_or("Compose-authored Podman-to-Compose diagnostics must be an array")?
                .iter()
                .map(supabase_contract_diagnostic)
                .collect::<Vec<_>>(),
        });
        assert!(supabase_contract_accepts_with_acquisition(
            &root,
            "podman",
            "compose",
            selection,
            "cli",
            &cli_authored_report,
        )?);
        assert!(supabase_contract_accepts_with_acquisition(
            &root,
            "podman",
            "compose",
            selection,
            "compose",
            &compose_authored_report,
        )?);
        if selection == "all" {
            let mut injected_compose_creation_evidence = compose_authored_report.clone();
            injected_compose_creation_evidence["diagnostics"]
                .as_array_mut()
                .ok_or("Compose-authored all report diagnostics must be an array")?
                .push(serde_json::json!({
                    "code": "BFP0002",
                    "severity": "warning",
                    "name": "injected Compose creation evidence",
                    "fields": [
                        {"name": "subject", "value": "services.contract-supabase-boundary-peer.creation_evidence"},
                        {"name": "decision", "value": "omitted"}
                    ]
                }));
            assert!(
                !supabase_contract_accepts_with_acquisition(
                    &root,
                    "podman",
                    "compose",
                    selection,
                    "compose",
                    &injected_compose_creation_evidence,
                )?,
                "all Compose-authored contract admitted injected boundary creation evidence"
            );

            let mut missing_cli_peer_evidence = cli_authored_report.clone();
            missing_cli_peer_evidence["diagnostics"]
                .as_array_mut()
                .ok_or("CLI-authored all report diagnostics must be an array")?
                .retain(|diagnostic| {
                    diagnostic["fields"].as_array().is_none_or(|fields| {
                        !fields.iter().any(|field| {
                            field["name"] == "subject"
                                && field["value"] == "services.contract-supabase-boundary-peer.creation_evidence"
                        })
                    })
                });
            missing_cli_peer_evidence["fidelity"]["unsupported"] = serde_json::json!(cli_unsupported - 2);
            assert!(
                !supabase_contract_accepts_with_acquisition(
                    &root,
                    "podman",
                    "compose",
                    selection,
                    "cli",
                    &missing_cli_peer_evidence,
                )?,
                "all CLI-authored contract admitted missing boundary creation evidence"
            );
        }
        let mut stale_compose_authored_report = compose_authored_report.clone();
        stale_compose_authored_report["fidelity"]["unsupported"] = serde_json::json!(compose_unsupported + 11);
        assert!(
            !supabase_contract_accepts_with_acquisition(
                &root,
                "podman",
                "compose",
                selection,
                "compose",
                &stale_compose_authored_report,
            )?,
            "{selection} Compose-authored contract admitted the stale creation-evidence count"
        );
        assert!(!supabase_contract_accepts_with_acquisition(
            &root,
            "podman",
            "compose",
            selection,
            "compose",
            &cli_authored_report,
        )?);
        assert!(!supabase_contract_accepts_with_acquisition(
            &root,
            "podman",
            "compose",
            selection,
            "cli",
            &compose_authored_report,
        )?);
        let compose_diagnostics = generated_supabase_contract(
            &root,
            "compose",
            "compose",
            selection,
            SupabaseContractMode::Diagnostics,
        )?;
        assert_eq!(
            compose_diagnostics,
            serde_json::json!([]),
            "{selection} Compose-to-Compose must be a zero-diagnostic digest-only reimport"
        );
        let compose_fidelity =
            generated_supabase_contract(&root, "compose", "compose", selection, SupabaseContractMode::Fidelity)?;
        assert_eq!(
            compose_fidelity,
            serde_json::json!({"approximate": 0, "unsupported": 0, "invalid": 0, "other": 0}),
            "{selection} Compose-to-Compose must have zero fidelity loss"
        );
        let compose_report = serde_json::json!({
            "schema_version": 1,
            "status": "success",
            "fidelity": {"exact": 0, "approximate": 0, "unsupported": 0, "invalid": 0, "other": 0},
            "diagnostics": [],
        });
        assert!(supabase_contract_accepts(
            &root,
            "compose",
            "compose",
            selection,
            &compose_report,
        )?);
        let mut spurious_image_approximation = compose_report.clone();
        spurious_image_approximation["diagnostics"]
            .as_array_mut()
            .ok_or("synthetic Compose reimport diagnostics must be an array")?
            .push(serde_json::json!({
                "code": "BFC0009",
                "severity": "warning",
                "name": "spurious tag-plus-digest approximation",
                "fields": [
                    {"name": "subject", "value": "services.contract-supabase-auth.image"},
                    {"name": "decision", "value": null},
                ],
            }));
        assert!(
            !supabase_contract_accepts(&root, "compose", "compose", selection, &spurious_image_approximation,)?,
            "{selection} Compose-to-Compose must reject a spurious BFC0009"
        );

        let quadlet_diagnostics = generated_supabase_contract(
            &root,
            "quadlet",
            "compose",
            selection,
            SupabaseContractMode::Diagnostics,
        )?;
        let quadlet_diagnostics = quadlet_diagnostics
            .as_array()
            .ok_or("generated Quadlet-to-Compose diagnostics must be an array")?;
        assert_eq!(
            quadlet_diagnostics.len(),
            26,
            "{selection} Quadlet-to-Compose diagnostic count"
        );
        assert!(quadlet_diagnostics.iter().all(|diagnostic| {
            diagnostic["code"] == "BFC0007" && diagnostic["severity"] == "warning" && diagnostic["decision"].is_null()
        }));
        let quadlet_subjects = quadlet_diagnostics
            .iter()
            .map(|diagnostic| {
                diagnostic["subject"]
                    .as_str()
                    .ok_or("Quadlet-to-Compose diagnostic subject must be a string")
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        assert_eq!(
            quadlet_subjects, expected_quadlet_compose_subjects,
            "{selection} Quadlet-to-Compose independent loss subjects"
        );
        let quadlet_fidelity =
            generated_supabase_contract(&root, "quadlet", "compose", selection, SupabaseContractMode::Fidelity)?;
        assert_eq!(
            quadlet_fidelity,
            serde_json::json!({"approximate": 0, "unsupported": 26, "invalid": 0, "other": 0}),
            "{selection} Quadlet-to-Compose has only BFC0007 unsupported loss"
        );
        let quadlet_report = serde_json::json!({
            "schema_version": 1,
            "status": "success",
            "fidelity": {"exact": 0, "approximate": 0, "unsupported": 26, "invalid": 0, "other": 0},
            "diagnostics": quadlet_diagnostics.iter().map(supabase_contract_diagnostic).collect::<Vec<_>>(),
        });
        assert!(supabase_contract_accepts(
            &root,
            "quadlet",
            "compose",
            selection,
            &quadlet_report,
        )?);
        let mut spurious_quadlet_image_approximation = quadlet_report.clone();
        spurious_quadlet_image_approximation["diagnostics"]
            .as_array_mut()
            .ok_or("synthetic Quadlet-to-Compose diagnostics must be an array")?
            .push(serde_json::json!({
                "code": "BFC0009",
                "severity": "warning",
                "name": "spurious tag-plus-digest approximation",
                "fields": [
                    {"name": "subject", "value": "services.contract-supabase-auth.image"},
                    {"name": "decision", "value": null},
                ],
            }));
        assert!(
            !supabase_contract_accepts(
                &root,
                "quadlet",
                "compose",
                selection,
                &spurious_quadlet_image_approximation,
            )?,
            "{selection} Quadlet-to-Compose must reject BFC0009"
        );
    }

    let compose_origin_quadlet_compose = run_supabase_report_contract(
        &root,
        "quadlet",
        "compose",
        "storage",
        SupabaseContractContext {
            podman_acquisition: "not-podman",
            provisioner_mode: "compose",
            include_system_network: false,
        },
        SupabaseContractMode::Diagnostics,
        None,
    )?;
    assert!(compose_origin_quadlet_compose.status.success());
    let compose_origin_quadlet_compose: serde_json::Value =
        serde_json::from_slice(&compose_origin_quadlet_compose.stdout)
            .map_err(|error| format!("invalid Compose-origin Quadlet-to-Compose contract: {error}"))?;
    let compose_origin_quadlet_compose = compose_origin_quadlet_compose
        .as_array()
        .ok_or("Compose-origin Quadlet-to-Compose diagnostics must be an array")?;
    let compose_origin_quadlet_compose_subjects = compose_origin_quadlet_compose
        .iter()
        .map(|diagnostic| {
            diagnostic["subject"]
                .as_str()
                .ok_or("Compose-origin Quadlet-to-Compose subject must be a string")
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    assert_eq!(
        compose_origin_quadlet_compose_subjects,
        BTreeSet::from([
            "networks.contract-supabase-backend.ipam.config",
            "services.contract-supabase-auth.healthcheck",
            "services.contract-supabase-db.healthcheck",
            "services.contract-supabase-functions.healthcheck",
            "services.contract-supabase-imgproxy.healthcheck",
            "services.contract-supabase-kong.healthcheck",
            "services.contract-supabase-realtime.healthcheck",
            "services.contract-supabase-rest.healthcheck",
            "services.contract-supabase-storage.healthcheck",
            "services.contract-supabase-studio.healthcheck",
        ]),
        "Compose-origin Quadlet-to-Compose must omit only absent native dependency evidence"
    );
    let compose_origin_quadlet_fidelity = run_supabase_report_contract(
        &root,
        "quadlet",
        "compose",
        "storage",
        SupabaseContractContext {
            podman_acquisition: "not-podman",
            provisioner_mode: "compose",
            include_system_network: false,
        },
        SupabaseContractMode::Fidelity,
        None,
    )?;
    assert!(compose_origin_quadlet_fidelity.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&compose_origin_quadlet_fidelity.stdout)
            .map_err(|error| format!("invalid Compose-origin Quadlet fidelity: {error}"))?,
        serde_json::json!({
            "approximate": 0,
            "unsupported": 10,
            "invalid": 0,
            "other": 0,
        })
    );
    let mut injected_compose_origin_dependency = serde_json::json!({
        "schema_version": 1,
        "status": "success",
        "fidelity": {
            "exact": 0,
            "approximate": 0,
            "unsupported": 10,
            "invalid": 0,
            "other": 0,
        },
        "diagnostics": compose_origin_quadlet_compose
            .iter()
            .map(supabase_contract_diagnostic)
            .collect::<Vec<_>>(),
    });
    injected_compose_origin_dependency["diagnostics"]
        .as_array_mut()
        .ok_or("Compose-origin Quadlet-to-Compose report diagnostics must be an array")?
        .push(serde_json::json!({
            "code": "BFC0007",
            "severity": "warning",
            "name": "injected dependency loss",
            "fields": [
                {
                    "name": "subject",
                    "value": "services.contract-supabase-auth.dependencies[0]",
                },
                {"name": "decision", "value": null},
            ],
        }));
    let rejected_compose_origin_dependency = run_supabase_report_contract(
        &root,
        "quadlet",
        "compose",
        "storage",
        SupabaseContractContext {
            podman_acquisition: "not-podman",
            provisioner_mode: "compose",
            include_system_network: false,
        },
        SupabaseContractMode::Validate,
        Some(&injected_compose_origin_dependency),
    )?;
    assert_eq!(rejected_compose_origin_dependency.status.code(), Some(1));
    assert_eq!(rejected_compose_origin_dependency.stdout, b"false\n");

    let all_compose_podman = run_supabase_report_contract(
        &root,
        "compose",
        "podman",
        "all",
        SupabaseContractContext {
            podman_acquisition: "not-podman",
            provisioner_mode: "cli",
            include_system_network: true,
        },
        SupabaseContractMode::Diagnostics,
        None,
    )?;
    if !all_compose_podman.status.success() {
        return Err(format!(
            "failed to generate all Compose-to-Podman system-network contract: {}",
            String::from_utf8_lossy(&all_compose_podman.stderr).trim()
        ));
    }
    let all_compose_podman: serde_json::Value = serde_json::from_slice(&all_compose_podman.stdout)
        .map_err(|error| format!("invalid all Compose-to-Podman system-network contract: {error}"))?;
    let all_compose_podman = all_compose_podman
        .as_array()
        .ok_or("all Compose-to-Podman diagnostics must be an array")?;
    assert_eq!(all_compose_podman.len(), 213);
    let system_network_losses = all_compose_podman
        .iter()
        .filter(|diagnostic| diagnostic["subject"] == "networks.podman.internal")
        .map(|diagnostic| {
            (
                diagnostic["code"].as_str().unwrap_or_default(),
                diagnostic["severity"].as_str().unwrap_or_default(),
                diagnostic["decision"].as_str().unwrap_or_default(),
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        system_network_losses,
        BTreeSet::from([("BFP0007", "warning", "omitted")])
    );
    let all_compose_podman_report = serde_json::json!({
        "schema_version": 1,
        "status": "success",
        "fidelity": {"exact": 0, "approximate": 0, "unsupported": 213, "invalid": 0, "other": 0},
        "diagnostics": all_compose_podman
            .iter()
            .map(supabase_contract_diagnostic)
            .collect::<Vec<_>>(),
    });
    assert!(supabase_contract_accepts_including_system_network(
        &root,
        "compose",
        "podman",
        "all",
        &all_compose_podman_report,
    )?);
    let mut missing_system_network_loss = all_compose_podman_report.clone();
    missing_system_network_loss["diagnostics"]
        .as_array_mut()
        .ok_or("all Compose-to-Podman report diagnostics must be an array")?
        .retain(|diagnostic| {
            diagnostic["fields"].as_array().is_none_or(|fields| {
                !fields
                    .iter()
                    .any(|field| field["name"] == "subject" && field["value"] == "networks.podman.internal")
            })
        });
    assert!(!supabase_contract_accepts_including_system_network(
        &root,
        "compose",
        "podman",
        "all",
        &missing_system_network_loss,
    )?);

    let all_quadlet_compose = run_supabase_report_contract(
        &root,
        "quadlet",
        "compose",
        "all",
        SupabaseContractContext {
            podman_acquisition: "not-podman",
            provisioner_mode: "cli",
            include_system_network: true,
        },
        SupabaseContractMode::Diagnostics,
        None,
    )?;
    if !all_quadlet_compose.status.success() {
        return Err(format!(
            "failed to generate all Quadlet-to-Compose system-network contract: {}",
            String::from_utf8_lossy(&all_quadlet_compose.stderr).trim()
        ));
    }
    let all_quadlet_compose: serde_json::Value = serde_json::from_slice(&all_quadlet_compose.stdout)
        .map_err(|error| format!("invalid all Quadlet-to-Compose system-network contract: {error}"))?;
    let all_quadlet_compose = all_quadlet_compose
        .as_array()
        .ok_or("all Quadlet-to-Compose diagnostics must be an array")?;
    assert_eq!(all_quadlet_compose.len(), 27);
    let system_network_losses = all_quadlet_compose
        .iter()
        .filter(|diagnostic| diagnostic["subject"] == "networks.podman.ipam.config")
        .map(|diagnostic| {
            (
                diagnostic["code"].as_str().unwrap_or_default(),
                diagnostic["severity"].as_str().unwrap_or_default(),
                diagnostic["decision"].as_str(),
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(system_network_losses, BTreeSet::from([("BFC0007", "warning", None)]));
    let all_quadlet_compose_report = serde_json::json!({
        "schema_version": 1,
        "status": "success",
        "fidelity": {"exact": 0, "approximate": 0, "unsupported": 27, "invalid": 0, "other": 0},
        "diagnostics": all_quadlet_compose
            .iter()
            .map(supabase_contract_diagnostic)
            .collect::<Vec<_>>(),
    });
    assert!(supabase_contract_accepts_including_system_network(
        &root,
        "quadlet",
        "compose",
        "all",
        &all_quadlet_compose_report,
    )?);
    let mut missing_system_network_loss = all_quadlet_compose_report.clone();
    missing_system_network_loss["diagnostics"]
        .as_array_mut()
        .ok_or("all Quadlet-to-Compose report diagnostics must be an array")?
        .retain(|diagnostic| {
            diagnostic["fields"].as_array().is_none_or(|fields| {
                !fields
                    .iter()
                    .any(|field| field["name"] == "subject" && field["value"] == "networks.podman.ipam.config")
            })
        });
    assert!(!supabase_contract_accepts_including_system_network(
        &root,
        "quadlet",
        "compose",
        "all",
        &missing_system_network_loss,
    )?);

    let all_quadlet_podman = run_supabase_report_contract(
        &root,
        "quadlet",
        "podman",
        "all",
        SupabaseContractContext {
            podman_acquisition: "not-podman",
            provisioner_mode: "cli",
            include_system_network: true,
        },
        SupabaseContractMode::Diagnostics,
        None,
    )?;
    if !all_quadlet_podman.status.success() {
        return Err(format!(
            "failed to generate all Quadlet-to-Podman system-network contract: {}",
            String::from_utf8_lossy(&all_quadlet_podman.stderr).trim()
        ));
    }
    let all_quadlet_podman: serde_json::Value = serde_json::from_slice(&all_quadlet_podman.stdout)
        .map_err(|error| format!("invalid all Quadlet-to-Podman system-network contract: {error}"))?;
    let all_quadlet_podman = all_quadlet_podman
        .as_array()
        .ok_or("all Quadlet-to-Podman diagnostics must be an array")?;
    assert_eq!(all_quadlet_podman.len(), 239);
    let system_network_losses = all_quadlet_podman
        .iter()
        .filter(|diagnostic| {
            matches!(
                diagnostic["subject"].as_str(),
                Some("networks.podman.internal" | "networks.podman.ipam_configs")
            )
        })
        .map(|diagnostic| {
            (
                diagnostic["code"].as_str().unwrap_or_default(),
                diagnostic["severity"].as_str().unwrap_or_default(),
                diagnostic["subject"].as_str().unwrap_or_default(),
                diagnostic["decision"].as_str().unwrap_or_default(),
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        system_network_losses,
        BTreeSet::from([
            ("BFP0007", "warning", "networks.podman.internal", "omitted",),
            ("BFP0007", "warning", "networks.podman.ipam_configs", "omitted",),
        ])
    );
    let all_quadlet_podman_report = serde_json::json!({
        "schema_version": 1,
        "status": "success",
        "fidelity": {"exact": 0, "approximate": 0, "unsupported": 239, "invalid": 0, "other": 0},
        "diagnostics": all_quadlet_podman
            .iter()
            .map(supabase_contract_diagnostic)
            .collect::<Vec<_>>(),
    });
    assert!(supabase_contract_accepts_including_system_network(
        &root,
        "quadlet",
        "podman",
        "all",
        &all_quadlet_podman_report,
    )?);
    let mut missing_system_network_loss = all_quadlet_podman_report.clone();
    missing_system_network_loss["diagnostics"]
        .as_array_mut()
        .ok_or("all Quadlet-to-Podman report diagnostics must be an array")?
        .retain(|diagnostic| {
            diagnostic["fields"].as_array().is_none_or(|fields| {
                !fields
                    .iter()
                    .any(|field| field["name"] == "subject" && field["value"] == "networks.podman.ipam_configs")
            })
        });
    assert!(!supabase_contract_accepts_including_system_network(
        &root,
        "quadlet",
        "podman",
        "all",
        &missing_system_network_loss,
    )?);

    let reviewed_fidelity = [
        ("exact", "podman", "compose", 63, 1_346),
        ("exact", "podman", "quadlet", 63, 1_318),
        ("exact", "podman", "podman", 63, 1_527),
        ("exact", "quadlet", "compose", 0, 26),
        ("exact", "compose", "compose", 0, 0),
        ("exact", "compose", "quadlet", 0, 1),
        ("exact", "compose", "podman", 0, 198),
        ("exact", "quadlet", "quadlet", 0, 0),
        ("exact", "quadlet", "podman", 0, 223),
        ("storage", "podman", "compose", 63, 1_346),
        ("storage", "podman", "quadlet", 63, 1_318),
        ("storage", "podman", "podman", 63, 1_527),
        ("storage", "quadlet", "compose", 0, 26),
        ("storage", "compose", "compose", 0, 0),
        ("storage", "compose", "quadlet", 0, 1),
        ("storage", "compose", "podman", 0, 198),
        ("storage", "quadlet", "quadlet", 0, 0),
        ("storage", "quadlet", "podman", 0, 223),
        ("label", "podman", "compose", 63, 1_346),
        ("label", "podman", "quadlet", 63, 1_318),
        ("label", "podman", "podman", 63, 1_527),
        ("label", "quadlet", "compose", 0, 26),
        ("label", "compose", "compose", 0, 0),
        ("label", "compose", "quadlet", 0, 1),
        ("label", "compose", "podman", 0, 198),
        ("label", "quadlet", "quadlet", 0, 0),
        ("label", "quadlet", "podman", 0, 223),
        ("all", "podman", "compose", 67, 1_446),
        ("all", "podman", "quadlet", 67, 1_418),
        ("all", "podman", "podman", 67, 1_641),
        ("all", "quadlet", "compose", 0, 26),
        ("all", "compose", "compose", 0, 0),
        ("all", "compose", "quadlet", 0, 1),
        ("all", "compose", "podman", 0, 212),
        ("all", "quadlet", "quadlet", 0, 0),
        ("all", "quadlet", "podman", 0, 237),
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
    for (selection, approximate, unsupported) in [
        ("exact", 63, 1_308),
        ("storage", 63, 1_308),
        ("label", 63, 1_308),
        ("all", 67, 1_406),
    ] {
        let generated = generated_supabase_contract_with_acquisition(
            &root,
            "podman",
            "compose",
            selection,
            "compose",
            SupabaseContractMode::Fidelity,
        )?;
        assert_eq!(
            generated,
            serde_json::json!({
                "approximate": approximate,
                "unsupported": unsupported,
                "invalid": 0,
                "other": 0,
            }),
            "{selection} Compose-authored Podman-to-Compose fidelity"
        );
    }

    let system_network_fidelity = run_supabase_report_contract(
        &root,
        "podman",
        "compose",
        "all",
        SupabaseContractContext {
            podman_acquisition: "cli",
            provisioner_mode: "cli",
            include_system_network: true,
        },
        SupabaseContractMode::Fidelity,
        None,
    )?;
    assert!(system_network_fidelity.status.success());
    let system_network_fidelity: serde_json::Value = serde_json::from_slice(&system_network_fidelity.stdout)
        .map_err(|error| format!("invalid system-network fidelity contract: {error}"))?;
    assert_eq!(
        system_network_fidelity,
        serde_json::json!({
            "approximate": 69,
            "unsupported": 1_452,
            "invalid": 0,
            "other": 0,
        })
    );
    let compose_authored_system_network_fidelity = run_supabase_report_contract(
        &root,
        "podman",
        "compose",
        "all",
        SupabaseContractContext {
            podman_acquisition: "compose",
            provisioner_mode: "compose",
            include_system_network: true,
        },
        SupabaseContractMode::Fidelity,
        None,
    )?;
    assert!(compose_authored_system_network_fidelity.status.success());
    let compose_authored_system_network_fidelity: serde_json::Value =
        serde_json::from_slice(&compose_authored_system_network_fidelity.stdout)
            .map_err(|error| format!("invalid Compose-authored system-network fidelity contract: {error}"))?;
    assert_eq!(
        compose_authored_system_network_fidelity,
        serde_json::json!({
            "approximate": 69,
            "unsupported": 1_412,
            "invalid": 0,
            "other": 0,
        })
    );
    let system_network_unsupported = system_network_fidelity["unsupported"]
        .as_u64()
        .ok_or("CLI-authored system-network unsupported fidelity must be an integer")?;
    let compose_authored_system_network_unsupported = compose_authored_system_network_fidelity["unsupported"]
        .as_u64()
        .ok_or("Compose-authored system-network unsupported fidelity must be an integer")?;
    assert_eq!(
        system_network_unsupported - compose_authored_system_network_unsupported,
        16 + ((application_creation_evidence_subjects.len() as u64 + 1) * 2),
        "system-network origin delta must retain the shared acquisition accounting"
    );
    let system_network_diagnostics = run_supabase_report_contract(
        &root,
        "podman",
        "compose",
        "all",
        SupabaseContractContext {
            podman_acquisition: "cli",
            provisioner_mode: "cli",
            include_system_network: true,
        },
        SupabaseContractMode::Diagnostics,
        None,
    )?;
    assert!(system_network_diagnostics.status.success());
    let system_network_diagnostics: serde_json::Value = serde_json::from_slice(&system_network_diagnostics.stdout)
        .map_err(|error| format!("invalid system-network diagnostic contract: {error}"))?;
    assert_eq!(
        system_network_diagnostics
            .as_array()
            .ok_or("system-network diagnostic contract must be an array")?
            .iter()
            .filter(|diagnostic| diagnostic["code"] == "BFC0007")
            .count(),
        30,
        "CLI-authored all-selection system-network target losses"
    );
    let compose_authored_system_network_diagnostics = run_supabase_report_contract(
        &root,
        "podman",
        "compose",
        "all",
        SupabaseContractContext {
            podman_acquisition: "compose",
            provisioner_mode: "compose",
            include_system_network: true,
        },
        SupabaseContractMode::Diagnostics,
        None,
    )?;
    assert!(compose_authored_system_network_diagnostics.status.success());
    let compose_authored_system_network_diagnostics: serde_json::Value =
        serde_json::from_slice(&compose_authored_system_network_diagnostics.stdout)
            .map_err(|error| format!("invalid Compose-authored system-network diagnostic contract: {error}"))?;
    assert_eq!(
        compose_authored_system_network_diagnostics
            .as_array()
            .ok_or("Compose-authored system-network diagnostic contract must be an array")?
            .iter()
            .filter(|diagnostic| diagnostic["code"] == "BFC0007")
            .count(),
        14,
        "Compose-authored all-selection system-network target losses"
    );
    let system_network_tuples = system_network_diagnostics
        .as_array()
        .ok_or("system-network diagnostic contract must be an array")?
        .iter()
        .filter(|diagnostic| {
            diagnostic["subject"] == "network:podman"
                || diagnostic["subject"]
                    .as_str()
                    .is_some_and(|subject| subject.starts_with("networks.podman."))
        })
        .map(|diagnostic| {
            (
                diagnostic["code"].as_str().unwrap_or_default(),
                diagnostic["subject"].as_str().unwrap_or_default(),
                diagnostic["decision"].as_str().unwrap_or_default(),
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        system_network_tuples,
        BTreeSet::from([
            ("BFC0007", "networks.podman.ipam.config", ""),
            ("BFP0002", "network:podman", "omitted"),
            ("BFP0002", "networks.podman.driver", "omitted"),
            ("BFP0002", "networks.podman.ipam_driver", "omitted"),
            ("BFP0002", "networks.podman.native_ipv6_enabled", "omitted"),
            ("BFP0003", "networks.podman.internal", "approximated"),
            ("BFP0003", "networks.podman.ipam", "approximated"),
        ])
    );

    let success_expected = run_supabase_report_contract(
        &root,
        "podman",
        "podman",
        "label",
        SupabaseContractContext {
            podman_acquisition: "cli",
            provisioner_mode: "cli",
            include_system_network: false,
        },
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
    let expected_fidelity = run_supabase_report_contract(
        &root,
        "podman",
        "podman",
        "label",
        SupabaseContractContext {
            podman_acquisition: "cli",
            provisioner_mode: "cli",
            include_system_network: false,
        },
        SupabaseContractMode::Fidelity,
        None,
    )?;
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
            "approximate": 63,
            "unsupported": 1_527,
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
    volume_metadata_as_approximate["fidelity"]["approximate"] = serde_json::json!(63);
    volume_metadata_as_approximate["fidelity"]["unsupported"] = serde_json::json!(1_343);
    assert!(!supabase_contract_accepts(
        &root,
        "podman",
        "podman",
        "label",
        &volume_metadata_as_approximate
    )?);

    let mut missing_kong_network_outcome = success_report.clone();
    missing_kong_network_outcome["fidelity"]["approximate"] = serde_json::json!(61);
    assert!(!supabase_contract_accepts(
        &root,
        "podman",
        "podman",
        "label",
        &missing_kong_network_outcome
    )?);

    let mut arbitrary_exact_count = success_report.clone();
    arbitrary_exact_count["fidelity"]["exact"] = serde_json::json!(999);
    // Exact diagnostics and every loss category are independent fixture
    // contracts. The silent exact-success counter is implementation-derived,
    // so only its integer shape is stable semantic evidence.
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
        "release_outer \"${apply_target_outer}\"",
        "prepare_outer_storage \"${outer}\" \"${image}\"",
        "--image-volume=ignore",
        "verify_outer_storage_volume_ownership",
        "volume rm -- \"${volume}\"",
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
        "compose-provider.sh",
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
            "if ! release_outer \"${outer}\"; then",
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
        "legacy_default_network_api_for_observation() {",
        "local rootless=${2:?caller must supply rootless state}",
        "local root_mode=${4:?caller must supply root mode}",
        "3.0.1:false) printf '%s\\n' 3.0.0 ;;",
        "3.4.4:false | 3.4.4:true) printf '%s\\n' 3.4.4 ;;",
        "local legacy_api_version=",
        "legacy_default_network_api_for_observation \"${version}\" \"${rootless}\"",
        "local root_mode=rootful",
        "root_mode=rootless",
        "\"${report}\" \"${version}\" \"${legacy_api_version}\" \"${root_mode}\"",
        "[[ \"${present}\" == true && -n \"${legacy_api_version}\" ]]",
        ".code == \"BFP0002\"",
        ".source_code == \"PLN0023\"",
        ".name == \"subject\" and .value == \"network:podman\"",
        "PodmanLens found native response fields without typed portable mappings; path descriptors were retained without values",
        ".name == \"decision\" and .value == \"omitted\"",
        "--arg version \"${version}\" --arg api_version \"${api_version}\"",
        ".name == \"source_engine\" and .value == $version",
        ".name == \"source_api\" and .value == $api_version",
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
        .current_dir(root)
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
        "OBSERVABILITY_PROVIDER_VERSION=\"${BOXFERRY_COMPOSE_PROVIDER_VERSION}\"",
        "scrape_timeout  = \"1s\"",
        "observability_validate_alloy_scrape_timing",
        "validate /etc/alloy/config.alloy",
        "observability_pipeline_roles_running",
        "observability_report_pipeline_states",
        "OBSERVABILITY_PROVIDER_SHA256=\"${BOXFERRY_COMPOSE_PROVIDER_SHA256}\"",
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
        "field(\"decision\"; ($diagnostic.code | startswith(\"BFP\")))",
        "field(\"required_loss_policy\"; ($diagnostic.code | startswith(\"BFP\")))",
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
        "SUPABASE_CELL_KILL_AFTER=\"10s\"",
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
                .split_once("\nsupabase_timed_in_shell_operation() {")
                .map(|(cell, _)| cell)
        })
        .ok_or("Supabase numbered cell body could not be isolated")?;
    let progress_checks = cell.matches("progress_run '").count();
    if progress_checks != 40 {
        return Err(format!(
            "Supabase application cell must contain 40 numbered checks, found {progress_checks}"
        ));
    }
    for contract in [
        "python3 \"${script_directory}/lib/in-shell-deadline.py\"",
        "--kill-after \"${SUPABASE_CELL_KILL_AFTER}\"",
        "supabase_timed_in_shell_operation \"${SUPABASE_CELL_TIMEOUT}\"",
        "supabase_run_application_cell_unbounded \"$@\"",
    ] {
        if !runner.contains(contract) {
            return Err(format!("Supabase in-shell deadline contract is missing `{contract}"));
        }
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
        "docker.io/postgrest/postgrest:v16.3@sha256:ec0e25a4e24b0a3bc5e4f011369bfc736bd1b19f513bd01079b86329a7636962",
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
        "SUPABASE_PROVIDER_VERSION=\"${BOXFERRY_COMPOSE_PROVIDER_VERSION}\"",
        "SUPABASE_PROVIDER_SHA256=\"${BOXFERRY_COMPOSE_PROVIDER_SHA256}\"",
        "compose-provider.sh",
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
        "Realtime WebSocket connection failed",
        "Realtime PostgreSQL subscription",
        "message.payload?.extension === \"postgres_changes\"",
        "supabase_assert_realtime_output_aliases",
        "awk -F= '$1 == \"NetworkAlias\"",
        "local output=$1 directory=$2 prefix=$3 provisioner_mode=$4",
        "expected_aliases=(\"realtime-dev.supabase-realtime\" \"realtime\")",
        "local provisioner_mode=${5:-cli}",
        "local provisioner_mode=${8:-cli}",
        "\"${provisioner_mode}\" == cli && \"${output}\" == quadlet",
        "\"${image_origin}\" \"${require_dependency_order}\" \"${provisioner_mode}\"",
        "local provisioner_mode=$9",
        "require_dependency_order=\"$(supabase_direct_export_dependency_order_required \"${mode}\")\" || return",
        "\"${include_system_network}\" podman \"${require_dependency_order}\" \"${mode}\"",
        "expected_podman_environment_fields",
        "KONG_NGINX_PROXY_PROXY_BUFFER_SIZE",
        "KONG_NGINX_PROXY_PROXY_BUFFERS",
        "--arg provisioner_mode \"${provisioner_mode}\"",
        "\"${include_system_network}\" \"${mode}\" \"${mode}\"",
        "\"${include_system_network}\" not-podman \"${mode}\"",
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
        ".[0].Config.CreateCommand as $command",
        "length >= 2 and length <= 128",
        "$command[0] == \"podman\" or ($command[0] | endswith(\"/podman\"))",
        "$command[1] == \"run\"",
        "$command[1] == \"--url\"",
        "startswith(\"unix://\") and length > (\"unix://\" | length)",
        "$command[3] == \"run\"",
        "has(\"CreateCommand\") | not",
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

    let direct_export_dependency_order = runner
        .split_once("supabase_direct_export_dependency_order_required() {")
        .and_then(|(_, following)| {
            following
                .split_once("\nsupabase_run_exports() {")
                .map(|(helper, _)| helper)
        })
        .ok_or("Supabase direct-export dependency-order helper could not be isolated")?;
    for contract in [
        "case $1 in",
        "cli)",
        "# The native CLI provisioner records Podman Dependencies evidence.",
        "printf '%s\\n' true",
        "compose)",
        "# Compose-created containers do not retain Podman Dependencies evidence.",
        "printf '%s\\n' false",
        "*)",
        "Unsupported Supabase direct-export provisioner mode: %s.\\n",
        "return 2",
    ] {
        if !direct_export_dependency_order.contains(contract) {
            return Err(format!(
                "Supabase direct-export dependency-order helper missing `{contract}`"
            ));
        }
    }

    let reimport_dependency_order = runner
        .split_once("supabase_reimport_dependency_order_required() {")
        .and_then(|(_, following)| {
            following
                .split_once("\nsupabase_run_reimports() {")
                .map(|(helper, _)| helper)
        })
        .ok_or("Supabase reimport dependency-order helper could not be isolated")?;
    for contract in [
        "case \"$1:$2\" in",
        "compose:cli | compose:compose)",
        "quadlet:cli)",
        "CLI acquisition retained native Podman Dependencies in the Quadlet artifact.",
        "quadlet:compose)",
        "Compose labels are authored graph evidence, not Podman Dependencies.",
        "Unsupported Supabase reimport dependency source/provisioner: %s/%s.\\n",
    ] {
        if !reimport_dependency_order.contains(contract) {
            return Err(format!(
                "Supabase reimport dependency-order helper missing `{contract}`"
            ));
        }
    }
    let reimport_invocation = "supabase_reimport_dependency_order_required \"${input}\" \"${mode}\"";
    if reimport_dependency_order.contains(reimport_invocation) || !runner.contains(reimport_invocation) {
        return Err("Supabase reimport dependency-order helper invocation is not isolated".to_owned());
    }

    let fixture_contract =
        fs::read_to_string(repository_root().join("fixtures/conformance/supabase-application/success-contract.jq"))
            .map_err(|error| format!("failed read Supabase success contract: {error}"))?;
    for contract in [
        "def quadlet_dependency_intent_retained:",
        "$input == \"quadlet\" and $provisioner_mode == \"cli\";",
        "if quadlet_dependency_intent_retained then compose_diagnostics else [] end",
        "(if quadlet_dependency_intent_retained then compose_outcomes else [] end)",
        "def quadlet_dependency_diagnostics:",
        "if quadlet_dependency_intent_retained then",
    ] {
        if !fixture_contract.contains(contract) {
            return Err(format!("Supabase Quadlet provenance contract missing `{contract}`"));
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
        "generated_podman_diagnostics(false)",
        "generated_podman_diagnostics(true) + quadlet_dependency_diagnostics",
        "supabase_assert_output_semantics",
        ".fidelity.unsupported == $fidelity.unsupported",
        ".fidelity.invalid == $fidelity.invalid",
        "expected_success_fidelity",
        "exact_fidelity_shape",
        "length(seen) != 9 || success != 9 || rejected != 0",
    ] {
        if !runner.contains(fixture_contract) {
            return Err(format!(
                "Supabase fixture and route contract is missing `{fixture_contract}`"
            ));
        }
    }

    for route in [
        "podman\tcompose\tmigration-success\tlive-unperformed\tBFP0002,BFP0003,BFC0007\texact-diagnostic-tuple-multiset-plus-loss-fidelity-v1",
        "podman\tquadlet\tmigration-success\tlive-unperformed\tBFP0002,BFP0003,BFQ0003\texact-diagnostic-tuple-multiset-plus-loss-fidelity-v1",
        "podman\tpodman\tmigration-success\tlive-unperformed\tBFP0002,BFP0003,BFP0007\texact-diagnostic-tuple-multiset-plus-loss-fidelity-v1",
        "compose\tcompose\tmigration-success\tlive-unperformed\t-\tzero-loss-zero-diagnostic-reimport",
        "compose\tquadlet\tmigration-success\tlive-unperformed\tBFQ0003\texact-diagnostic-tuple-multiset-plus-loss-fidelity-v1",
        "quadlet\tcompose\tmigration-success\tlive-unperformed\tBFC0007\texact-diagnostic-tuple-multiset-plus-loss-fidelity-v1",
        "quadlet\tquadlet\tmigration-success\tlive-unperformed\t-\tzero-loss-zero-diagnostic-reimport",
        "compose\tpodman\tmigration-success\tlive-unperformed\tBFP0007\texact-diagnostic-tuple-multiset-plus-loss-fidelity-v1",
        "quadlet\tpodman\tmigration-success\tlive-unperformed\tBFP0007\texact-diagnostic-tuple-multiset-plus-loss-fidelity-v1",
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
        "uses: actions/upload-artifact@",
        "uses: actions/download-artifact@",
        "chmod +x target/debug/boxferry",
        "application:",
        "name: Nextcloud application / podman-6.1-rootless",
        "timeout-minutes: 60",
        "source scripts/lib/compose-provider.sh",
        "boxferry_install_compose_provider target/tools/docker-compose",
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
    validate_live_cleanup_workflow(hosted)?;
    Ok(())
}

fn validate_live_cleanup_workflow(hosted: &str) -> Result<(), String> {
    let cleanup_job = hosted
        .split("\n  cleanup-regression:\n")
        .nth(1)
        .and_then(|remainder| remainder.split("\n  application:\n").next())
        .ok_or("hosted cleanup job must precede the application job")?;
    if !cleanup_job.contains("run: chmod +x target/debug/boxferry") {
        return Err("hosted cleanup job must restore downloaded binary permissions".to_owned());
    }
    for required in [
        "- cleanup-regression",
        "expected_sha:",
        "admit-cleanup:",
        "[[ \"${GITHUB_REPOSITORY}\" == Strukturpiloten/boxferry ]]",
        "[[ \"${EXPECTED_SHA}\" == \"${GITHUB_SHA}\" ]]",
        "needs.matrix.outputs.profile != 'cleanup-regression'",
        "needs: [admit-cleanup, build-boxferry]",
        "if: github.event_name == 'workflow_dispatch' && inputs.profile == 'cleanup-regression'",
        "ref: ${{ github.sha }}",
        "[[ \"$(git rev-parse HEAD)\" == \"${EXPECTED_SHA}\" ]]",
        "bash scripts/test-podman-live-outer-storage-hosted.sh",
    ] {
        if !hosted.contains(required) {
            return Err(format!("hosted cleanup workflow is missing `{required}`"));
        }
    }
    let probe = fs::read_to_string(repository_root().join("scripts/test-podman-live-outer-storage-hosted.sh"))
        .map_err(|error| format!("failed to read hosted storage probe: {error}"))?;
    for required in [
        "fixtures/conformance/podman-live/matrix.tsv",
        "for iteration in 1 2; do",
        "--profile smoke --matrix-cell \"${cell}\" --engine podman",
        "sudo podman volume ls",
        "test-podman-live-outer-storage-native.sh",
    ] {
        if !probe.contains(required) {
            return Err(format!("hosted storage probe is missing `{required}`"));
        }
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
                "BoxFerry: Format and lint only (no tests)",
                "scripts/format-lint.sh",
                "\"args\": [\"--fix\"]",
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
fn lightweight_format_lint_task_is_bounded_and_executes_no_tests() -> Result<(), String> {
    let root = repository_root();
    let script = fs::read_to_string(root.join("scripts/format-lint.sh"))
        .map_err(|error| format!("failed to read format/lint runner: {error}"))?;
    for required in [
        "--check|--fix",
        "BOXFERRY_LINT_JOBS",
        "cargo fmt --all",
        "scripts/check-files.sh",
        "git --no-pager diff --check",
        "git --no-pager diff --cached --check",
        "actionlint",
        "zizmor .github/workflows",
        "cargo ci-clippy",
        "no tests were executed",
    ] {
        if !script.contains(required) {
            return Err(format!("format/lint runner is missing `{required}`"));
        }
    }
    for forbidden in [
        "cargo test",
        "cargo ci-test",
        "migration-readiness.py run",
        "llvm-cov",
        "cargo semver-checks",
        "podman-live-conformance",
    ] {
        if script.contains(forbidden) {
            return Err(format!("format/lint runner must not dispatch `{forbidden}`"));
        }
    }
    for document in [
        "README.md",
        "AGENTS.md",
        "docs/testing.md",
        "docs/development-environment.md",
        "docs/public/development/testing/index.md",
        "docs/public/development/contributing/index.md",
    ] {
        let text =
            fs::read_to_string(root.join(document)).map_err(|error| format!("failed to read {document}: {error}"))?;
        if !text.contains("scripts/format-lint.sh")
            || !(text.contains("no tests")
                || text.contains("without tests")
                || text.contains("without executing any tests"))
        {
            return Err(format!("{document} must explain the no-tests format/lint task"));
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
                "./scripts/check-all.sh",
                "hard gate against commit, push",
                "primary agent runs this workflow",
                "GPT-6 Sol with `xhigh` reasoning",
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
                "selected GitHub checks and fail-closed `PR gate`",
                "primary agent uses GPT-6 Sol with `xhigh` reasoning",
                "Worker agents",
                "never perform Git or GitHub writes",
                "the primary agent's final responsibility",
            ][..],
        ),
    ] {
        let contents =
            fs::read_to_string(root.join(path)).map_err(|error| format!("failed to read {path}: {error}"))?;
        let flattened = contents.split_whitespace().collect::<Vec<_>>().join(" ");
        for value in required {
            if !contents.contains(value) && !flattened.contains(value) {
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
    for required in [
        "npm ci --ignore-scripts",
        "bash scripts/install-file-tools.sh /usr/local/bin",
        "bash scripts/check-files.sh --check",
    ] {
        if !ci.contains(required) {
            return Err(format!(
                "{} must enforce non-Rust file contract `{required}`",
                ci_path.display()
            ));
        }
    }

    let release_path = root.join(".github/workflows/release.yml");
    let release = fs::read_to_string(&release_path)
        .map_err(|error| format!("failed to read {}: {error}", release_path.display()))?;
    if !release.contains("uses: ./.github/workflows/ci.yml") {
        return Err("Release must consume the canonical reusable non-Rust file checks".to_owned());
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
        "\"name\": \"DockerLens\"",
        "\"path\": \".boxferry-workspace/docker-lens\"",
        "\"name\": \"PodmanLens\"",
        "\"path\": \".boxferry-workspace/podman-lens\"",
        "\"name\": \"QuadletLens\"",
        "\"path\": \".boxferry-workspace/quadlet-lens\"",
        "\"label\": \"Workspace: Format, lint, and test all repositories\"",
        "\"dependsOrder\": \"sequence\"",
        "\"label\": \"Workspace: Format and lint BoxFerry only (no tests)\"",
        "${workspaceFolder:BoxFerry}/scripts/format-lint.sh",
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
        "source=${localWorkspaceFolder}/../docker-lens,target=/workspaces/boxferry/.boxferry-workspace/docker-lens,type=bind",
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
                "\"path\": \".boxferry-workspace/docker-lens\"",
                "\"path\": \".boxferry-workspace/podman-lens\"",
                "\"path\": \".boxferry-workspace/quadlet-lens\"",
            ],
        ),
        (
            "Dev Container sibling mounts",
            devcontainer.as_str(),
            [
                "../compose-lens,target=/workspaces/boxferry/.boxferry-workspace/compose-lens",
                "../docker-lens,target=/workspaces/boxferry/.boxferry-workspace/docker-lens",
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
            return Err(format!(
                "{label} must order ComposeLens, DockerLens, PodmanLens, and QuadletLens"
            ));
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
        "for repository in compose-lens docker-lens podman-lens quadlet-lens boxferry-website; do",
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
fn ci_workflow_enforces_coverage_linux_only_and_pr_gate_contract() -> Result<(), String> {
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
        "  validation-plan:\n    name: Validation plan",
        "--policy-root .validation-base",
        "First rollout: never execute a PR-head classifier without trusted base policy.",
        "First-rollout complete validation: every required job succeeded",
        "PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-validation-plan.py",
        "cargo test --locked --package boxferry --all-features --test documentation_examples",
        "  coverage:\n    name: Coverage ratchet",
        "rustup component add llvm-tools-preview",
        "cargo llvm-cov clean --locked",
        "cargo llvm-cov --locked --no-clean --workspace --all-features --all-targets --summary-only\n          --fail-under-regions 82 --fail-under-functions 87 --fail-under-lines 82",
        "run: cargo ci-check",
        "run: cargo ci-test",
        "  release-metadata:\n    name: Release metadata and changelog",
        "run: bash scripts/validate-release-metadata.sh",
        "  pr-gate:\n    name: PR gate\n    if: always()",
        "needs:\n      [\n        validation-plan,\n        rust,\n        msrv,\n        dependencies,\n        documentation,\n        release-metadata,\n        semver-release-type,\n        semver,\n        coverage,\n        migration-readiness,\n        lockfile-release-age,\n      ]",
        "PLAN_JSON: ${{ needs.validation-plan.outputs.plan }}",
        "NEEDS_JSON: ${{ toJSON(needs) }}",
    ] {
        if !workflow.contains(required) {
            return Err(format!("CI workflow is missing contract `{required}`"));
        }
    }

    for selector in [
        "rust",
        "msrv",
        "dependencies",
        "documentation",
        "release_metadata",
        "semver_release_type",
        "semver",
        "coverage",
        "migration_readiness",
        "lockfile_release_age",
    ] {
        let output = format!("select_{selector}: ${{{{ steps.plan.outputs.select_{selector} }}}}");
        let condition = format!("if: needs.validation-plan.outputs.select_{selector} == 'true'");
        if !workflow.contains(&output) || !workflow.contains(&condition) {
            return Err(format!("CI is missing fail-closed plan wiring for `{selector}`"));
        }
    }
    let planner = fs::read_to_string(repository_root().join("scripts/validation-plan.py"))
        .map_err(|error| format!("failed to read validation planner: {error}"))?;
    for required in [
        "if jobs[job] and result != \"success\"",
        "elif not jobs[job] and result not in {\"skipped\", \"success\"}",
        "validation plan is not bound to the exact PR comparison",
        "non-PR validation must be complete at the tested revision",
    ] {
        if !planner.contains(required) {
            return Err(format!("validation gate is missing `{required}`"));
        }
    }
    if workflow.contains("windows-") {
        return Err("CI must not claim unsupported native Windows portability".to_owned());
    }
    if workflow.contains("macos-") {
        return Err("CI validation must not require a macOS runner".to_owned());
    }

    Ok(())
}

#[test]
fn release_workflow_reuses_the_complete_ci_contract() -> Result<(), String> {
    let root = repository_root();
    let ci_path = root.join(".github/workflows/ci.yml");
    let ci = fs::read_to_string(&ci_path).map_err(|error| format!("failed to read {}: {error}", ci_path.display()))?;
    let release_path = root.join(".github/workflows/release.yml");
    let release = fs::read_to_string(&release_path)
        .map_err(|error| format!("failed to read {}: {error}", release_path.display()))?;

    for required in [
        "workflow_call:",
        "  validation-plan:\n    name: Validation plan",
        "python3 scripts/validation-plan.py plan --event \"${EVENT_NAME}\"",
        "  coverage:\n    name: Coverage ratchet",
        "lycheeverse/lychee-action@",
        "  msrv:\n    name: MSRV",
        "  dependencies:\n    name: Dependency and license policy",
        "  semver:\n    name: SemVer (${{ matrix.package }})",
        "  migration-readiness:\n    name: Offline migration readiness",
        "name: migration-readiness-offline-${{ github.sha }}-${{ github.run_id }}",
        "overwrite: true",
        "  pr-gate:\n    name: PR gate\n    if: always()",
    ] {
        if !ci.contains(required) {
            return Err(format!("reusable CI workflow missing complete gate `{required}`"));
        }
    }

    for required in [
        "  deterministic:\n    name: Complete deterministic validation",
        "permissions:\n      contents: read\n    uses: ./.github/workflows/ci.yml",
        "  release-metadata:\n    name: Validate release metadata",
        "bash scripts/validate-release-metadata.sh",
        "needs: [deterministic, release-metadata, migration-readiness]",
        "needs: [deterministic, release-metadata, migration-readiness, migration-readiness-evidence]",
        "needs: [deterministic, release-metadata, migration-readiness-evidence, release-validation]",
    ] {
        if !release.contains(required) {
            return Err(format!("release workflow missing shared validation guard `{required}`"));
        }
    }
    for duplicated in [
        "cargo ci-test",
        "cargo llvm-cov",
        "runs-on: macos-14",
        "cargo \"+${RUST_MSRV}\" ci-check",
    ] {
        if release.contains(duplicated) {
            return Err(format!(
                "release workflow duplicates reusable deterministic task `{duplicated}`"
            ));
        }
    }
    Ok(())
}

#[test]
fn release_uses_fresh_reusable_readiness_evidence_and_validation_only_mode() -> Result<(), String> {
    let root = repository_root();
    let release_path = root.join(".github/workflows/release.yml");
    let release = fs::read_to_string(&release_path)
        .map_err(|error| format!("failed to read {}: {error}", release_path.display()))?;
    let readiness_path = root.join(".github/workflows/migration-readiness.yml");
    let readiness = fs::read_to_string(&readiness_path)
        .map_err(|error| format!("failed to read {}: {error}", readiness_path.display()))?;

    for required in [
        "validation_only:",
        "type: boolean",
        "uses: ./.github/workflows/migration-readiness.yml",
        "tier: pre-release",
        "permissions:\n      contents: read\n    uses: ./.github/workflows/migration-readiness.yml",
        "needs: [deterministic, release-metadata, migration-readiness]",
        "always() && github.repository == 'Strukturpiloten/boxferry'",
        "name: ${{ needs.migration-readiness.outputs.evidence_artifact }}",
        "actions/download-artifact@",
        "--require-aggregate",
        "--require-success",
        "release-validation:",
        "needs: [deterministic, release-metadata, migration-readiness, migration-readiness-evidence]",
        "if: always()",
        "permissions: {}",
        "DETERMINISTIC_RESULT: ${{ needs.deterministic.result }}",
        "RELEASE_METADATA_RESULT: ${{ needs.release-metadata.result }}",
        "READINESS_RESULT: ${{ needs.migration-readiness.result }}",
        "EVIDENCE_RESULT: ${{ needs.migration-readiness-evidence.result }}",
        "for result in \"${DETERMINISTIC_RESULT}\" \"${RELEASE_METADATA_RESULT}\" \\\n            \"${READINESS_RESULT}\" \"${EVIDENCE_RESULT}\"; do",
        "if [[ \"${result}\" != success ]]",
        "needs: [deterministic, release-metadata, migration-readiness-evidence, release-validation]",
        "!inputs.validation_only",
        "name: release-migration-readiness-${{ github.sha }}-${{ github.run_id }}",
        "overwrite: true",
    ] {
        if !release.contains(required) {
            return Err(format!("release workflow missing fresh-evidence contract `{required}`"));
        }
    }
    for forbidden in ["gh run list", "gh run download", "--workflow migration-readiness.yml"] {
        if release.contains(forbidden) {
            return Err(format!("release workflow must not search a prior run: `{forbidden}`"));
        }
    }

    for required in [
        "evidence_artifact:",
        "value: ${{ jobs.collect.outputs.artifact }}",
        "migration-readiness-boxferry-%s-%s",
        "migration-readiness-worker-${{ github.sha }}-${{ github.run_id }}-${{ matrix.task }}",
        "pattern: migration-readiness-worker-${{ github.sha }}-${{ github.run_id }}-*",
        "migration-readiness-pre-release-%s-%s",
        "overwrite: true",
    ] {
        if !readiness.contains(required) {
            return Err(format!("migration readiness missing retry-safe binding `{required}`"));
        }
    }
    let coordinator_argument = readiness
        .find("--coordinator-id \"${COORDINATOR_ID}\"")
        .ok_or_else(|| "coordinated readiness worker argument is missing".to_owned())?;
    let attempt_forwarding = readiness
        .find("GITHUB_RUN_ATTEMPT=\"${GITHUB_RUN_ATTEMPT}\"")
        .ok_or_else(|| "privileged readiness workers must retain the GitHub run attempt across sudo".to_owned())?;
    if readiness
        .matches("GITHUB_RUN_ATTEMPT=\"${GITHUB_RUN_ATTEMPT}\"")
        .count()
        != 1
        || attempt_forwarding < coordinator_argument
    {
        return Err("privileged readiness workers must retain the GitHub run attempt across sudo".to_owned());
    }
    if readiness.matches("overwrite: true").count() < 4 {
        return Err("every rerunnable migration-readiness producer must replace its artifact".into());
    }

    let validation = release
        .find("  release-validation:\n")
        .ok_or("release workflow must contain an explicit validation aggregate")?;
    let publish = release
        .find("  publish:\n")
        .ok_or("release workflow must contain publication")?;
    if validation >= publish {
        return Err("release validation aggregate must precede publication".to_owned());
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
            "  release-metadata:\n    name: Release metadata and changelog\n    needs: validation-plan\n    if: needs.validation-plan.outputs.select_release_metadata == 'true'\n    runs-on: ubuntu-24.04\n    timeout-minutes: 5\n    steps:\n      - name: Check out repository with release history",
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
        "Such compilation is incidental unless that platform appears in the supported Linux CI",
        "no macOS runner",
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
fn public_api_compatibility_has_one_ci_owner_and_release_consumer() -> Result<(), String> {
    const ACTION: &str = "uses: obi1kenobi/cargo-semver-checks-action@";
    let root = repository_root();
    let ci_path = root.join(".github/workflows/ci.yml");
    let ci = fs::read_to_string(&ci_path).map_err(|error| format!("failed to read {}: {error}", ci_path.display()))?;
    if ci.matches(ACTION).count() != 1 {
        return Err("canonical CI must invoke cargo-semver-checks exactly once".to_owned());
    }
    for required in ["package: ${{ matrix.package }}", "feature-group: all-features"] {
        if !ci.contains(required) {
            return Err(format!("canonical CI SemVer job missing `{required}`"));
        }
    }
    for package in PUBLISHED_PACKAGES {
        if !ci.contains(&format!("          - {package}")) {
            return Err(format!("canonical CI SemVer matrix missing {package}"));
        }
    }

    let release_path = root.join(".github/workflows/release.yml");
    let release = fs::read_to_string(&release_path)
        .map_err(|error| format!("failed to read {}: {error}", release_path.display()))?;
    if !release.contains("uses: ./.github/workflows/ci.yml") {
        return Err("Release must consume canonical CI SemVer validation".to_owned());
    }
    if release.contains(ACTION) {
        return Err("Release must not duplicate the canonical SemVer action".to_owned());
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
        "needs: [validation-plan, semver-release-type]",
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
        "version: \"",
        "uses: release-plz/action@",
        "uses: actions/create-github-app-token@",
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
fn yoke_derive_msrv_exclusion_is_owned_by_cargo_and_renovate() -> Result<(), String> {
    let root = repository_root();
    let manifest_path = root.join("crates/boxferry-podman/Cargo.toml");
    let manifest = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("failed to read {}: {error}", manifest_path.display()))?;
    let manifest: toml::Value =
        toml::from_str(&manifest).map_err(|error| format!("failed to parse {}: {error}", manifest_path.display()))?;
    if manifest["dev-dependencies"]["yoke-derive"].as_str() != Some("=0.8.2") {
        return Err("boxferry-podman must constrain yoke-derive to the Rust 1.85-compatible release".to_owned());
    }

    let renovate_path = root.join(".github/renovate.json");
    let renovate = fs::read_to_string(&renovate_path)
        .map_err(|error| format!("failed to read {}: {error}", renovate_path.display()))?;
    let renovate: serde_json::Value = serde_json::from_str(&renovate)
        .map_err(|error| format!("failed to parse {}: {error}", renovate_path.display()))?;
    let package_rules = renovate["packageRules"]
        .as_array()
        .ok_or_else(|| "Renovate packageRules must be an array".to_owned())?;
    let exclusion = package_rules.iter().find(|rule| {
        rule["matchManagers"]
            .as_array()
            .is_some_and(|managers| managers.iter().any(|manager| manager == "cargo"))
            && rule["matchPackageNames"]
                .as_array()
                .is_some_and(|packages| packages.iter().any(|package| package == "yoke-derive"))
    });
    if exclusion.and_then(|rule| rule["allowedVersions"].as_str()) != Some(r"!/^(0\.8\.3)$/") {
        return Err("Renovate must exclude the yoke-derive release known to fail on Rust 1.85".to_owned());
    }

    let policy = fs::read_to_string(root.join("docs/dependency-policy.md"))
        .map_err(|error| format!("failed to read dependency policy: {error}"))?;
    for required in [
        "exact development-only `yoke-derive` constraint",
        "Remove both constraints only after a newer upstream release passes the unchanged MSRV gate",
    ] {
        if !policy.contains(required) {
            return Err(format!("dependency policy missing `{required}`"));
        }
    }

    Ok(())
}

fn validate_renovate_lock_maintenance(
    renovate: &serde_json::Value,
    package_rules: &[serde_json::Value],
) -> Result<(), String> {
    if renovate["minimumReleaseAge"] != "3 days" {
        return Err("Renovate direct updates must retain the three-day minimum release age".to_owned());
    }

    let generic_matches = package_rules
        .iter()
        .enumerate()
        .filter(|(_, rule)| rule["description"] == "Automerge tested non-major dependency updates")
        .collect::<Vec<_>>();
    if generic_matches.len() != 1 {
        return Err("Renovate must define exactly one generic non-major automerge rule".to_owned());
    }
    let (generic_position, generic_rule) = generic_matches[0];
    if generic_rule["matchUpdateTypes"] != serde_json::json!(["minor", "patch", "pin", "digest", "pinDigest"])
        || generic_rule["automerge"] != true
        || generic_rule["automergeType"] != "pr"
        || generic_rule["platformAutomerge"] != false
    {
        return Err("Renovate generic automerge must cover only tested non-major updates".to_owned());
    }

    let lock_matches = package_rules
        .iter()
        .enumerate()
        .filter(|(_, rule)| rule["description"] == "Automerge green-gated lock-file maintenance")
        .collect::<Vec<_>>();
    if lock_matches.len() != 1 {
        return Err("Renovate must define exactly one lock-file maintenance rule".to_owned());
    }
    let (lock_position, lock_rule) = lock_matches[0];
    if lock_rule["matchUpdateTypes"] != serde_json::json!(["lockFileMaintenance"])
        || lock_rule["minimumReleaseAge"] != "0 days"
        || lock_rule["automerge"] != true
        || lock_rule["automergeType"] != "pr"
        || lock_rule["platformAutomerge"] != false
        || lock_position <= generic_position
    {
        return Err(
            "Renovate lock-file maintenance must follow generic automerge and rely on the shared PR guard".to_owned(),
        );
    }

    for description in [
        "Review GitHub-hosted runner environment upgrades manually",
        "Keep Dev Container feature versions current; the lock file owns digests",
        "Require explicit revalidation for live application and workload images",
        "Require checksum review for the Docker Compose provider",
        "Require checksum review for downloaded file-quality tools",
    ] {
        let (position, rule) = package_rules
            .iter()
            .enumerate()
            .find(|(_, rule)| rule["description"] == description)
            .ok_or_else(|| format!("Renovate is missing sensitive manual rule `{description}`"))?;
        if position <= generic_position || position <= lock_position || rule["automerge"] != false {
            return Err(format!(
                "Renovate sensitive rule `{description}` must follow both automerge rules and disable it explicitly"
            ));
        }
    }

    Ok(())
}

fn validate_shared_lockfile_guard(renovate: &serde_json::Value, workflow: &str) -> Result<(), String> {
    let shared_managers = renovate["customManagers"]
        .as_array()
        .ok_or_else(|| "Renovate customManagers must be an array".to_owned())?
        .iter()
        .filter(|manager| manager["description"] == "Track the immutable Strukturpiloten shared-policy commit")
        .collect::<Vec<_>>();
    if shared_managers.len() != 1 {
        return Err("Renovate must own the shared-policy commit exactly once".to_owned());
    }
    let manager = shared_managers[0];
    if manager["datasourceTemplate"] != "github-digest" {
        return Err("Renovate shared-policy manager must declare the github-digest datasource".to_owned());
    }
    let manager_pattern = manager["matchStrings"]
        .as_array()
        .and_then(|patterns| (patterns.len() == 1).then(|| patterns[0].as_str()).flatten())
        .ok_or_else(|| "shared-policy Renovate manager must have one match expression".to_owned())?;
    let replacement_template = manager["autoReplaceStringTemplate"]
        .as_str()
        .ok_or_else(|| "shared-policy Renovate manager must have a replacement template".to_owned())?;
    if manager["customType"] != "regex"
        || manager["managerFilePatterns"] != serde_json::json!([r"/^\.github/workflows/.*\.ya?ml$/"])
        || !manager_pattern.contains("datasource=github-digest")
        || !manager_pattern.contains("currentValue=(?<currentValue>main)")
        || !manager_pattern.contains("currentDigest>[a-f0-9]{40}")
        || !replacement_template.contains("ref: {{{newDigest}}}")
    {
        return Err("Renovate shared-policy manager must update one immutable GitHub digest".to_owned());
    }

    let marker = "# renovate: datasource=github-digest depName=Strukturpiloten/.github currentValue=main";
    if !replacement_template.contains('\n') || replacement_template.contains(r"\n") {
        return Err("Renovate shared-policy replacement must contain a real YAML line break".to_owned());
    }
    let replacement_digest = "0123456789abcdef0123456789abcdef01234567";
    let rendered_replacement = replacement_template
        .replace("{{{indentation}}}", "        ")
        .replace("{{{depName}}}", "Strukturpiloten/.github")
        .replace("{{{newValue}}}", "main")
        .replace("{{{newDigest}}}", replacement_digest);
    let rendered_lines = rendered_replacement.lines().collect::<Vec<_>>();
    if rendered_lines.len() != 2
        || rendered_lines[0].trim() != marker
        || rendered_lines[1].trim() != format!("ref: {replacement_digest}")
        || !manager_pattern.contains("(?<indentation>[ \\t]*)")
    {
        return Err("Renovate shared-policy replacement must re-extract as one adjacent marker/ref pair".to_owned());
    }
    if workflow.matches(marker).count() != 1 {
        return Err("CI must contain exactly one Renovate-owned shared-policy ref".to_owned());
    }
    let lines = workflow.lines().collect::<Vec<_>>();
    let marker_line = lines
        .iter()
        .position(|line| line.trim() == marker)
        .ok_or_else(|| "CI shared-policy Renovate marker is missing".to_owned())?;
    let shared_ref = lines
        .get(marker_line + 1)
        .and_then(|line| line.trim().strip_prefix("ref: "))
        .ok_or_else(|| "CI shared-policy marker must be adjacent to its ref".to_owned())?;
    if shared_ref.len() != 40
        || !shared_ref
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err("CI shared-policy ref must be a full lowercase commit SHA".to_owned());
    }

    for required in [
        "lockfile-release-age:\n    name: Lockfile release age",
        "if: github.event_name != 'pull_request'",
        "repository: Strukturpiloten/.github",
        "ref: ${{ github.event.pull_request.head.sha }}",
        "BASE_SHA: ${{ github.event.pull_request.base.sha }}",
        "HEAD_SHA: ${{ github.event.pull_request.head.sha }}",
        "python3 .github/strukturpiloten-shared/scripts/lockfile_release_age.py",
        "--repository-root \"${GITHUB_WORKSPACE}\"",
        "--base \"${BASE_SHA}\"",
        "--head \"${HEAD_SHA}\"",
        "--minimum-age-hours 72",
        "select_lockfile_release_age: ${{ steps.plan.outputs.select_lockfile_release_age }}",
        "if: needs.validation-plan.outputs.select_lockfile_release_age == 'true'",
        "NEEDS_JSON: ${{ toJSON(needs) }}",
    ] {
        if !workflow.contains(required) {
            return Err(format!("CI lockfile release-age contract is missing `{required}`"));
        }
    }
    let job = workflow
        .split_once("\n  lockfile-release-age:\n")
        .and_then(|(_, remainder)| remainder.split_once("\n  pr-gate:\n"))
        .map(|(job, _)| job)
        .ok_or_else(|| "CI lockfile release-age job boundary is missing".to_owned())?;
    let before_steps = job.split_once("\n    steps:\n").map_or(job, |(prefix, _)| prefix);
    if !before_steps.contains("if: needs.validation-plan.outputs.select_lockfile_release_age == 'true'") {
        return Err("CI lockfile release-age job must follow the fail-closed full-main plan".to_owned());
    }
    Ok(())
}

fn validate_renovate_manager_coverage(package_rules: &[serde_json::Value]) -> Result<(), String> {
    for manager in ["cargo", "npm", "github-actions", "devcontainer", "rust-toolchain"] {
        if !package_rules.iter().any(|rule| {
            rule["matchManagers"]
                .as_array()
                .is_some_and(|managers| managers.iter().any(|candidate| candidate == manager))
        }) {
            return Err(format!("Renovate configuration is missing manager `{manager}`"));
        }
    }
    let release_age_rule = package_rules
        .iter()
        .find(|rule| rule["description"] == "Do not delay BoxFerry and Lens releases")
        .ok_or_else(|| "Renovate release-age exception is missing".to_owned())?;
    if release_age_rule["minimumReleaseAge"] != "0 days" {
        return Err("Renovate must not delay coordinated BoxFerry and Lens releases".to_owned());
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
        "Signal updates for the checksum-pinned Docker Compose provider",
        "Track reviewed live-application container images",
        "Track reviewed live Podman matrix images",
        "Track the digest-pinned live workload probe image",
        "Require explicit revalidation for live application and workload images",
        "Require checksum review for the Docker Compose provider",
        "Automerge tested non-major dependency updates",
        "Do not delay BoxFerry and Lens releases",
        r#""boxferry-model""#,
        r#""compose-lens""#,
        r#""podman-lens""#,
        r#""quadlet-lens""#,
    ] {
        if !renovate.contains(required) {
            return Err(format!("Renovate configuration is missing `{required}`"));
        }
    }

    let ci_path = root.join(".github/workflows/ci.yml");
    let ci = fs::read_to_string(&ci_path).map_err(|error| format!("failed to read {}: {error}", ci_path.display()))?;
    for required in [
        "renovate: datasource=crate depName=cargo-llvm-cov",
        "renovate: datasource=node-version depName=node",
    ] {
        if !ci.contains(required) {
            return Err(format!("canonical CI is missing Renovate marker `{required}`"));
        }
    }
    let release_path = root.join(".github/workflows/release.yml");
    let release = fs::read_to_string(&release_path)
        .map_err(|error| format!("failed to read {}: {error}", release_path.display()))?;
    if !release.contains("uses: ./.github/workflows/ci.yml") {
        return Err("Release must consume the canonical Renovate-owned CI pins".to_owned());
    }

    for entry in fs::read_dir(root.join(".github/workflows"))
        .map_err(|error| format!("failed to enumerate GitHub workflows: {error}"))?
    {
        let path = entry
            .map_err(|error| format!("failed to inspect GitHub workflow entry: {error}"))?
            .path();
        if !matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("yml" | "yaml")
        ) {
            continue;
        }
        let workflow_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| format!("workflow path is not valid UTF-8: {}", path.display()))?;
        let workflow =
            fs::read_to_string(&path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        validate_workflow_renovate_pins(workflow_name, &workflow)?;
    }

    let renovate_value: serde_json::Value =
        serde_json::from_str(&renovate).map_err(|error| format!("failed to parse Renovate configuration: {error}"))?;
    let package_rules = renovate_value["packageRules"]
        .as_array()
        .ok_or_else(|| "Renovate packageRules must be an array".to_owned())?;
    validate_renovate_manager_coverage(package_rules)?;
    let ci_workflow = fs::read_to_string(root.join(".github/workflows/ci.yml"))
        .map_err(|error| format!("failed to read CI workflow: {error}"))?;
    validate_renovate_lock_maintenance(&renovate_value, package_rules)?;
    validate_shared_lockfile_guard(&renovate_value, &ci_workflow)?;
    for description in [
        "Require explicit revalidation for live application and workload images",
        "Require checksum review for the Docker Compose provider",
    ] {
        let rule = package_rules
            .iter()
            .find(|rule| rule["description"] == description)
            .ok_or_else(|| format!("Renovate is missing manual rule `{description}`"))?;
        if rule["automerge"] != false || rule["dependencyDashboardApproval"] != true {
            return Err(format!(
                "Renovate rule `{description}` must require dashboard approval and disable automerge"
            ));
        }
    }
    for manager in ["dockerfile", "docker-compose", "quadlet"] {
        if renovate_value[manager]["enabled"] != false {
            return Err(format!(
                "Renovate native {manager} extraction must stay disabled for fixture-owned files"
            ));
        }
    }

    if renovate.contains(r#""**/fixtures/**""#) {
        return Err(
            "Renovate must inspect curated live-image catalogues while package rules exclude native fixture managers"
                .to_owned(),
        );
    }

    let compose_provider = fs::read_to_string(root.join("scripts/lib/compose-provider.sh"))
        .map_err(|error| format!("failed to read shared Compose provider metadata: {error}"))?;
    for required in [
        "renovate: datasource=github-releases depName=docker/compose",
        "local -r expected_version=\"",
        "local -r expected_sha256=\"",
        "readonly BOXFERRY_COMPOSE_PROVIDER_VERSION=\"",
        "readonly BOXFERRY_COMPOSE_PROVIDER_SHA256=\"",
        "releases/download/v${expected_version}",
        "must only come from this library",
        "--retry 3 --retry-all-errors --connect-timeout 10 --max-time 300",
        "sha256sum --check --strict",
    ] {
        if !compose_provider.contains(required) {
            return Err(format!("shared Compose provider metadata is missing `{required}`"));
        }
    }
    let provider_version = shell_quoted_value(&compose_provider, "local -r expected_version")
        .ok_or_else(|| "shared Compose provider version must be a quoted canonical value".to_owned())?;
    let provider_sha = shell_quoted_value(&compose_provider, "local -r expected_sha256")
        .ok_or_else(|| "shared Compose provider checksum must be a quoted canonical value".to_owned())?;
    if provider_sha.len() != 64 || !provider_sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("shared Compose provider checksum must be 64 lowercase hex characters".to_owned());
    }

    let provider_path = root.join("scripts/lib/compose-provider.sh");
    let repeated_source = Command::new("bash")
        .args(["-c", "source \"$1\" && source \"$1\"", "compose-provider-test"])
        .arg(&provider_path)
        .env_clear()
        .output()
        .map_err(|error| format!("failed to test repeated Compose provider loading: {error}"))?;
    if !repeated_source.status.success() {
        return Err(format!(
            "shared Compose provider must support repeated sourcing: {}",
            String::from_utf8_lossy(&repeated_source.stderr)
        ));
    }

    let injected_metadata = Command::new("bash")
        .args(["-c", "source \"$1\"", "compose-provider-test"])
        .arg(&provider_path)
        .env_clear()
        .env("BOXFERRY_COMPOSE_PROVIDER_VERSION", "caller-controlled")
        .env("BOXFERRY_COMPOSE_PROVIDER_SHA256", "caller-controlled")
        .env(
            "BOXFERRY_COMPOSE_PROVIDER_URL",
            "https://example.invalid/caller-controlled",
        )
        .output()
        .map_err(|error| format!("failed to test injected Compose provider metadata: {error}"))?;
    if injected_metadata.status.success()
        || !String::from_utf8_lossy(&injected_metadata.stderr).contains("must only come from this library")
    {
        return Err("shared Compose provider must reject caller-supplied metadata".to_owned());
    }

    for metadata_name in [
        "BOXFERRY_COMPOSE_PROVIDER_VERSION",
        "BOXFERRY_COMPOSE_PROVIDER_SHA256",
        "BOXFERRY_COMPOSE_PROVIDER_URL",
    ] {
        let declared_unset_metadata = Command::new("bash")
            .args(["-c", "readonly \"$2\"; source \"$1\"", "compose-provider-test"])
            .arg(&provider_path)
            .arg(metadata_name)
            .env_clear()
            .output()
            .map_err(|error| format!("failed to test declared-but-unset Compose provider metadata: {error}"))?;
        if declared_unset_metadata.status.success()
            || !String::from_utf8_lossy(&declared_unset_metadata.stderr).contains("must only come from this library")
        {
            return Err(format!(
                "shared Compose provider must reject declared-but-unset metadata `{metadata_name}`"
            ));
        }
    }

    for application in [
        "nextcloud",
        "forgejo",
        "paperless",
        "immich",
        "observability",
        "supabase",
    ] {
        let module_path = root.join("scripts/lib").join(format!("{application}-application.sh"));
        let injected_metadata = Command::new("bash")
            .args(["-c", "source \"$1\"", "application-provider-test"])
            .arg(&module_path)
            .env_clear()
            .env("BOXFERRY_COMPOSE_PROVIDER_VERSION", "caller-controlled")
            .env("BOXFERRY_COMPOSE_PROVIDER_SHA256", "caller-controlled")
            .env(
                "BOXFERRY_COMPOSE_PROVIDER_URL",
                "https://example.invalid/caller-controlled",
            )
            .output()
            .map_err(|error| format!("failed to test {application} provider loading: {error}"))?;
        if injected_metadata.status.success()
            || !String::from_utf8_lossy(&injected_metadata.stderr).contains("must only come from this library")
        {
            return Err(format!(
                "{application} application module must propagate a rejected provider override"
            ));
        }
    }
    let provider_url =
        format!("https://github.com/docker/compose/releases/download/v{provider_version}/docker-compose-linux-x86_64");
    for application in [
        "nextcloud-application",
        "forgejo-application",
        "paperless-ngx-application",
        "immich-application",
        "observability-application",
        "supabase-application",
    ] {
        let catalogue = fs::read_to_string(
            root.join("fixtures/conformance")
                .join(application)
                .join("providers.tsv"),
        )
        .map_err(|error| format!("failed to read {application} provider catalogue: {error}"))?;
        for required in [provider_version, provider_sha, provider_url.as_str()] {
            if !catalogue.contains(required) {
                return Err(format!(
                    "{application} provider catalogue does not match canonical value `{required}`"
                ));
            }
        }
    }

    for (workflow_name, expected_installs) in [("migration-readiness.yml", 2), ("podman-live-conformance.yml", 4)] {
        let workflow = fs::read_to_string(root.join(".github/workflows").join(workflow_name))
            .map_err(|error| format!("failed to read {workflow_name}: {error}"))?;
        if workflow
            .matches("boxferry_install_compose_provider target/tools/docker-compose")
            .count()
            != expected_installs
            || workflow.contains("COMPOSE_URL:")
            || workflow.contains("COMPOSE_SHA256:")
            || workflow.contains("docker/compose/releases/download/v")
        {
            return Err(format!(
                "{workflow_name} must install the canonical Compose provider exactly {expected_installs} times without version duplicates"
            ));
        }
    }

    for application in [
        "nextcloud",
        "forgejo",
        "paperless",
        "immich",
        "observability",
        "supabase",
    ] {
        let module = fs::read_to_string(root.join("scripts/lib").join(format!("{application}-application.sh")))
            .map_err(|error| format!("failed to read {application} application module: {error}"))?;
        if !module.contains("source \"$(cd -- \"$(dirname -- \"${BASH_SOURCE[0]}\")\" && pwd -P)/compose-provider.sh\"")
            || !module.contains("${BOXFERRY_COMPOSE_PROVIDER_VERSION}")
            || !module.contains("${BOXFERRY_COMPOSE_PROVIDER_SHA256}")
            || module.contains("docker/compose/releases/download/v5.5.0")
        {
            return Err(format!(
                "{application} application module must consume shared Compose provider metadata"
            ));
        }
    }

    let image_catalogues = [
        "nextcloud-application",
        "forgejo-application",
        "paperless-ngx-application",
        "immich-application",
        "observability-application",
        "supabase-application",
    ];
    let mut active_images = 0;
    for application in image_catalogues {
        let catalogue = fs::read_to_string(root.join("fixtures/conformance").join(application).join("images.tsv"))
            .map_err(|error| format!("failed to read {application} image catalogue: {error}"))?;
        for line in catalogue.lines().filter(|line| {
            let line = line.trim();
            !line.is_empty() && !line.starts_with('#')
        }) {
            let reference = line
                .split_whitespace()
                .nth(1)
                .ok_or_else(|| format!("{application} image row is missing a reference: {line}"))?;
            let Some((_, digest)) = reference.rsplit_once("@sha256:") else {
                return Err(format!("{application} image is not digest pinned: {reference}"));
            };
            if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                return Err(format!("{application} image has an invalid digest: {reference}"));
            }
            active_images += 1;
        }
    }
    if active_images != 32 {
        return Err(format!(
            "Renovate live-application manager must cover 32 active images, found {active_images}"
        ));
    }

    let live_runner = fs::read_to_string(root.join("scripts/podman-live-conformance.sh"))
        .map_err(|error| format!("failed to read live Podman runner: {error}"))?;
    for required in [
        "renovate: datasource=docker depName=quay.io/libpod/alpine",
        "workload_image=\"quay.io/libpod/alpine@sha256:",
        "workload_local_tag=\"localhost/boxferry-live/alpine:${workload_image##*@sha256:}\"",
    ] {
        if !live_runner.contains(required) {
            return Err(format!("live workload probe is missing Renovate contract `{required}`"));
        }
    }

    let file_tool_installer = fs::read_to_string(root.join("scripts/install-file-tools.sh"))
        .map_err(|error| format!("failed to read file-tool installer: {error}"))?;
    if !file_tool_installer.contains("--retry 3 --retry-all-errors --connect-timeout 10 --max-time 300") {
        return Err("checksum-pinned file-tool downloads must retry bounded transient failures".to_owned());
    }

    let development = fs::read_to_string(root.join("docs/development-environment.md"))
        .map_err(|error| format!("failed to read development environment guide: {error}"))?;
    let devcontainer_command = development
        .lines()
        .find(|line| line.starts_with("npx --yes @devcontainers/cli@"))
        .ok_or_else(|| "Dev Container feature-lock guide must pin its Renovate-managed CLI".to_owned())?;
    let version = devcontainer_command
        .strip_prefix("npx --yes @devcontainers/cli@")
        .and_then(|command| command.strip_suffix(" upgrade --workspace-folder ."))
        .ok_or_else(|| "Dev Container feature-lock command has an unexpected shape".to_owned())?;
    if version.split('.').count() != 3
        || !version
            .split('.')
            .all(|component| !component.is_empty() && component.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err("Dev Container feature-lock command must use an exact semantic version".to_owned());
    }

    let policy_source = fs::read_to_string(root.join("crates/boxferry/tests/repository_policy.rs"))
        .map_err(|error| format!("failed to read repository policy source: {error}"))?;
    for line in policy_source.lines() {
        let Some((_, after_at)) = line.split_once('@') else {
            continue;
        };
        let candidate = after_at.as_bytes();
        if candidate.len() >= 40 && candidate[..40].iter().all(u8::is_ascii_hexdigit) && after_at[40..].contains("# v")
        {
            return Err("semantic repository policy must not embed a Renovate-owned action revision".to_owned());
        }
    }

    Ok(())
}

#[test]
fn renovate_policy_rejects_unmanaged_workflow_pin_counterexamples() -> Result<(), String> {
    for (description, workflow) in [
        (
            "Node.js version",
            "steps:\n  - uses: actions/setup-node@example\n    with:\n      node-version: 25.0.0\n",
        ),
        (
            "Cargo tool version",
            "steps:\n  - run: cargo install --locked --version 1.2.3 example-tool\n",
        ),
        (
            "release download",
            "steps:\n  - run: curl https://github.com/example/tool/releases/download/v1.2.3/tool\n",
        ),
        (
            "job container image",
            "jobs:\n  test:\n    container:\n      image: postgres:17.6\n",
        ),
        (
            "Docker action image",
            "steps:\n  - uses: docker://ghcr.io/example/tool:1.2.3\n",
        ),
        (
            "direct container pull",
            "steps:\n  - run: docker pull ghcr.io/example/tool:1.2.3\n",
        ),
    ] {
        if validate_workflow_renovate_pins("counterexample.yml", workflow).is_ok() {
            return Err(format!("Renovate workflow policy accepted an unmanaged {description}"));
        }
    }

    Ok(())
}

#[test]
fn renovate_workflow_tool_manager_covers_yaml_extensions() -> Result<(), String> {
    let path = repository_root().join(".github/renovate.json");
    let renovate = fs::read_to_string(&path).map_err(|error| format!("failed read {}: {error}", path.display()))?;
    let renovate: serde_json::Value =
        serde_json::from_str(&renovate).map_err(|error| format!("failed parse {}: {error}", path.display()))?;
    let managers = renovate["customManagers"]
        .as_array()
        .ok_or_else(|| "Renovate customManagers must be an array".to_owned())?;
    let workflow_tool_manager = managers
        .iter()
        .find(|manager| manager["description"] == "Update directly pinned workflow tool versions")
        .ok_or_else(|| "Renovate must track directly pinned workflow tool versions".to_owned())?;
    if workflow_tool_manager["managerFilePatterns"] != serde_json::json!(["/^\\.github/workflows/.*\\.ya?ml$/"]) {
        return Err("Renovate workflow-tool manager must inspect .yml and .yaml workflows".to_owned());
    }
    Ok(())
}

#[test]
fn renovate_tracks_fixed_github_hosted_runners() -> Result<(), String> {
    let path = repository_root().join(".github/renovate.json");
    let renovate = fs::read_to_string(&path).map_err(|error| format!("failed read {}: {error}", path.display()))?;
    let renovate: serde_json::Value =
        serde_json::from_str(&renovate).map_err(|error| format!("failed parse {}: {error}", path.display()))?;
    let managers = renovate["customManagers"]
        .as_array()
        .ok_or_else(|| "Renovate customManagers must be an array".to_owned())?;
    let runner_manager = managers
        .iter()
        .find(|manager| manager["description"] == "Track fixed GitHub-hosted runner environments")
        .ok_or_else(|| "Renovate must track fixed GitHub-hosted runner labels".to_owned())?;
    let expected_pattern = concat!(
        "(?:^|\\n)\\s*runs-on:\\s*(?<depName>ubuntu|macos|windows)-",
        "(?<currentValue>[0-9]+(?:\\.[0-9]+)?(?:-[a-z0-9]+)*)\\s*(?:\\n|$)"
    );
    if runner_manager["datasourceTemplate"] != "github-runners"
        || runner_manager["managerFilePatterns"] != serde_json::json!(["/^\\.github/workflows/.*\\.ya?ml$/"])
        || runner_manager["matchStrings"] != serde_json::json!([expected_pattern])
    {
        return Err(
            "Renovate runner manager must capture fixed workflow runner names and versions with a JavaScript-compatible pattern"
                .to_owned(),
        );
    }

    let rules = renovate["packageRules"]
        .as_array()
        .ok_or_else(|| "Renovate packageRules must be an array".to_owned())?;
    let rule = rules
        .iter()
        .find(|rule| rule["description"] == "Review GitHub-hosted runner environment upgrades manually")
        .ok_or_else(|| "Renovate must retain a manual GitHub-hosted runner review rule".to_owned())?;
    if rule["automerge"] != false || rule["groupName"] != "GitHub-hosted runners" {
        return Err("Renovate GitHub-hosted runner updates must be grouped without automerge".to_owned());
    }

    for workflow in [
        "runs-on: ubuntu-latest\n",
        "runs-on: ubuntu22\n",
        "runs-on: macos-preview\n",
        "runs-on: windows-preview\n",
        "runs-on: \"ubuntu-24.04\"\n",
        "runs-on: &hosted macos-14\n",
    ] {
        if validate_workflow_renovate_pins("runner-counterexample.yml", workflow).is_ok() {
            return Err(format!(
                "Renovate workflow policy accepted unmanaged runner `{workflow:?}`"
            ));
        }
    }
    for workflow in [
        "runs-on: ubuntu-24.04\n",
        "runs-on: ubuntu-24.04-arm\n",
        "runs-on: macos-14-large\n",
        "runs-on: windows-2025\n",
        "runs-on: ${{ matrix.runner }}\n",
        "runs-on: self-hosted\n",
    ] {
        validate_workflow_renovate_pins("runner-example.yml", workflow)?;
    }
    Ok(())
}

fn validate_live_podman_matrix_renovate_policy(renovate: &serde_json::Value) -> Result<(), String> {
    let managers = renovate["customManagers"]
        .as_array()
        .ok_or_else(|| "Renovate customManagers must be an array".to_owned())?;
    let manager = managers
        .iter()
        .find(|manager| manager["description"] == "Track reviewed live Podman matrix images")
        .ok_or_else(|| "Renovate must track the live Podman matrix images".to_owned())?;
    if manager["managerFilePatterns"] != serde_json::json!(["/^fixtures/conformance/podman-live/matrix\\.tsv$/"]) {
        return Err("Renovate live Podman manager must target only matrix.tsv".to_owned());
    }
    if manager["matchStrings"]
        != serde_json::json!([
            r"(?:^|\n)[^#\s]+[ \t]+(?<depName>[^\s:@]+(?:/[^\s:@]+)+):(?<currentValue>[^@\s]+)@(?<currentDigest>sha256:[a-f0-9]{64})(?:[ \t]|$)"
        ])
        || manager["datasourceTemplate"] != "docker"
        || manager["versioningTemplate"] != "docker"
    {
        return Err("Renovate live Podman manager must extract every image tag and digest".to_owned());
    }

    let rules = renovate["packageRules"]
        .as_array()
        .ok_or_else(|| "Renovate packageRules must be an array".to_owned())?;
    let review_rule = rules
        .iter()
        .find(|rule| rule["description"] == "Require explicit revalidation for live application and workload images")
        .ok_or_else(|| "Renovate must define the live-image review rule".to_owned())?;
    let reviewed_files = review_rule["matchFileNames"]
        .as_array()
        .ok_or_else(|| "Renovate live-image review rule must list matched files".to_owned())?;
    if !reviewed_files
        .iter()
        .any(|path| path == "fixtures/conformance/podman-live/matrix.tsv")
        || review_rule["dependencyDashboardApproval"] != true
        || review_rule["automerge"] != false
    {
        return Err("Renovate live Podman updates must require explicit review and revalidation".to_owned());
    }
    Ok(())
}

#[test]
fn renovate_tracks_every_live_podman_matrix_image_under_manual_review() -> Result<(), String> {
    let root = repository_root();
    let path = root.join(".github/renovate.json");
    let renovate = fs::read_to_string(&path).map_err(|error| format!("failed read {}: {error}", path.display()))?;
    let renovate: serde_json::Value =
        serde_json::from_str(&renovate).map_err(|error| format!("failed parse {}: {error}", path.display()))?;
    validate_live_podman_matrix_renovate_policy(&renovate)?;

    let matrix = fs::read_to_string(root.join("fixtures/conformance/podman-live/matrix.tsv"))
        .map_err(|error| format!("failed read live Podman matrix: {error}"))?;
    let mut image_count = 0;
    for (index, line) in matrix.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let id = fields
            .next()
            .ok_or_else(|| format!("matrix row {} has no ID", index + 1))?;
        let reference = fields
            .next()
            .ok_or_else(|| format!("matrix row {id} has no image reference"))?;
        let (tagged_image, digest) = reference
            .rsplit_once("@sha256:")
            .ok_or_else(|| format!("matrix row {id} is not digest pinned"))?;
        let (dependency, tag) = tagged_image
            .rsplit_once(':')
            .ok_or_else(|| format!("matrix row {id} has no image tag"))?;
        if !dependency.contains('/')
            || tag.is_empty()
            || digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(format!(
                "matrix row {id} does not match the Renovate-managed image contract"
            ));
        }
        image_count += 1;
    }
    if image_count == 0 {
        return Err("live Podman matrix must contain Renovate-managed images".to_owned());
    }

    let mut missing_manager = renovate.clone();
    missing_manager["customManagers"]
        .as_array_mut()
        .ok_or_else(|| "validated customManagers array disappeared".to_owned())?
        .retain(|manager| manager["description"] != "Track reviewed live Podman matrix images");
    if validate_live_podman_matrix_renovate_policy(&missing_manager).is_ok() {
        return Err("Renovate policy accepted an unmanaged live Podman matrix".to_owned());
    }

    let mut missing_review = renovate;
    let review_rule = missing_review["packageRules"]
        .as_array_mut()
        .ok_or_else(|| "validated packageRules array disappeared".to_owned())?
        .iter_mut()
        .find(|rule| rule["description"] == "Require explicit revalidation for live application and workload images")
        .ok_or_else(|| "validated live-image review rule disappeared".to_owned())?;
    review_rule["matchFileNames"]
        .as_array_mut()
        .ok_or_else(|| "validated matchFileNames array disappeared".to_owned())?
        .retain(|path| path != "fixtures/conformance/podman-live/matrix.tsv");
    if validate_live_podman_matrix_renovate_policy(&missing_review).is_ok() {
        return Err("Renovate policy accepted an automatically updated live Podman matrix".to_owned());
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
        "model = \"gpt-6-sol\"",
        "model_reasoning_effort = \"xhigh\"",
        "max_concurrent_threads_per_session = 9",
        "default_subagent_model = \"gpt-6-sol\"",
        "default_subagent_reasoning_effort = \"medium\"",
    ] {
        assert!(config.contains(required), "missing agent default: {required}");
    }
    for (role, model, effort, sandbox) in [
        ("implementation-worker", "gpt-6-sol", "high", "workspace-write"),
        ("specification-researcher", "gpt-6-sol", "high", "read-only"),
        ("reviewer", "gpt-6-sol", "high", "read-only"),
        ("verifier", "gpt-6-luna", "high", "workspace-write"),
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
    assert!(verifier.contains("Escalate complex failure diagnosis to a Sol agent"));
    Ok(())
}

#[test]
fn workspace_git_authorization_and_agent_limits_are_bounded() -> Result<(), Box<dyn std::error::Error>> {
    let instructions = fs::read_to_string(repository_root().join("AGENTS.md"))?;
    let authorization = instructions
        .split_once("## Workspace scope and standing GitHub authorization")
        .ok_or("missing workspace authorization section")?
        .1
        .split("\n## ")
        .next()
        .ok_or("missing workspace authorization content")?;
    let repositories = authorization
        .lines()
        .filter(|line| line.starts_with("- "))
        .collect::<Vec<_>>();
    assert_eq!(
        repositories,
        [
            "- `Strukturpiloten/boxferry`",
            "- `Strukturpiloten/compose-lens`",
            "- `Strukturpiloten/podman-lens`",
            "- `Strukturpiloten/quadlet-lens`",
            "- `Strukturpiloten/boxferry-website`",
            "- `Strukturpiloten/docker-lens`",
        ]
    );
    let flattened = instructions.split_whitespace().collect::<Vec<_>>().join(" ");
    for required in [
        "The primary manager always uses `gpt-6-sol` with `xhigh` reasoning",
        "up to nine concurrent subagents plus the primary manager",
        "subject to the session's actual runtime limit",
        "Nine is a ceiling, not a target or nine distinct roles",
        "Do not create nested agents to evade the limit",
        "Never run two writers in one checkout",
        "at most one complete gate or heavy runtime suite at a time across this workspace",
        "Do not work on or modify any repository outside this explicit allowlist",
        "For user-requested work within this scope",
        "may create issues, branches, commits, pushes, and pull requests and merge verified task-related pull requests without asking for renewed approval",
        "does not authorize unrelated backlog work, implementation of discussion-only proposals",
        "A later user instruction may narrow or revoke this permission",
        "ready, mergeable, independently reviewed, and has every required check successful",
        "exact-head safeguard; never bypass branch protection or use an administrator override",
        "synchronize local `main` with `origin/main`",
        "does not authorize releases, publication, deployment operations, or merging release/publication/deployment pull requests",
        "Subagents remain within their assigned task and checkout and must not perform those writes",
    ] {
        assert!(flattened.contains(required), "missing workspace boundary: {required}");
    }
    for obsolete in [
        "Use at most three subagents",
        "does not authorize a merge",
        "Merge only when the user explicitly authorizes",
    ] {
        assert!(!flattened.contains(obsolete), "obsolete authorization rule: {obsolete}");
    }
    Ok(())
}

// The full shell gate targets the Linux Dev Container; there is no macOS runner.
// Keep configuration assertions above platform-independent.
#[cfg(target_os = "linux")]
#[test]
fn linux_gate_modes_and_failure_propagation_are_correct() -> Result<(), Box<dyn std::error::Error>> {
    let root = repository_root();
    let result = Command::new("bash")
        .arg("scripts/test-check-all.sh")
        .current_dir(&root)
        .output()?;
    assert!(
        result.status.success(),
        "gate mode regression failed:\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let result = Command::new("bash")
        .arg("scripts/test-format-lint.sh")
        .current_dir(&root)
        .output()?;
    assert!(
        result.status.success(),
        "format/lint task regression failed:\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    Ok(())
}
