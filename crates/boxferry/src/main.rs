//! Command-line interface for `BoxFerry`.

use std::{
    collections::BTreeSet,
    env,
    error::Error,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    str::FromStr,
};

use boxferry::compose::compose_lens::{
    diagnostic::Diagnostic as ComposeDiagnostic,
    interpolation::MapEnvironment,
    loader::{DocumentInput, DocumentOrigin, LoadedProject},
    merge::merge_project,
    profiles::{ProfileRequest, select_profiles},
    source::SourceId as ComposeSourceId,
};
use boxferry::{
    ComposeImporter, ComposeSource, Diagnostic, Identifier, LossPolicy, PlatformVersion, QuadletExporter,
    QuadletGroupingPolicy, SourceId, TargetProfile, convert,
};
use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(version, about = "Loss-aware container-definition conversion")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Convert one explicitly ordered Compose project into Quadlet files.
    ComposeToQuadlet(ComposeToQuadlet),
}

#[derive(Debug, Args)]
struct ComposeToQuadlet {
    /// Compose files in merge order; repeat for overrides.
    #[arg(short = 'f', long = "file", required = true)]
    files: Vec<PathBuf>,

    /// Fallback application name when the Compose project has no top-level name.
    #[arg(long)]
    project_name: String,

    /// Activate one Compose profile; repeat to activate more than one.
    #[arg(long = "profile", conflicts_with = "all_profiles")]
    profiles: Vec<String>,

    /// Activate every valid profile declared by the project.
    #[arg(long)]
    all_profiles: bool,

    /// Resolve Compose interpolation using only explicitly supplied variables and defaults.
    #[arg(long)]
    interpolate: bool,

    /// Supply one non-sensitive interpolation variable as NAME=VALUE; repeat as needed.
    #[arg(long = "variable", value_name = "NAME=VALUE", requires = "interpolate")]
    variables: Vec<VariableAssignment>,

    /// Read one explicitly named process variable as sensitive interpolation input; repeat as needed.
    #[arg(
        long = "variable-from-environment",
        value_name = "NAME",
        requires = "interpolate",
        value_parser = parse_variable_name
    )]
    environment_variables: Vec<String>,

    /// Compatibility floor for generated Quadlet files.
    #[arg(long, default_value = "5.4.0")]
    podman_minimum_version: PlatformVersion,

    /// Optional inclusive compatibility ceiling.
    #[arg(long)]
    podman_maximum_version: Option<PlatformVersion>,

    /// Authorization for documented non-exact conversion outcomes.
    #[arg(long, value_enum, default_value_t = CliLossPolicy::Exact)]
    loss_policy: CliLossPolicy,

    /// Keep services separate or explicitly request one shared Podman pod.
    #[arg(long, value_enum, default_value_t = Grouping::Separate)]
    grouping: Grouping,

    /// New directory that will receive the generated Quadlet files.
    #[arg(long)]
    output_directory: PathBuf,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum CliLossPolicy {
    Exact,
    Approximate,
    Partial,
}

