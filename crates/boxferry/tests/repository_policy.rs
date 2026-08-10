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
