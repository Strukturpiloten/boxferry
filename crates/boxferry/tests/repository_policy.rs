//! Executable repository and fixture-contract checks.

mod support;

use std::path::PathBuf;

const FIXTURE_SUITES: &[&str] = &[
    "model",
    "adapter-contract",
    "conversion",
    "roundtrip",
    "differential",
    "runtime",
    "real-world",
];

#[test]
fn github_actions_are_immutable_and_versioned() -> Result<(), String> {
    support::validate_action_pins(&repository_root())
}

#[test]
fn fixture_manifests_follow_the_common_contract() -> Result<(), String> {
    support::validate_fixture_tree(&repository_root(), FIXTURE_SUITES)
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
        errors
            .iter()
            .any(|error| error.contains("secrets_reviewed")),
        "{errors:#?}"
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("unsafe fixture path")),
        "{errors:#?}"
    );
    assert!(
        errors.iter().any(|error| error.contains("`url`")),
        "{errors:#?}"
    );
    assert!(
        errors.iter().any(|error| error.contains("`revision`")),
        "{errors:#?}"
    );
}

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
