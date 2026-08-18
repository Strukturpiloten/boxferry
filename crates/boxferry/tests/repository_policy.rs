//! Executable repository and fixture-contract checks.

mod support;

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

const FIXTURE_SUITES: &[&str] = &[
    "model",
    "adapter-contract",
    "conversion",
    "roundtrip",
    "differential",
    "real-world",
];

const PUBLISHED_PACKAGES: &[&str] = &[
    "boxferry-model",
    "boxferry-engine",
    "boxferry-compose",
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
        "git --no-pager diff --check",
        "actionlint",
        "zizmor .github/workflows",
        "cargo ci-check",
        "cargo ci-core",
        "cargo ci-compose",
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
fn issue_to_pr_workflow_requires_sol_ownership_and_the_complete_local_gate() -> Result<(), String> {
    let root = repository_root();
    for (path, required) in [
        (
            "AGENTS.md",
            &[
                "## GitHub issue-to-PR workflow",
                "Run `./scripts/check-all.sh`",
                "hard gate against commit, push",
                "primary Sol agent runs this workflow",
                "high reasoning effort",
                "Terra subagents",
                "never execute the Git or GitHub",
                "remains Sol's responsibility",
            ][..],
        ),
        (
            "docs/development-environment.md",
            &[
                "## Issue-to-PR contribution workflow",
                "./scripts/check-all.sh",
                "All steps must pass before the change is committed, pushed, or submitted",
                "primary Sol agent uses high reasoning effort",
                "Terra agents",
                "never perform Git or GitHub writes",
                "Sol's final responsibility",
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

    let markdown_format = script
        .find(r#"prettier --write --ignore-unknown "${markdown_files[@]}""#)
        .ok_or("non-Rust file runner must format Markdown with Prettier")?;
    let markdown_fix = script
        .find(r#"markdownlint-cli2 --fix "${markdown_literals[@]}""#)
        .ok_or("non-Rust file runner must apply fixable Markdown lint rules")?;
    let markdown_lint = script
        .find(r#"markdownlint-cli2 "${markdown_literals[@]}""#)
        .ok_or("non-Rust file runner must lint Markdown after formatting")?;
    let markdown_check = script
        .find(r#"prettier --check --ignore-unknown "${markdown_files[@]}""#)
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
        "\"CARGO_TARGET_DIR\": \"/workspaces/.boxferry-target\"",
        "\"GH_CONFIG_DIR\": \"/workspaces/.boxferry-gh\"",
        "source=boxferry-cargo-${devcontainerId},target=/workspaces/.boxferry-cargo,type=volume",
        "source=boxferry-gh-${devcontainerId},target=/workspaces/.boxferry-gh,type=volume",
        "source=boxferry-target-${devcontainerId},target=/workspaces/.boxferry-target,type=volume",
        "source=${localWorkspaceFolder}/../compose-lens,target=/workspaces/boxferry/.boxferry-workspace/compose-lens,type=bind",
        "source=${localWorkspaceFolder}/../quadlet-lens,target=/workspaces/boxferry/.boxferry-workspace/quadlet-lens,type=bind",
    ] {
        if !devcontainer.contains(required) {
            return Err(format!("BoxFerry Dev Container is missing sibling mount `{required}`"));
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
        "CARGO_TARGET_DIR",
        "GH_CONFIG_DIR",
        "[[ ! -w \"${persistent_directory}\" ]]",
        "sudo chown -R",
        "chmod 0700 \"${GH_CONFIG_DIR}\"",
    ] {
        if !script.contains(required) {
            return Err(format!("Dev Container lifecycle check is missing `{required}`"));
        }
    }
    if script.contains("sudo -u") {
        return Err("Dev Container lifecycle check must not require forbidden sudo user switching".to_owned());
    }

    Ok(())
}

#[test]
fn ci_workflow_enforces_coverage_portability_and_pr_gate_contract() -> Result<(), String> {
    let workflow_path = repository_root().join(".github/workflows/ci.yml");
    let workflow = fs::read_to_string(&workflow_path)
        .map_err(|error| format!("failed to read {}: {error}", workflow_path.display()))?;

    for required in [
        "  coverage:\n    name: Coverage ratchet",
        "rustup component add llvm-tools-preview",
        "cargo install --locked --version 0.8.7 cargo-llvm-cov",
        "cargo llvm-cov clean --locked",
        "cargo llvm-cov --locked --no-clean --workspace --all-features --all-targets --summary-only\n          --fail-under-regions 82 --fail-under-functions 87 --fail-under-lines 82",
        "  portability:\n    name: Portability (macOS)",
        "runs-on: macos-14",
        "run: cargo ci-check",
        "run: cargo ci-test",
        "  pr-gate:\n    name: PR gate\n    if: always()",
        "needs:\n      [rust, msrv, dependencies, documentation, semver-release-type, semver, coverage, portability]",
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
            "SemVer release type",
            "SEMVER_RELEASE_TYPE_RESULT",
            "semver-release-type",
        ),
        ("SemVer", "SEMVER_RESULT", "semver"),
        ("Coverage ratchet", "COVERAGE_RESULT", "coverage"),
        ("macOS portability", "PORTABILITY_RESULT", "portability"),
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
    let workflow_path = repository_root().join(".github/workflows/release.yml");
    let workflow = fs::read_to_string(&workflow_path)
        .map_err(|error| format!("failed to read {}: {error}", workflow_path.display()))?;

    for required in [
        "rustup component add llvm-tools-preview",
        "cargo install --locked --version 0.8.7 cargo-llvm-cov",
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
        "actions/attest@508db95dd578ae2727ebd6217d5ba78e4fbda05d # v4.2.1",
        "Create or verify annotated release tag",
        "Publish immutable GitHub release",
        "Release package metadata is invalid:",
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
        return Err("release-plz.toml must configure all five lockstep packages".to_owned());
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
        || facade["changelog_include"].as_array().map(Vec::len) != Some(4)
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
    ] {
        if !renovate.contains(required) {
            return Err(format!("Renovate configuration is missing `{required}`"));
        }
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
fn deferred_runtime_namespaces_are_not_published_by_the_current_product() {
    for rule in boxferry_engine::RULES {
        assert!(
            !matches!(rule.code().get(..3), Some("BFD" | "BFP" | "BFR")),
            "deferred runtime rule namespace must not remain published: {}",
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
