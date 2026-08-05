//! Validation for the pinned real-world Compose catalogue.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Component, Path};
use toml::{Table, Value};

const ROOT_FIELDS: &[&str] = &["projects", "reviewed", "schema"];
const PROJECT_FIELDS: &[&str] = &[
    "blob_sha",
    "compose_fields",
    "goals",
    "id",
    "license",
    "minimum_services",
    "path",
    "repository",
    "revision",
    "tier",
];
const LICENSES: &[&str] = &["AGPL-3.0-only", "Apache-2.0", "BSD-3-Clause", "GPL-3.0-only", "MIT"];

pub(crate) fn validate_real_world_compose_catalog(repository_root: &Path) -> Result<(), String> {
    let path = repository_root.join("fixtures/real-world/corpus.toml");
    let text = fs::read_to_string(&path).map_err(|error| format!("failed to read {}: {error}", path.display()))?;
    let table = text
        .parse::<Table>()
        .map_err(|error| format!("{}: invalid TOML: {error}", path.display()))?;
    let name = path.display().to_string();
    let mut errors = Vec::new();

    validate_root(&name, &table, &mut errors);
    let Some(projects) = table.get("projects").and_then(Value::as_array) else {
        errors.push(format!("{name}: `projects` must be a non-empty array of tables"));
        return finish(&errors);
    };
    if projects.is_empty() {
        errors.push(format!("{name}: `projects` must not be empty"));
    }

    let mut ids = BTreeSet::new();
    let mut sources = BTreeSet::new();
    let mut tiers = BTreeSet::new();
    let mut previous_order = None;
    for (index, value) in projects.iter().enumerate() {
        let Some(project) = value.as_table() else {
            errors.push(format!("{name}: projects[{index}] must be a table"));
            continue;
        };
        validate_project(
            &name,
            index,
            project,
            &mut ids,
            &mut sources,
            &mut tiers,
            &mut previous_order,
            &mut errors,
        );
    }

    for tier in ["baseline", "migration", "stress"] {
        if !tiers.contains(tier) {
            errors.push(format!("{name}: corpus must contain the `{tier}` tier"));
        }
    }
    finish(&errors)
}

fn validate_root(name: &str, table: &Table, errors: &mut Vec<String>) {
    for key in table.keys() {
        if !ROOT_FIELDS.contains(&key.as_str()) {
            errors.push(format!("{name}: unknown root field `{key}`"));
        }
    }
    if table.get("schema").and_then(Value::as_integer) != Some(1) {
        errors.push(format!("{name}: `schema` must be integer 1"));
    }
    match string_field(table, "reviewed") {
        Some(reviewed) if valid_date(reviewed) => {}
        _ => errors.push(format!("{name}: `reviewed` must use YYYY-MM-DD")),
    }
}

#[allow(clippy::too_many_arguments)]
fn validate_project(
    name: &str,
    index: usize,
    project: &Table,
    ids: &mut BTreeSet<String>,
    sources: &mut BTreeSet<(String, String)>,
    tiers: &mut BTreeSet<String>,
    previous_order: &mut Option<(u8, String)>,
    errors: &mut Vec<String>,
) {
    let subject = format!("{name}: projects[{index}]");
    for key in project.keys() {
        if !PROJECT_FIELDS.contains(&key.as_str()) {
            errors.push(format!("{subject}: unknown field `{key}`"));
        }
    }

    let id = required_string(&subject, project, "id", errors);
    if let Some(id) = id {
        if !valid_slug(id) {
            errors.push(format!("{subject}: `id` must be a lowercase ASCII slug"));
        }
        if !ids.insert(id.to_owned()) {
            errors.push(format!("{subject}: duplicate id `{id}`"));
        }
    }

    let tier = required_string(&subject, project, "tier", errors);
    let tier_rank = match tier {
        Some("baseline") => Some(0),
        Some("migration") => Some(1),
        Some("stress") => Some(2),
        Some(value) => {
            errors.push(format!("{subject}: unsupported tier `{value}`"));
            None
        }
        None => None,
    };
    if let Some(tier) = tier {
        tiers.insert(tier.to_owned());
    }
    if let (Some(rank), Some(id)) = (tier_rank, id) {
        let order = (rank, id.to_owned());
        if previous_order.as_ref().is_some_and(|previous| previous >= &order) {
            errors.push(format!("{subject}: projects must be ordered by tier and id"));
        }
        *previous_order = Some(order);
    }

    let repository = required_string(&subject, project, "repository", errors);
    if repository.is_some_and(|value| !valid_repository(value)) {
        errors.push(format!("{subject}: `repository` must use owner/name form"));
    }
    let path = required_string(&subject, project, "path", errors);
    if path.is_some_and(|value| !safe_relative_path(value)) {
        errors.push(format!("{subject}: `path` must be a safe relative repository path"));
    }
    if let (Some(repository), Some(path)) = (repository, path) {
        if !sources.insert((repository.to_owned(), path.to_owned())) {
            errors.push(format!("{subject}: duplicate repository path `{repository}/{path}`"));
        }
    }

    for field in ["revision", "blob_sha"] {
        if required_string(&subject, project, field, errors).is_some_and(|value| !valid_git_sha(value)) {
            errors.push(format!("{subject}: `{field}` must be a full lowercase Git SHA"));
        }
    }
    match required_string(&subject, project, "license", errors) {
        Some(license) if LICENSES.contains(&license) => {}
        Some(license) => errors.push(format!("{subject}: unreviewed SPDX license `{license}`")),
        None => {}
    }
    if project
        .get("minimum_services")
        .and_then(Value::as_integer)
        .is_none_or(|value| value <= 0)
    {
        errors.push(format!("{subject}: `minimum_services` must be a positive integer"));
    }
    validate_string_array(&subject, project, "compose_fields", errors);
    validate_string_array(&subject, project, "goals", errors);
}

fn validate_string_array(subject: &str, table: &Table, field: &str, errors: &mut Vec<String>) {
    let Some(values) = table.get(field).and_then(Value::as_array) else {
        errors.push(format!("{subject}: `{field}` must be a non-empty string array"));
        return;
    };
    if values.is_empty() {
        errors.push(format!("{subject}: `{field}` must not be empty"));
    }
    let mut unique = BTreeSet::new();
    for value in values {
        let Some(value) = value.as_str().filter(|value| !value.is_empty()) else {
            errors.push(format!("{subject}: `{field}` values must be non-empty strings"));
            continue;
        };
        if !unique.insert(value) {
            errors.push(format!("{subject}: duplicate `{field}` value `{value}`"));
        }
    }
}

fn required_string<'a>(subject: &str, table: &'a Table, field: &str, errors: &mut Vec<String>) -> Option<&'a str> {
    let value = string_field(table, field);
    if value.is_none() {
        errors.push(format!("{subject}: `{field}` must be a non-empty string"));
    }
    value
}

fn string_field<'a>(table: &'a Table, field: &str) -> Option<&'a str> {
    table
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
}

fn valid_slug(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn valid_repository(value: &str) -> bool {
    let mut components = value.split('/');
    let valid_component = |component: &str| {
        !component.is_empty()
            && component
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    };
    matches!(
        (components.next(), components.next(), components.next()),
        (Some(owner), Some(repository), None) if valid_component(owner) && valid_component(repository)
    )
}

fn safe_relative_path(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn valid_git_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn valid_date(value: &str) -> bool {
    value.len() == 10
        && value.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 4 | 7) {
                byte == b'-'
            } else {
                byte.is_ascii_digit()
            }
        })
}

fn finish(errors: &[String]) -> Result<(), String> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("\n"))
    }
}
