//! Validation for immutable external GitHub Action references.

use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn validate_action_pins(repository_root: &Path) -> Result<(), String> {
    let workflows = repository_root.join(".github/workflows");
    let mut files = Vec::new();
    collect_workflows(&workflows, &mut files)?;

    let mut errors = Vec::new();
    for path in files {
        let text = fs::read_to_string(&path)
            .map_err(|error| format!("failed to read {}: {error}", path.display()))?;

        for (index, line) in text.lines().enumerate() {
            let Some(reference) = action_reference(line) else {
                continue;
            };

            if reference.starts_with("./") || reference.starts_with("docker://") {
                continue;
            }

            if !is_immutable_versioned_action(reference) {
                errors.push(format!(
                    "{}:{} must use owner/action@<40-character SHA> # v<major>.<minor>.<patch>",
                    path.display(),
                    index + 1
                ));
            }
        }
    }

    finish(&errors)
}

fn collect_workflows(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("failed to read {}: {error}", directory.display()))?;

    for entry in entries {
        let entry =
            entry.map_err(|error| format!("failed to inspect {}: {error}", directory.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect {}: {error}", path.display()))?;

        if file_type.is_dir() {
            collect_workflows(&path, files)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "yml" || extension == "yaml")
        {
            files.push(path);
        }
    }

    Ok(())
}

fn action_reference(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.starts_with('#') {
        return None;
    }

    trimmed
        .strip_prefix("- uses:")
        .or_else(|| trimmed.strip_prefix("uses:"))
        .map(str::trim)
}

fn is_immutable_versioned_action(reference: &str) -> bool {
    let Some((action_and_sha, version)) = reference.split_once('#') else {
        return false;
    };
    let Some((action, sha)) = action_and_sha.trim().rsplit_once('@') else {
        return false;
    };

    !action.is_empty()
        && sha.len() == 40
        && sha
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        && is_exact_version(version.trim())
}

fn is_exact_version(version: &str) -> bool {
    let version = version.strip_prefix('v').unwrap_or(version);
    let core = version
        .split_once('-')
        .map_or(version, |(core, _prerelease)| core);
    let core = core.split_once('+').map_or(core, |(core, _build)| core);
    let mut components = core.split('.');

    matches!(components.next(), Some(value) if is_ascii_number(value))
        && matches!(components.next(), Some(value) if is_ascii_number(value))
        && matches!(components.next(), Some(value) if is_ascii_number(value))
        && components.next().is_none()
}

fn is_ascii_number(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn finish(errors: &[String]) -> Result<(), String> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::is_immutable_versioned_action;

    #[test]
    fn accepts_a_full_sha_with_an_exact_version() {
        assert!(is_immutable_versioned_action(
            "actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1"
        ));
    }

    #[test]
    fn rejects_a_mutable_tag() {
        assert!(!is_immutable_versioned_action("actions/checkout@v7.0.1"));
    }
}
