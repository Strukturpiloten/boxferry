//! Executable repository and fixture-contract checks.

mod support;

use std::{fs, path::PathBuf};

const FIXTURE_SUITES: &[&str] = &[
    "model",
    "adapter-contract",
    "conversion",
    "roundtrip",
    "differential",
    "runtime",
    "real-world",
];

const PUBLISHED_PACKAGES: &[&str] = &[
    "boxferry-model",
    "boxferry-engine",
    "boxferry-compose",
    "boxferry-quadlet",
    "boxferry-runtime",
    "boxferry-docker",
    "boxferry-podman",
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
                "tamasfe.even-better-toml",
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
fn local_validation_runner_covers_deterministic_repository_checks() -> Result<(), String> {
    let script_path = repository_root().join("scripts/check-all.sh");
    let script = fs::read_to_string(&script_path)
        .map_err(|error| format!("failed to read {}: {error}", script_path.display()))?;

    for required in [
        "cargo fmt --all",
        "bash scripts/check-files.sh --fix",
        "git diff --check",
        "actionlint",
        "zizmor .github/workflows",
        "cargo ci-check",
        "cargo ci-core",
        "cargo ci-compose",
        "cargo ci-docker",
        "cargo ci-quadlet",
        "cargo ci-podman",
        "cargo ci-runtime",
        "cargo ci-policy",
        "cargo ci-clippy",
        "cargo ci-test",
        "cargo ci-doctest",
        "RUSTDOCFLAGS=\"-D warnings\" cargo ci-doc",
        "cargo llvm-cov --locked --workspace --all-features",
        "--fail-under-regions 82 --fail-under-functions 87",
        "cargo \"+${msrv}\" ci-check",
        "cargo \"+${msrv}\" ci-policy",
        "cargo deny --all-features check",
        "lychee --root-dir .",
        "cargo semver-checks check-release --workspace",
    ] {
        if !script.contains(required) {
            return Err(format!("local validation runner missing `{required}`"));
        }
    }

    for opt_in in [
        "ci-docker-conformance",
        "ci-podman-conformance",
        "ci-podman-current-conformance",
        "ci-real-world-compose",
    ] {
        if script.contains(opt_in) {
            return Err(format!(
                "local validation runner must not invoke opt-in tier `{opt_in}`"
            ));
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
        "markdownlint-cli2 --fix",
        "prettier --write",
        "prettier --check",
        "taplo fmt",
        "taplo check",
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
        "source=boxferry-cargo-${devcontainerId},target=/workspaces/.boxferry-cargo,type=volume",
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
        "[[ ! -w \"${cargo_directory}\" ]]",
        "sudo chown -R",
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
        "cargo llvm-cov --locked --workspace --all-features --all-targets --summary-only\n          --fail-under-regions 82 --fail-under-functions 87 --fail-under-lines 82",
        "  portability:\n    name: Portability (macOS)",
        "runs-on: macos-14",
        "run: cargo ci-check",
        "run: cargo ci-test",
        "  pr-gate:\n    name: PR gate\n    if: always()",
        "needs: [rust, msrv, dependencies, documentation, semver, coverage, portability]",
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
fn fixture_manifests_follow_the_common_contract() -> Result<(), String> {
    support::validate_fixture_tree(&repository_root(), FIXTURE_SUITES)
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
