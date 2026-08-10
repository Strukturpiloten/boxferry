//! Opt-in ingestion checks for immutable upstream Compose projects.

#![cfg(feature = "compose")]

use std::error::Error;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use boxferry::compose::compose_lens::{
    loader::{DocumentInput, DocumentOrigin, LoadedProject},
    merge::merge_project,
    profiles::{ProfileRequest, select_profiles},
    source::SourceId as ComposeSourceId,
};
use boxferry::{ComposeImporter, ComposeSource, ConversionKind, Identifier, ImportAdapter, Severity, SourceId};
use toml::{Table, Value};

#[derive(Debug)]
struct CorpusProject {
    id: String,
    repository: String,
    revision: String,
    path: String,
    blob_sha: String,
    minimum_services: usize,
    compose_fields: Vec<String>,
}

impl CorpusProject {
    fn raw_url(&self) -> String {
        format!(
            "https://raw.githubusercontent.com/{}/{}/{}",
            self.repository, self.revision, self.path
        )
    }
}

#[test]
#[ignore = "downloads immutable public GitHub files; run with cargo ci-real-world-compose"]
fn pinned_real_world_projects_load_merge_and_reach_the_importer() -> Result<(), Box<dyn Error>> {
    let adapter = ComposeImporter::new()?;
    for (index, project) in load_catalog()?.into_iter().enumerate() {
        let url = project.raw_url();
        let source = download(&url)?;
        let actual_blob_sha = git_blob_sha(&source)?;
        if actual_blob_sha != project.blob_sha {
            return Err(format!(
                "{}: expected blob {}, received {}",
                project.id, project.blob_sha, actual_blob_sha
            )
            .into());
        }
        for field in &project.compose_fields {
            if !contains_compose_field(&source, field) {
                return Err(format!("{}: expected Compose field `{field}` was not found", project.id).into());
            }
        }

        let numeric_id = u32::try_from(index + 1)?;
        let compose_id = ComposeSourceId::new(numeric_id);
        let origin = DocumentOrigin::new(
            url.clone(),
            PathBuf::from(format!("/boxferry-real-world/{}", project.id)),
        );
        let loaded = LoadedProject::load(vec![DocumentInput::new(compose_id, origin, source)])?;
        if !loaded.is_valid() {
            return Err(format!("{}: loading failed: {:#?}", project.id, loaded.diagnostics()).into());
        }
        let merged = merge_project(&loaded, None);
        if !merged.is_valid() {
            return Err(format!("{}: merge failed: {:#?}", project.id, merged.diagnostics()).into());
        }
        let native_project = merged
            .project()
            .ok_or_else(|| format!("{}: merge returned no project", project.id))?
            .clone();
        let selection = select_profiles(&native_project, &ProfileRequest::all());
        if !selection.is_valid() {
            return Err(format!(
                "{}: profile selection failed: {:#?}",
                project.id,
                selection.diagnostics()
            )
            .into());
        }

        let source_id = SourceId::new(url)?;
        let compose_source = ComposeSource::new(native_project, Identifier::new(project.id.clone())?)?
            .with_source_id(compose_id, source_id)
            .with_profile_selection(selection);
        let result = adapter.import(&compose_source);
        let mut kinds = [0_usize; 5];
        for outcome in result.outcomes() {
            kinds[match outcome.kind() {
                ConversionKind::Exact => 0,
                ConversionKind::Approximate => 1,
                ConversionKind::Unsupported => 2,
                ConversionKind::Invalid => 3,
                _ => 4,
            }] += 1;
        }
        let error_diagnostics = result
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.severity() == Severity::Error)
            .count();
        let application = result
            .application()
            .ok_or_else(|| format!("{}: importer returned no application", project.id))?;
        if application.services().len() < project.minimum_services {
            return Err(format!(
                "{}: expected at least {} services, imported {}",
                project.id,
                project.minimum_services,
                application.services().len()
            )
            .into());
        }

        println!(
            "{}: {} services; outcomes exact={}, approximate={}, unsupported={}, invalid={}, future={}; import errors={}",
            project.id,
            application.services().len(),
            kinds[0],
            kinds[1],
            kinds[2],
            kinds[3],
            kinds[4],
            error_diagnostics
        );
    }
    Ok(())
}

fn load_catalog() -> Result<Vec<CorpusProject>, Box<dyn Error>> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/real-world/corpus.toml");
    let table = std::fs::read_to_string(path)?.parse::<Table>()?;
    let projects = table
        .get("projects")
        .and_then(Value::as_array)
        .ok_or("catalog projects array missing")?;
    projects
        .iter()
        .map(|value| {
            let table = value.as_table().ok_or("catalog project must be a table")?;
            Ok(CorpusProject {
                id: string(table, "id")?.to_owned(),
                repository: string(table, "repository")?.to_owned(),
                revision: string(table, "revision")?.to_owned(),
                path: string(table, "path")?.to_owned(),
                blob_sha: string(table, "blob_sha")?.to_owned(),
                minimum_services: positive_usize(table, "minimum_services")?,
                compose_fields: strings(table, "compose_fields")?,
            })
        })
        .collect()
}

fn string<'a>(table: &'a Table, field: &str) -> Result<&'a str, Box<dyn Error>> {
    table
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("catalog field `{field}` must be a non-empty string").into())
}

fn strings(table: &Table, field: &str) -> Result<Vec<String>, Box<dyn Error>> {
    table
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("catalog field `{field}` must be an array"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .ok_or_else(|| format!("catalog field `{field}` must contain non-empty strings").into())
        })
        .collect()
}

fn positive_usize(table: &Table, field: &str) -> Result<usize, Box<dyn Error>> {
    let value = table
        .get(field)
        .and_then(Value::as_integer)
        .ok_or_else(|| format!("catalog field `{field}` must be an integer"))?;
    usize::try_from(value).map_err(Into::into)
}

fn download(url: &str) -> Result<String, Box<dyn Error>> {
    let output = Command::new("curl")
        .args([
            "--disable",
            "--fail",
            "--location",
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--silent",
            "--show-error",
            "--max-time",
            "30",
            "--max-filesize",
            "1048576",
            url,
        ])
        .output()?;
    if !output.status.success() {
        return Err(format!(
            "curl failed for {url} with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn git_blob_sha(source: &str) -> Result<String, Box<dyn Error>> {
    let mut child = Command::new("git")
        .args(["hash-object", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or("git stdin unavailable")?
        .write_all(source.as_bytes())?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(format!(
            "git hash-object failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn contains_compose_field(source: &str, field: &str) -> bool {
    let prefix = format!("{field}:");
    source.lines().any(|line| line.trim_start().starts_with(&prefix))
}
