//! Executable contracts for every `BoxFerry` command published in the user documentation.

#![cfg(all(feature = "cli", feature = "compose", feature = "quadlet"))]

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use serde::Deserialize;

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
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedFile {
    actual: PathBuf,
    fixture: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GeneratedFile {
    directory: PathBuf,
    prefix: String,
    suffix: String,
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
        for directory in &example.mkdir {
            fs::create_dir_all(temporary.path().join(directory))?;
        }

        let output = Command::new(binary)
            .args(&example.args)
            .current_dir(temporary.path())
            .env_remove("IMAGE")
            .env_remove("RESTART")
            .output()?;
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
        for expected in &example.expected_files {
            assert_eq!(
                fs::read(temporary.path().join(&expected.actual))?,
                fs::read(fixture_directory.join(&expected.fixture))?,
                "example `{}` output `{}` drifted",
                example.id,
                expected.actual.display()
            );
        }
        for generated in &example.generated_files {
            let matches = fs::read_dir(temporary.path().join(&generated.directory))?
                .filter_map(Result::ok)
                .filter(|entry| {
                    let name = entry.file_name();
                    let name = name.to_string_lossy();
                    entry.path().is_file() && name.starts_with(&generated.prefix) && name.ends_with(&generated.suffix)
                })
                .count();
            assert_eq!(matches, 1, "example `{}` generated-file contract drifted", example.id);
        }
        for absent in &example.absent {
            assert!(
                !temporary.path().join(absent).exists(),
                "example `{}` unexpectedly created `{}`",
                example.id,
                absent.display()
            );
        }

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
        "compose->quadlet".to_owned(),
        "quadlet->compose".to_owned(),
        "quadlet->quadlet".to_owned(),
    ]);
    assert_eq!(successful_routes, expected_routes);
    assert_eq!(failing_routes, expected_routes);
    Ok(())
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