impl From<CliLossPolicy> for LossPolicy {
    fn from(value: CliLossPolicy) -> Self {
        match value {
            CliLossPolicy::Exact => Self::ExactOnly,
            CliLossPolicy::Approximate => Self::AllowApproximate,
            CliLossPolicy::Partial => Self::AllowPartial,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Grouping {
    Separate,
    Pod,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct VariableAssignment {
    name: String,
    value: String,
}

impl FromStr for VariableAssignment {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (name, value) = value
            .split_once('=')
            .ok_or_else(|| "interpolation variable must use NAME=VALUE form".to_owned())?;
        Ok(Self {
            name: parse_variable_name(name)?,
            value: value.to_owned(),
        })
    }
}

impl From<Grouping> for QuadletGroupingPolicy {
    fn from(value: Grouping) -> Self {
        match value {
            Grouping::Separate => Self::SeparateContainers,
            Grouping::Pod => Self::SinglePod,
        }
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("boxferry: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode, Box<dyn Error>> {
    match cli.command {
        Command::ComposeToQuadlet(arguments) => convert_compose_to_quadlet(arguments),
    }
}

fn convert_compose_to_quadlet(arguments: ComposeToQuadlet) -> Result<ExitCode, Box<dyn Error>> {
    let project_root = absolute_parent(&arguments.files[0])?;
    let mut source_identities = Vec::with_capacity(arguments.files.len());
    let mut inputs = Vec::with_capacity(arguments.files.len());
    for (index, path) in arguments.files.iter().enumerate() {
        let numeric_id = u32::try_from(index + 1).map_err(|_| io::Error::other("too many Compose input files"))?;
        let compose_id = ComposeSourceId::new(numeric_id);
        let label = path.display().to_string();
        let directory = absolute_parent(path)?;
        let text = fs::read_to_string(path)?;
        inputs.push(DocumentInput::new(
            compose_id,
            DocumentOrigin::new(label.clone(), directory),
            text,
        ));
        source_identities.push((compose_id, SourceId::new(label)?));
    }

    let loaded = LoadedProject::load(inputs)?;
    let interpolation_environment = interpolation_environment(&arguments)?;
    let interpolation = interpolation_environment
        .as_ref()
        .map(|environment| loaded.interpolate(environment));
    let merged = merge_project(&loaded, interpolation.as_ref());
    if !merged.is_valid() {
        print_compose_diagnostics(merged.diagnostics());
        return Ok(ExitCode::from(2));
    }
    let project = merged
        .project()
        .ok_or_else(|| io::Error::other("Compose merge produced no project"))?
        .clone();
    let profile_request = profile_request(&arguments);
    let selection = select_profiles(&project, &profile_request);
    if !selection.is_valid() {
        print_compose_diagnostics(selection.diagnostics());
        return Ok(ExitCode::from(2));
    }

    let mut source = ComposeSource::new(project, Identifier::new(arguments.project_name)?)?;
    for (compose, neutral) in source_identities {
        source = source.with_source_id(compose, neutral);
    }
    source = source.with_profile_selection(selection);

    let importer = ComposeImporter::new()?;
    let exporter = QuadletExporter::new()?
        .with_relative_host_path_root(project_root.to_string_lossy().into_owned())?
        .with_grouping_policy(arguments.grouping.into());
    let target = TargetProfile::new(
        "podman",
        arguments.podman_minimum_version,
        arguments.podman_maximum_version,
    )?;
    let result = convert(&importer, &source, &exporter, &target, arguments.loss_policy.into())?;
    print_diagnostics(result.diagnostics());
    let Some(output) = result.output() else {
        eprintln!("boxferry: output blocked by the selected loss policy");
        return Ok(ExitCode::from(2));
    };

    write_new_output_directory(&arguments.output_directory, output.files())?;
    for file in output.files() {
        println!("{}", arguments.output_directory.join(file.name().as_str()).display());
    }
    Ok(ExitCode::SUCCESS)
}

fn interpolation_environment(arguments: &ComposeToQuadlet) -> Result<Option<MapEnvironment>, Box<dyn Error>> {
    if !arguments.interpolate {
        return Ok(None);
    }

    let mut names = BTreeSet::new();
    let mut environment = MapEnvironment::new();
    for variable in &arguments.variables {
        register_variable_name(&mut names, &variable.name)?;
        let _ = environment.insert(variable.name.clone(), variable.value.clone());
    }
    for name in &arguments.environment_variables {
        register_variable_name(&mut names, name)?;
        let value = env::var(name).map_err(|error| match error {
            env::VarError::NotPresent => io::Error::new(
                io::ErrorKind::NotFound,
                format!("authorized interpolation variable `{name}` is not present in the process environment"),
            ),
            env::VarError::NotUnicode(_) => io::Error::new(
                io::ErrorKind::InvalidData,
                format!("authorized interpolation variable `{name}` is not valid Unicode"),
            ),
        })?;
        let _ = environment.insert_sensitive(name.clone(), value);
    }
    Ok(Some(environment))
}

fn register_variable_name(names: &mut BTreeSet<String>, name: &str) -> io::Result<()> {
    if names.insert(name.to_owned()) {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("interpolation variable `{name}` was supplied more than once"),
        ))
    }
}

fn parse_variable_name(value: &str) -> Result<String, String> {
    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return Err("interpolation variable name must not be empty".to_owned());
    };
    if first != b'_' && !first.is_ascii_alphabetic() {
        return Err(format!(
            "interpolation variable name `{value}` must start with an ASCII letter or underscore"
        ));
    }
    if !bytes.all(|byte| byte == b'_' || byte.is_ascii_alphanumeric()) {
        return Err(format!(
            "interpolation variable name `{value}` may contain only ASCII letters, digits, and underscores"
        ));
    }
    Ok(value.to_owned())
}

fn profile_request(arguments: &ComposeToQuadlet) -> ProfileRequest {
    if arguments.all_profiles {
        ProfileRequest::all()
    } else {
        arguments
            .profiles
            .iter()
            .fold(ProfileRequest::new(), ProfileRequest::with_profile)
    }
}

fn absolute_parent(path: &Path) -> io::Result<PathBuf> {
    let absolute = fs::canonicalize(path)?;
    absolute
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| io::Error::other(format!("input path has no parent: {}", path.display())))
}

fn write_new_output_directory(directory: &Path, files: &[boxferry::QuadletFile]) -> io::Result<()> {
    if let Err(error) = fs::create_dir(directory) {
        if error.kind() == io::ErrorKind::AlreadyExists {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("output directory already exists: {}", directory.display()),
            ));
        }
        return Err(error);
    }
    let mut created = Vec::with_capacity(files.len());
    for file in files {
        let path = directory.join(file.name().as_str());
        let written = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .and_then(|mut destination| destination.write_all(file.text().as_bytes()));
        if let Err(error) = written {
            for created_path in &created {
                let _ = fs::remove_file(created_path);
            }
            let _ = fs::remove_dir(directory);
            return Err(error);
        }
        created.push(path);
    }
    Ok(())
}

fn print_compose_diagnostics(diagnostics: &[ComposeDiagnostic]) {
    for diagnostic in diagnostics {
        eprintln!("{}: {}", diagnostic.code(), diagnostic.message());
    }
}

fn print_diagnostics(diagnostics: &[Diagnostic]) {
    for diagnostic in diagnostics {
        eprintln!("{diagnostic}");
    }
}
