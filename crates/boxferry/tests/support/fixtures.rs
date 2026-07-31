//! Validation for the common fixture manifest contract.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use toml::{Table, Value};

const ROOT_FIELDS: &[&str] = &[
    "description",
    "environment",
    "expectations",
    "extensions",
    "files",
    "id",
    "oracle",
    "provenance",
    "schema",
    "secrets_reviewed",
    "suite",
];

pub(crate) fn validate_fixture_tree(
    repository_root: &Path,
    allowed_suites: &[&str],
) -> Result<(), String> {
    let fixtures_root = repository_root.join("fixtures");
    if !fixtures_root.join("README.md").is_file() {
        return Err(format!(
            "{} must contain README.md",
            fixtures_root.display()
        ));
    }

    let mut manifests = Vec::new();
    collect_manifests(&fixtures_root, &mut manifests)?;
    let mut ids = BTreeSet::new();
    let mut errors = Vec::new();

    for path in manifests {
        validate_manifest_file(&path, &fixtures_root, allowed_suites, &mut ids, &mut errors);
    }

    finish(&errors)
}

pub(crate) fn validate_fixture_manifest_text(
    name: &str,
    text: &str,
    allowed_suites: &[&str],
) -> Vec<String> {
    let table = match text.parse::<Table>() {
        Ok(table) => table,
        Err(error) => return vec![format!("{name}: invalid TOML: {error}")],
    };
    let mut errors = Vec::new();

    validate_root_fields(name, &table, &mut errors);
    validate_identity(name, &table, allowed_suites, &mut errors);
    validate_file_list(name, &table, &mut errors);
    validate_provenance(name, &table, &mut errors);
    validate_named_table(name, &table, "environment", "description", &mut errors);
    validate_named_table(name, &table, "expectations", "summary", &mut errors);

    errors
}

fn validate_manifest_file(
    path: &Path,
    fixtures_root: &Path,
    allowed_suites: &[&str],
    ids: &mut BTreeSet<String>,
    errors: &mut Vec<String>,
) {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            errors.push(format!("failed to read {}: {error}", path.display()));
            return;
        }
    };

    errors.extend(validate_fixture_manifest_text(
        &path.display().to_string(),
        &text,
        allowed_suites,
    ));

    let Ok(table) = text.parse::<Table>() else {
        return;
    };
    validate_location(path, fixtures_root, &table, errors);
    validate_referenced_files(path, &table, errors);

    if let Some(id) = string_field(&table, "id") {
        if !ids.insert(id.to_owned()) {
            errors.push(format!("{}: duplicate fixture id `{id}`", path.display()));
        }
    }
}

fn validate_root_fields(name: &str, table: &Table, errors: &mut Vec<String>) {
    for key in table.keys() {
        if !ROOT_FIELDS.contains(&key.as_str()) {
            errors.push(format!("{name}: unknown root field `{key}`"));
        }
    }

    if table.get("schema").and_then(Value::as_integer) != Some(1) {
        errors.push(format!("{name}: `schema` must be integer 1"));
    }
    if table.get("secrets_reviewed").and_then(Value::as_bool) != Some(true) {
        errors.push(format!("{name}: `secrets_reviewed` must be true"));
    }
    require_string(name, table, "description", errors);
}

fn validate_identity(name: &str, table: &Table, allowed_suites: &[&str], errors: &mut Vec<String>) {
    match string_field(table, "id") {
        Some(id) if is_slug(id) => {}
        Some(_) => errors.push(format!("{name}: `id` must be a lowercase ASCII slug")),
        None => errors.push(format!("{name}: `id` must be a non-empty string")),
    }

    match string_field(table, "suite") {
        Some(suite) if allowed_suites.contains(&suite) => {}
        Some(suite) => errors.push(format!("{name}: unsupported suite `{suite}`")),
        None => errors.push(format!("{name}: `suite` must be a non-empty string")),
    }
}

