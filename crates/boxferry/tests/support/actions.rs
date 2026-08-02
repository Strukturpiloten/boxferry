//! Validation for immutable external GitHub Action references.

use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn validate_repository_supply_chain(repository_root: &Path) -> Result<(), String> {
    let mut errors = Vec::new();
    validate_msrv_source(repository_root, &mut errors)?;
    validate_devcontainer_image(repository_root, &mut errors)?;
    validate_devcontainer_features(repository_root, &mut errors)?;
    finish(&errors)
}

fn validate_msrv_source(repository_root: &Path, errors: &mut Vec<String>) -> Result<(), String> {
    let manifest_path = repository_root.join("Cargo.toml");
    let manifest = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("failed to read {}: {error}", manifest_path.display()))?;
    let msrv = manifest
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("rust-version =")
                .map(|value| value.trim().trim_matches('"'))
        })
        .ok_or_else(|| format!("{} must declare rust-version", manifest_path.display()))?;

    let workflows = repository_root.join(".github/workflows");
    let mut files = Vec::new();
    collect_workflows(&workflows, &mut files)?;
    for path in files {
        let workflow =
            fs::read_to_string(&path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
        if workflow.contains(msrv) {
            errors.push(format!(
                "{} duplicates MSRV {msrv}; workflows must read Cargo metadata",
                path.display()
            ));
        }
    }

    Ok(())
}

fn validate_devcontainer_image(repository_root: &Path, errors: &mut Vec<String>) -> Result<(), String> {
    let dockerfile_path = repository_root.join(".devcontainer/Dockerfile");
    let dockerfile = fs::read_to_string(&dockerfile_path)
        .map_err(|error| format!("failed to read {}: {error}", dockerfile_path.display()))?;
    let images: Vec<_> = dockerfile
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("FROM ")
                .and_then(|line| line.split_ascii_whitespace().next())
        })
        .collect();
    let releases: Vec<_> = dockerfile
        .lines()
        .filter_map(|line| line.trim().strip_prefix("# release: "))
        .collect();

    if images.is_empty() || images.len() != releases.len() {
        errors.push(format!(
            "{} must record one exact release per FROM instruction",
            dockerfile_path.display()
        ));
        return Ok(());
    }

    for (release, image) in releases.into_iter().zip(images) {
        let digest = image.split_once("@sha256:").map(|(_image, digest)| digest);
        if !is_exact_version(release)
            || !digest.is_some_and(|digest| {
                digest.len() == 64
                    && digest
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
            })
        {
            errors.push(format!(
                "{} base images must record exact releases and use 64-character lowercase sha256 digests",
                dockerfile_path.display()
            ));
        }
    }

    Ok(())
}

fn validate_devcontainer_features(repository_root: &Path, errors: &mut Vec<String>) -> Result<(), String> {
    let config_path = repository_root.join(".devcontainer/devcontainer.json");
    let config = fs::read_to_string(&config_path)
        .map_err(|error| format!("failed to read {}: {error}", config_path.display()))?;
    let mut configured_features = Vec::new();
    for line in config.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("\"ghcr.io/") {
            continue;
        }
        let reference = trimmed.split('"').nth(1).unwrap_or_default();
        let version = reference.rsplit_once(':').map(|(_name, version)| version);
        configured_features.push((reference.to_owned(), version.unwrap_or_default().to_owned()));
        if !version.is_some_and(is_exact_version) {
            errors.push(format!(
                "{} feature {reference} must use an exact version",
                config_path.display()
            ));
        }
    }

    let lock_path = repository_root.join(".devcontainer/devcontainer-lock.json");
    let lock =
        fs::read_to_string(&lock_path).map_err(|error| format!("failed to read {}: {error}", lock_path.display()))?;
    let integrities: Vec<_> = lock
        .lines()
        .filter_map(|line| {
            line.trim()
                .strip_prefix("\"integrity\": \"sha256:")
                .and_then(|value| value.strip_suffix('\"'))
        })
        .collect();

    if configured_features.is_empty() || integrities.len() != configured_features.len() {
        errors.push(format!(
            "{} must contain one integrity hash per configured feature",
            lock_path.display()
        ));
    }
    for digest in integrities {
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            errors.push(format!(
                "{} contains an invalid feature integrity hash",
                lock_path.display()
            ));
        }
    }

    for (reference, version) in configured_features {
        let Some(block) = json_object_for_key(&lock, &reference) else {
            errors.push(format!(
                "{} does not lock configured feature {reference}",
                lock_path.display()
            ));
            continue;
        };
        let feature_name = reference
            .rsplit_once(':')
            .map_or(reference.as_str(), |(name, _version)| name);
        let locked_version = json_string_field(block, "version");
        let resolved = json_string_field(block, "resolved");
        let integrity = json_string_field(block, "integrity");
        let resolved_digest = resolved
            .and_then(|value| value.strip_prefix(&format!("{feature_name}@")))
            .and_then(valid_sha256);
        let integrity_digest = integrity.and_then(valid_sha256);

        if locked_version != Some(version.as_str()) || resolved_digest.is_none() || resolved_digest != integrity_digest
        {
            errors.push(format!(
                "{} feature {reference} must lock the same exact version and matching sha256 integrity",
                lock_path.display()
            ));
        }
    }

    Ok(())
}

fn json_object_for_key<'a>(document: &'a str, key: &str) -> Option<&'a str> {
    let marker = format!("\"{key}\": {{");
    let start = document.find(&marker)? + marker.len() - 1;
    let mut depth = 0_usize;

    for (offset, character) in document[start..].char_indices() {
        match character {
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(&document[start..=start + offset]);
                }
            }
            _ => {}
        }
    }

    None
}

fn json_string_field<'a>(object: &'a str, field: &str) -> Option<&'a str> {
    let marker = format!("\"{field}\": \"");
    let start = object.find(&marker)? + marker.len();
    object[start..].split_once('"').map(|(value, _rest)| value)
}

fn valid_sha256(value: &str) -> Option<&str> {
    let digest = value.strip_prefix("sha256:")?;
    (digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()))
    .then_some(digest)
}

pub(crate) fn validate_action_pins(repository_root: &Path) -> Result<(), String> {
    let workflows = repository_root.join(".github/workflows");
    let mut files = Vec::new();
    collect_workflows(&workflows, &mut files)?;

    let mut errors = Vec::new();
    for path in files {
        let text = fs::read_to_string(&path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;

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
    let entries =
        fs::read_dir(directory).map_err(|error| format!("failed to read {}: {error}", directory.display()))?;

    for entry in entries {
        let entry = entry.map_err(|error| format!("failed to inspect {}: {error}", directory.display()))?;
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
    let core = version.split_once('-').map_or(version, |(core, _prerelease)| core);
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
