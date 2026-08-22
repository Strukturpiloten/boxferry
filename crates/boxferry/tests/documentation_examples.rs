//! Executable contracts for every `BoxFerry` command published in the user documentation.

#![cfg(all(feature = "cli", feature = "compose", feature = "podman", feature = "quadlet"))]

#[path = "support/podman_cassette.rs"]
#[allow(dead_code, reason = "shared support also exposes route-matrix mutation helpers")]
mod podman_cassette;

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use serde::Deserialize;

use podman_cassette::{PodmanCassette, PodmanCassetteServer};

static TEMPORARY_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct DocumentationManifest {
    schema: u8,
    fixture_directory: PathBuf,
    examples: Vec<DocumentationExample>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct DocumentationExample {
    id: String,
    pages: Vec<PathBuf>,
    command: String,
    args: Vec<String>,
    expected_exit: i32,
    #[serde(default)]
    copy: Vec<PathBuf>,
    #[serde(default)]
    copy_as: Vec<CopyFile>,
    #[serde(default)]
    mkdir: Vec<PathBuf>,
    #[serde(default)]
    stdout_contains: Vec<String>,
    #[serde(default)]
    stderr_contains: Vec<String>,
    #[serde(default)]
    expected_files: Vec<ExpectedFile>,
    #[serde(default)]
    generated_files: Vec<GeneratedFile>,
    #[serde(default)]
    absent: Vec<PathBuf>,
    podman_cassette: Option<PathBuf>,
    podman_socket_placeholder: Option<String>,
    #[serde(default)]
    artifact_sets: Vec<ArtifactSet>,
    #[serde(default)]
    artifact_contains: Vec<ArtifactContains>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedFile {
    actual: PathBuf,
    fixture: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CopyFile {
    source: PathBuf,
    destination: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GeneratedFile {
    directory: PathBuf,
    prefix: String,
    suffix: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactSet {
    directory: PathBuf,
    files: BTreeSet<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArtifactContains {
    actual: PathBuf,
    #[serde(default)]
    contains: Vec<String>,
    #[serde(default)]
    excludes: Vec<String>,
}

#[test]
fn documented_commands_match_the_checked_manifest() -> Result<(), Box<dyn Error>> {
    let repository = repository_root()?;
    let manifest = load_manifest(&repository)?;
    let examples = example_map(&manifest)?;
    let mut seen = BTreeSet::new();

    for example in &manifest.examples {
        let expected = rendered_example(example);
        for page in &example.pages {
            let text = fs::read_to_string(repository.join(page))?;
            let occurrences = text.match_indices(&expected).count();
            assert_eq!(
                occurrences,
                1,
                "{} must contain exactly one checked `{}` command block",
                page.display(),
                example.id
            );
        }
    }

    for page in markdown_files(&repository.join("docs/public"))? {
        let text = fs::read_to_string(&page)?;
        let lines = text.lines().collect::<Vec<_>>();
        for (index, line) in lines.iter().enumerate() {
            if *line != "```console" {
                continue;
            }
            let Some(command) = lines.get(index + 1).filter(|value| value.starts_with("boxferry ")) else {
                continue;
            };
            assert_eq!(
                lines.get(index + 2),
                Some(&"```"),
                "{} contains a multi-line BoxFerry command; keep checked commands on one copyable line",
                page.display()
            );
            let marker = lines[..index]
                .iter()
                .rev()
                .find(|value| !value.trim().is_empty())
                .and_then(|value| example_id(value))
                .ok_or_else(|| {
                    std::io::Error::other(format!(
                        "{} contains an unchecked BoxFerry command: {command}",
                        page.display()
                    ))
                })?;
            let example = examples.get(marker).ok_or_else(|| {
                std::io::Error::other(format!("{} references unknown example `{marker}`", page.display()))
            })?;
            assert_eq!(*command, example.command, "{} command drifted", page.display());
            seen.insert(marker.to_owned());
        }
    }

    assert_eq!(
        seen,
        examples.keys().cloned().collect(),
        "every manifest command must appear in the public documentation"
    );
    Ok(())
}

#[test]
fn every_documented_command_executes_with_its_reviewed_contract() -> Result<(), Box<dyn Error>> {
    let repository = repository_root()?;
    let manifest = load_manifest(&repository)?;
    let fixture_directory = repository.join(&manifest.fixture_directory);
    let binary = env!("CARGO_BIN_EXE_boxferry");
    let mut successful_routes = BTreeSet::new();
    let mut failing_routes = BTreeSet::new();

    for example in &manifest.examples {
        let temporary = TemporaryDirectory::new(&example.id)?;
        for source in &example.copy {
            let destination = temporary.path().join(source);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(fixture_directory.join(source), destination)?;
        }
        for source in &example.copy_as {
            let destination = temporary.path().join(&source.destination);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(fixture_directory.join(&source.source), destination)?;
        }
        for directory in &example.mkdir {
            fs::create_dir_all(temporary.path().join(directory))?;
        }

        let cassette = example
            .podman_cassette
            .as_ref()
            .map(|path| PodmanCassette::load(&repository.join(path)))
            .transpose()?;
        let server = cassette.map(PodmanCassetteServer::start).transpose()?;
        let args = execution_args(example, server.as_ref())?;
        let output = Command::new(binary)
            .args(args)
            .current_dir(temporary.path())
            .env_remove("IMAGE")
            .env_remove("RESTART")
            .output()?;
        if let Some(server) = server {
            server.finish()?;
        }
        let stdout = String::from_utf8(output.stdout)?;
        let stderr = String::from_utf8(output.stderr)?;
        assert_eq!(
            output.status.code(),
            Some(example.expected_exit),
            "example `{}` exited unexpectedly\nstdout:\n{stdout}\nstderr:\n{stderr}",
            example.id
        );

        for expected in &example.stdout_contains {
            assert!(
                stdout.contains(expected),
                "example `{}` stdout omitted `{expected}`",
                example.id
            );
        }
        for expected in &example.stderr_contains {
            assert!(
                stderr.contains(expected),
                "example `{}` stderr omitted `{expected}`",
                example.id
            );
        }
        assert_artifact_contract(example, temporary.path(), &fixture_directory)?;

        if example.args.first().map(String::as_str) == Some("convert") && example.args.len() >= 3 {
            let route = format!("{}->{}", example.args[1], example.args[2]);
            if example.expected_exit == 0 {
                successful_routes.insert(route);
            } else {
                failing_routes.insert(route);
            }
        }
    }

    let expected_routes = BTreeSet::from([
        "compose->compose".to_owned(),
        "compose->podman".to_owned(),
        "compose->quadlet".to_owned(),
        "quadlet->compose".to_owned(),
        "quadlet->podman".to_owned(),
        "quadlet->quadlet".to_owned(),
        "podman->compose".to_owned(),
        "podman->podman".to_owned(),
        "podman->quadlet".to_owned(),
    ]);
    assert_eq!(successful_routes, expected_routes);
    let expected_failing_routes = BTreeSet::from([
        "compose->compose".to_owned(),
        "compose->quadlet".to_owned(),
        "podman->compose".to_owned(),
        "quadlet->compose".to_owned(),
        "quadlet->quadlet".to_owned(),
    ]);
    assert_eq!(failing_routes, expected_failing_routes);
    Ok(())
}

fn assert_artifact_contract(
    example: &DocumentationExample,
    temporary: &Path,
    fixture_directory: &Path,
) -> Result<(), Box<dyn Error>> {
    for expected in &example.expected_files {
        assert_eq!(
            fs::read(temporary.join(&expected.actual))?,
            fs::read(fixture_directory.join(&expected.fixture))?,
            "example `{}` output `{}` drifted",
            example.id,
            expected.actual.display()
        );
    }
    for generated in &example.generated_files {
        let matches = fs::read_dir(temporary.join(&generated.directory))?
            .filter_map(Result::ok)
            .filter(|entry| {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                entry.path().is_file() && name.starts_with(&generated.prefix) && name.ends_with(&generated.suffix)
            })
            .count();
        assert_eq!(matches, 1, "example `{}` generated-file contract drifted", example.id);
    }
    for artifact_set in &example.artifact_sets {
        let actual = fs::read_dir(temporary.join(&artifact_set.directory))?
            .map(|entry| {
                entry?
                    .file_name()
                    .into_string()
                    .map_err(|name| std::io::Error::other(format!("non-UTF-8 artifact name: {name:?}")))
            })
            .collect::<Result<BTreeSet<_>, _>>()?;
        assert_eq!(
            actual,
            artifact_set.files,
            "example `{}` artifact set in `{}` drifted",
            example.id,
            artifact_set.directory.display()
        );
    }
    for artifact in &example.artifact_contains {
        let text = fs::read_to_string(temporary.join(&artifact.actual))?;
        for expected in &artifact.contains {
            assert!(
                text.contains(expected),
                "example `{}` artifact `{}` omitted `{expected}`",
                example.id,
                artifact.actual.display()
            );
        }
        for excluded in &artifact.excludes {
            assert!(
                !text.contains(excluded),
                "example `{}` artifact `{}` disclosed `{excluded}`",
                example.id,
                artifact.actual.display()
            );
        }
    }
    for absent in &example.absent {
        assert!(
            !temporary.join(absent).exists(),
            "example `{}` unexpectedly created `{}`",
            example.id,
            absent.display()
        );
    }
    Ok(())
}

fn execution_args(
    example: &DocumentationExample,
    server: Option<&PodmanCassetteServer>,
) -> Result<Vec<String>, Box<dyn Error>> {
    match (
        server,
        example.podman_socket_placeholder.as_deref(),
        example.podman_cassette.as_ref(),
    ) {
        (None, None, None) => Ok(example.args.clone()),
        (Some(server), Some(placeholder), Some(_)) => {
            let socket = server.socket().to_string_lossy();
            let mut replacements = 0;
            let args = example
                .args
                .iter()
                .map(|argument| {
                    if argument == placeholder {
                        replacements += 1;
                        socket.to_string()
                    } else {
                        argument.clone()
                    }
                })
                .collect::<Vec<_>>();
            if replacements != 1 {
                return Err(format!(
                    "example `{}` must contain its Podman socket placeholder exactly once",
                    example.id
                )
                .into());
            }
            Ok(args)
        }
        _ => Err(format!(
            "example `{}` must declare Podman cassette and socket placeholder together",
            example.id
        )
        .into()),
    }
}

#[test]
fn published_rule_data_matches_the_runtime_catalogue() -> Result<(), Box<dyn Error>> {
    let repository = repository_root()?;
    let output = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["rules", "--console-format", "json"])
        .output()?;
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let runtime: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let published: serde_json::Value = serde_json::from_slice(&fs::read(
        repository.join("docs/public/reference/diagnostics/rules.json"),
    )?)?;
    assert_eq!(
        runtime, published,
        "regenerate the published diagnostic catalogue after changing a rule"
    );
    Ok(())
}

fn load_manifest(repository: &Path) -> Result<DocumentationManifest, Box<dyn Error>> {
    let text = fs::read_to_string(repository.join("docs/documentation-examples.toml"))?;
    let manifest: DocumentationManifest = toml::from_str(&text)?;
    assert_eq!(manifest.schema, 1);
    assert!(!manifest.examples.is_empty());
    Ok(manifest)
}

fn example_map(manifest: &DocumentationManifest) -> Result<BTreeMap<String, &DocumentationExample>, Box<dyn Error>> {
    let mut examples = BTreeMap::new();
    for example in &manifest.examples {
        assert_eq!(
            example.command,
            format!("boxferry {}", example.args.join(" ")),
            "example `{}` command and argv differ",
            example.id
        );
        if examples.insert(example.id.clone(), example).is_some() {
            return Err(format!("duplicate documentation example `{}`", example.id).into());
        }
    }
    Ok(examples)
}

fn rendered_example(example: &DocumentationExample) -> String {
    format!(
        "<!-- boxferry-example: {} -->\n\n```console\n{}\n```",
        example.id, example.command
    )
}

fn example_id(line: &str) -> Option<&str> {
    line.strip_prefix("<!-- boxferry-example: ")?.strip_suffix(" -->")
}

fn markdown_files(directory: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            files.extend(markdown_files(&path)?);
        } else if path.extension().and_then(|value| value.to_str()) == Some("md") {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn repository_root() -> Result<PathBuf, std::io::Error> {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize()
}

struct TemporaryDirectory {
    path: PathBuf,
}

impl TemporaryDirectory {
    fn new(label: &str) -> Result<Self, Box<dyn Error>> {
        let id = TEMPORARY_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("boxferry-docs-{label}-{}-{id}", std::process::id()));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