fn validate_file_list(name: &str, table: &Table, errors: &mut Vec<String>) {
    let Some(files) = table.get("files").and_then(Value::as_array) else {
        errors.push(format!("{name}: `files` must be a non-empty array"));
        return;
    };
    if files.is_empty() {
        errors.push(format!("{name}: `files` must not be empty"));
    }

    let mut unique = BTreeSet::new();
    for value in files {
        let Some(file) = value.as_str() else {
            errors.push(format!("{name}: every `files` entry must be a string"));
            continue;
        };
        if !is_safe_relative_path(file) {
            errors.push(format!("{name}: unsafe fixture path `{file}`"));
        }
        if !unique.insert(file) {
            errors.push(format!("{name}: duplicate fixture path `{file}`"));
        }
    }
}

fn validate_provenance(name: &str, table: &Table, errors: &mut Vec<String>) {
    let Some(provenance) = table.get("provenance").and_then(Value::as_table) else {
        errors.push(format!("{name}: `provenance` must be a table"));
        return;
    };

    require_string(name, provenance, "license", errors);
    require_string(name, provenance, "modifications", errors);
    match string_field(provenance, "redistribution") {
        Some("allowed" | "forbidden" | "not-applicable") => {}
        _ => errors.push(format!(
            "{name}: `provenance.redistribution` must be allowed, forbidden, or not-applicable"
        )),
    }

    match string_field(provenance, "source") {
        Some("authored") => {}
        Some("external") => {
            require_string(name, provenance, "url", errors);
            require_string(name, provenance, "revision", errors);
        }
        Some("generated") => validate_oracle(name, table, errors),
        _ => errors.push(format!(
            "{name}: `provenance.source` must be authored, external, or generated"
        )),
    }
}

fn validate_oracle(name: &str, table: &Table, errors: &mut Vec<String>) {
    let Some(oracle) = table.get("oracle").and_then(Value::as_table) else {
        errors.push(format!(
            "{name}: generated fixtures require an `oracle` table"
        ));
        return;
    };
    for field in ["implementation", "version", "command"] {
        require_string(name, oracle, field, errors);
    }
}

fn validate_named_table(
    name: &str,
    table: &Table,
    table_name: &str,
    field: &str,
    errors: &mut Vec<String>,
) {
    let Some(nested) = table.get(table_name).and_then(Value::as_table) else {
        errors.push(format!("{name}: `{table_name}` must be a table"));
        return;
    };
    require_string(name, nested, field, errors);
}

fn validate_location(path: &Path, fixtures_root: &Path, table: &Table, errors: &mut Vec<String>) {
    let Some(parent) = path.parent() else {
        return;
    };
    let directory_id = parent.file_name().and_then(|value| value.to_str());
    let suite_directory = parent
        .parent()
        .and_then(Path::file_name)
        .and_then(|value| value.to_str());

    if directory_id != string_field(table, "id") {
        errors.push(format!(
            "{}: fixture id must match its directory under {}",
            path.display(),
            fixtures_root.display()
        ));
    }
    if suite_directory != string_field(table, "suite") {
        errors.push(format!(
            "{}: suite must match its parent directory",
            path.display()
        ));
    }
}

fn validate_referenced_files(path: &Path, table: &Table, errors: &mut Vec<String>) {
    let Some(directory) = path.parent() else {
        return;
    };
    let Some(files) = table.get("files").and_then(Value::as_array) else {
        return;
    };

    for file in files.iter().filter_map(Value::as_str) {
        if is_safe_relative_path(file) && !directory.join(file).is_file() {
            errors.push(format!(
                "{}: listed fixture file `{file}` does not exist",
                path.display()
            ));
        }
    }
}

fn collect_manifests(directory: &Path, paths: &mut Vec<PathBuf>) -> Result<(), String> {
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
            collect_manifests(&path, paths)?;
        } else if path.file_name().is_some_and(|name| name == "fixture.toml") {
            paths.push(path);
        }
    }

    Ok(())
}

fn require_string(name: &str, table: &Table, field: &str, errors: &mut Vec<String>) {
    if string_field(table, field).is_none() {
        errors.push(format!("{name}: `{field}` must be a non-empty string"));
    }
}

fn string_field<'a>(table: &'a Table, field: &str) -> Option<&'a str> {
    table
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
}

fn is_slug(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn is_safe_relative_path(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn finish(errors: &[String]) -> Result<(), String> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}
