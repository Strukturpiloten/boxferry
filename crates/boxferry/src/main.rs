//! Command-line interface for `BoxFerry`.

use std::{
    collections::BTreeSet,
    env,
    error::Error,
    fs::{self, OpenOptions},
    io::{self, Cursor, Read, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    str::FromStr,
    sync::atomic::{AtomicU64, Ordering},
};

use boxferry::compose::compose_lens::{
    interpolation::MapEnvironment,
    loader::{DocumentInput, DocumentOrigin, LoadedProject},
    merge::merge_project,
    profiles::{ProfileRequest, select_profiles},
    source::SourceId as ComposeSourceId,
};
use boxferry::report::{
    ConversionReport, DiscoveryDecision, ExitCategory, FailedStage, FidelityCounts, HostMetadata, OutputArtifact,
    ReportChoice, ReportDiagnostic, ReportField, ReportInput, ReportStatus, SanitizedInvocation, VersionBounds,
    redact_text,
};
use boxferry::{
    ComposeImporter, ComposeSource, ConversionKind, Diagnostic, Identifier, LossPolicy, PlatformVersion,
    QuadletExporter, QuadletGroupingPolicy, SourceId, TargetProfile, convert,
};
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum, parser::ValueSource};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

const CONVENTIONAL_COMPOSE_FILES: [&str; 6] = [
    "compose.yaml",
    "compose.yml",
    "podman-compose.yaml",
    "podman-compose.yml",
    "docker-compose.yaml",
    "docker-compose.yml",
];
const MAX_README_BYTES: usize = 128 * 1024;
const MAX_ARCHIVE_BYTES: usize = 5 * 1024 * 1024;
const SUPPORT_BUNDLE_README: &str = "# BoxFerry diagnostic support bundle\n\nreview_required: true\n\nInspect both files before uploading this archive. It is generated locally and BoxFerry never uploads it.\n\n## Contents\n\n- `report.json` is the complete structured diagnostic report.\n- This archive intentionally omits source and generated-file contents, environment values, runtime inspection data, raw panic payloads, backtraces, hostname, username, and the ambient process environment.\n\n## Attaching to an issue\n\nAfter review, create a GitHub issue using your normal browser or GitHub client, describe the problem, and attach this ZIP. Do not upload it if the review finds unwanted context. This archive contains no network instructions or automatic submission.\n";
static SUPPORT_BUNDLE_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Loss-aware container-definition conversion",
    propagate_version = true,
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Convert one explicit source type into one explicit target type.
    Convert(ConvertCommand),
    /// Parse and plan one explicit route without writing output.
    Validate(GenericConversion),
    /// List generic routes implemented by this build.
    Capabilities(Presentation),
    /// Show contextual command help.
    Help { command: Option<String> },
    /// Print `BoxFerry` version information.
    Version,
    /// Convert one explicitly ordered Compose project into Quadlet files.
    ComposeToQuadlet(ComposeToQuadlet),
}

#[derive(Clone, Copy, Debug, Args)]
struct Presentation {
    /// Include discovery and target-resolution detail in human output.
    #[arg(long, conflicts_with_all = ["quiet", "console_format"])]
    verbose: bool,
    /// Suppress human progress and success messages.
    #[arg(long, conflicts_with_all = ["verbose", "console_format"])]
    quiet: bool,
    /// Select machine-readable console output.
    #[arg(long, value_enum, conflicts_with_all = ["verbose", "quiet"])]
    console_format: Option<ConsoleFormat>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ConsoleFormat {
    Json,
}

#[derive(Debug, Args)]
struct GenericConversion {
    #[command(flatten)]
    presentation: Presentation,

    /// Write the complete privacy-safe structured result to a new file.
    #[arg(long, value_name = "PATH")]
    report_file: Option<PathBuf>,

    /// Write a locally reviewable ZIP with fixed README.md and report.json entries.
    #[arg(long, value_name = "PATH")]
    generate_error_report: Option<PathBuf>,

    /// Native format of every supplied input.
    #[arg(long, value_enum)]
    input_type: InputType,
    /// Native format to generate.
    #[arg(long, value_enum)]
    output_type: OutputType,
    /// Explicit input document in merge order; repeat as needed.
    #[arg(long = "input-file", value_name = "PATH")]
    input_files: Vec<PathBuf>,
    /// Directory that contributes one conventional Compose document at this position.
    #[arg(long = "input-directory", value_name = "PATH")]
    input_directories: Vec<PathBuf>,
    /// Explicit Compose project root for relative source paths.
    #[arg(long)]
    project_directory: Option<PathBuf>,
    /// Fallback name when the Compose document has no top-level name.
    #[arg(long)]
    project_name: Option<String>,
    /// Resolve Compose interpolation using only explicitly supplied variables.
    #[arg(long)]
    interpolate: bool,
    /// Strict `BoxFerry` interpolation assignments; later files override earlier files.
    #[arg(long = "env-file", value_name = "PATH", requires = "interpolate")]
    env_files: Vec<PathBuf>,
    /// Interpolation NAME=VALUE or authorized process NAME; repeat as needed.
    #[arg(
        long = "env",
        value_name = "NAME[=VALUE]",
        num_args = 0..=1,
        default_missing_value = "",
        requires = "interpolate"
    )]
    environment: Vec<EnvironmentInput>,
    /// Activate one Compose profile; repeat to activate more than one.
    #[arg(long = "profile", conflicts_with = "all_profiles")]
    profiles: Vec<String>,
    /// Activate every declared Compose profile.
    #[arg(long)]
    all_profiles: bool,
    /// Minimum reviewed Podman version, as major.minor or major.minor.patch.
    #[arg(long, default_value = "5.4")]
    podman_minimum_version: PodmanSelector,
    /// Maximum reviewed Podman version, as major.minor or major.minor.patch.
    #[arg(long, default_value = "6.0")]
    podman_maximum_version: PodmanSelector,
    /// Keep services separate or request one explicit pod.
    #[arg(long = "quadlet-grouping", value_enum, default_value_t = Grouping::Separate)]
    grouping: Grouping,
    /// Native name for the requested single Podman pod.
    #[arg(long)]
    pod_name: Option<String>,
    /// Native physical output layout.
    #[arg(long, value_enum, default_value_t = OutputLayout::Files)]
    output_layout: OutputLayout,
    /// Authorization for documented non-exact conversion outcomes.
    #[arg(long, value_enum, default_value_t = CliLossPolicy::Exact)]
    loss_policy: CliLossPolicy,
}

#[derive(Debug, Args)]
struct ConvertCommand {
    #[command(flatten)]
    conversion: GenericConversion,
    /// New absent directory that will receive generated Quadlet files.
    #[arg(long, required = true)]
    output_directory: PathBuf,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum InputType {
    Compose,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum OutputType {
    Quadlet,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum OutputLayout {
    Files,
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
    #[arg(long = "variable-from-environment", value_name = "NAME", requires = "interpolate", value_parser = parse_variable_name)]
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

impl From<Grouping> for QuadletGroupingPolicy {
    fn from(value: Grouping) -> Self {
        match value {
            Grouping::Separate => Self::SeparateContainers,
            Grouping::Pod => Self::SinglePod,
        }
    }
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

#[derive(Clone, Debug, Eq, PartialEq)]
enum EnvironmentInput {
    Literal(VariableAssignment),
    Process(String),
}

impl FromStr for EnvironmentInput {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.is_empty() {
            return Err("--env requires NAME or NAME=VALUE".to_owned());
        }
        if value.contains('=') {
            return VariableAssignment::from_str(value).map(Self::Literal);
        }
        parse_variable_name(value).map(Self::Process)
    }
}

#[derive(Clone, Debug)]
struct PodmanSelector {
    requested: String,
    major: u64,
    minor: u64,
    patch: Option<u64>,
}

impl FromStr for PodmanSelector {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let parts: Vec<_> = value.split('.').collect();
        if !(2..=3).contains(&parts.len()) {
            return Err("Podman version must use major.minor or major.minor.patch".to_owned());
        }
        let parse = |part: &str| {
            part.parse::<u64>()
                .map_err(|_| format!("invalid Podman version `{value}`"))
        };
        Ok(Self {
            requested: value.to_owned(),
            major: parse(parts[0])?,
            minor: parse(parts[1])?,
            patch: parts.get(2).map(|part| parse(part)).transpose()?,
        })
    }
}

#[derive(Default)]
struct ReportAliases {
    values: Vec<(String, String)>,
}

impl ReportAliases {
    fn for_invocation(arguments: &GenericConversion, output_directory: Option<&Path>) -> Self {
        let mut values = Vec::new();
        if let Some(project) = &arguments.project_directory {
            values.push((project.display().to_string(), "<project>".to_owned()));
        }
        for (index, path) in arguments.input_files.iter().enumerate() {
            if path != Path::new("-") {
                values.push((path.display().to_string(), format!("<input-{}>", index + 1)));
            }
        }
        for (index, path) in arguments.input_directories.iter().enumerate() {
            values.push((path.display().to_string(), format!("<input-directory-{}>", index + 1)));
        }
        for (index, path) in arguments.env_files.iter().enumerate() {
            values.push((path.display().to_string(), format!("<env-file-{}>", index + 1)));
        }
        if let Some(path) = output_directory {
            values.push((path.display().to_string(), "<output>".to_owned()));
        }
        values.sort_by_key(|value| std::cmp::Reverse(value.0.len()));
        Self { values }
    }

    fn add_resolved(&mut self, project_root: &Path, inputs: &[ResolvedInput]) {
        self.values
            .push((project_root.display().to_string(), "<project>".into()));
        for (index, input) in inputs.iter().enumerate() {
            if let Some(path) = input.path() {
                self.values
                    .push((path.display().to_string(), format!("<input-{}>", index + 1)));
            }
            for (ignored_index, ignored) in input.ignored().iter().enumerate() {
                self.values.push((
                    ignored.display().to_string(),
                    format!("<input-{}-ignored-{}>", index + 1, ignored_index + 1),
                ));
            }
        }
        self.values.sort_by_key(|value| std::cmp::Reverse(value.0.len()));
    }

    fn value(&self, value: &str) -> String {
        let mut value = value.to_owned();
        for (path, alias) in &self.values {
            if !path.is_empty() {
                value = value.replace(path, alias);
            }
        }
        value
            .split_whitespace()
            .map(|word| {
                if word.contains('/')
                    || word.contains('\\')
                    || (word.len() > 2 && word.as_bytes()[1] == b':' && matches!(word.as_bytes()[2], b'\\' | b'/'))
                {
                    "<path>".to_owned()
                } else {
                    word.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn new_report(arguments: &GenericConversion) -> ConversionReport {
    let mut report = ConversionReport::new(
        env!("CARGO_PKG_VERSION"),
        "compose",
        "quadlet",
        VersionBounds {
            minimum: arguments.podman_minimum_version.requested.clone(),
            maximum: arguments.podman_maximum_version.requested.clone(),
        },
    );
    report.choices = vec![
        ReportChoice {
            name: "loss_policy".into(),
            value: format!("{:?}", arguments.loss_policy).to_lowercase(),
        },
        ReportChoice {
            name: "grouping".into(),
            value: format!("{:?}", arguments.grouping).to_lowercase(),
        },
        ReportChoice {
            name: "profiles".into(),
            value: if arguments.all_profiles {
                "all".into()
            } else {
                arguments.profiles.join(",")
            },
        },
    ];
    report.host = HostMetadata {
        os_family: env::consts::FAMILY.into(),
        architecture: env::consts::ARCH.into(),
    };
    report
}

fn sanitized_invocation(matches: &clap::ArgMatches, command_kind: &str) -> SanitizedInvocation {
    let command_matches = matches.subcommand().map_or(matches, |(_, matches)| matches);
    let mut option_names: Vec<_> = [
        ("input_type", "--input-type"),
        ("output_type", "--output-type"),
        ("input_files", "--input-file"),
        ("input_directories", "--input-directory"),
        ("project_directory", "--project-directory"),
        ("project_name", "--project-name"),
        ("interpolate", "--interpolate"),
        ("env_files", "--env-file"),
        ("environment", "--env"),
        ("profiles", "--profile"),
        ("all_profiles", "--all-profiles"),
        ("podman_minimum_version", "--podman-minimum-version"),
        ("podman_maximum_version", "--podman-maximum-version"),
        ("grouping", "--quadlet-grouping"),
        ("pod_name", "--pod-name"),
        ("output_layout", "--output-layout"),
        ("loss_policy", "--loss-policy"),
        ("report_file", "--report-file"),
        ("generate_error_report", "--generate-error-report"),
        ("verbose", "--verbose"),
        ("quiet", "--quiet"),
        ("console_format", "--console-format"),
    ]
    .into_iter()
    .filter_map(|(id, name)| {
        (command_matches.value_source(id) == Some(ValueSource::CommandLine)).then_some(name.into())
    })
    .collect();
    if command_kind == "convert" && command_matches.value_source("output_directory") == Some(ValueSource::CommandLine) {
        option_names.push("--output-directory".into());
    }
    SanitizedInvocation {
        command_kind: command_kind.into(),
        provided_option_names: option_names,
    }
}

fn report_failure(
    arguments: &GenericConversion,
    summary: &str,
    stage: FailedStage,
    aliases: &ReportAliases,
) -> ConversionReport {
    let mut report = new_report(arguments);
    report.failed_stage = Some(stage);
    report
        .diagnostics
        .push(sanitized_diagnostic("BFC0019", "error", summary, &[], aliases));
    report
}

fn sanitized_diagnostic(
    code: &str,
    severity: &str,
    summary: &str,
    fields: &[(&str, &str, bool)],
    aliases: &ReportAliases,
) -> ReportDiagnostic {
    let (summary, _) = redact_text("summary", &aliases.value(summary), false);
    let mut safe_fields = Vec::new();
    for (name, value, sensitive) in fields {
        let (value, redacted) = redact_text(name, &aliases.value(value), *sensitive);
        safe_fields.push(ReportField {
            name: (*name).into(),
            value,
        });
        let _ = redacted;
    }
    ReportDiagnostic {
        code: code.into(),
        severity: severity.into(),
        summary,
        fields: safe_fields,
        spans: Vec::new(),
    }
}

fn main() -> ExitCode {
    let matches = Cli::command().get_matches();
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(error) => error.exit(),
    };
    match run(cli, &matches) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("boxferry: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli, matches: &clap::ArgMatches) -> Result<ExitCode, Box<dyn Error>> {
    match cli.command {
        Command::ComposeToQuadlet(arguments) => convert_compose_to_quadlet(&arguments),
        Command::Convert(arguments) => {
            run_generic(&arguments.conversion, matches, Some(&arguments.output_directory), false)
        }
        Command::Validate(arguments) => run_generic(&arguments, matches, None, true),
        Command::Capabilities(presentation) => print_capabilities(presentation),
        Command::Help { command } => print_help(command),
        Command::Version => {
            println!("boxferry {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn print_help(command: Option<String>) -> Result<ExitCode, Box<dyn Error>> {
    let mut root = Cli::command();
    if let Some(command) = command {
        let subcommand = root
            .find_subcommand_mut(&command)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("unknown command `{command}`")))?;
        subcommand.print_long_help()?;
    } else {
        root.print_long_help()?;
    }
    println!();
    Ok(ExitCode::SUCCESS)
}

fn print_capabilities(presentation: Presentation) -> Result<ExitCode, Box<dyn Error>> {
    let coverage = QuadletExporter::new()?.catalogue().coverage();
    let minimum = coverage.minimum().to_string();
    let maximum = coverage.maximum().to_string();
    if matches!(presentation.console_format, Some(ConsoleFormat::Json)) {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "schema_version": 1,
                "routes": [{
                    "input_type": "compose",
                    "output_type": "quadlet",
                    "podman_minimum": minimum,
                    "podman_maximum": maximum,
                    "fidelity_boundaries": {
                        "exact": "supported-compose-quadlet-intersection",
                        "approximate": ["pod-grouping"],
                        "policy_controlled": ["unsupported-fields"]
                    }
                }]
            }))?
        );
    } else if !presentation.quiet {
        println!("compose -> quadlet (Podman {minimum} through {maximum})");
        if presentation.verbose {
            println!("fidelity: exact for the supported Compose and Quadlet intersection");
            println!(
                "fidelity boundary: pod grouping is approximate; unsupported fields require the selected loss policy"
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn generic_input_order(matches: &clap::ArgMatches) -> io::Result<Vec<OrderedInput>> {
    let (_, subcommand) = matches
        .subcommand()
        .ok_or_else(|| io::Error::other("missing command"))?;
    let files = subcommand.get_many::<PathBuf>("input_files").into_iter().flatten();
    let file_indices = subcommand.indices_of("input_files").into_iter().flatten();
    let directories = subcommand
        .get_many::<PathBuf>("input_directories")
        .into_iter()
        .flatten();
    let directory_indices = subcommand.indices_of("input_directories").into_iter().flatten();
    let mut inputs: Vec<_> = file_indices
        .zip(files)
        .map(|(index, path)| (index, OrderedInput::File(path.clone())))
        .chain(
            directory_indices
                .zip(directories)
                .map(|(index, path)| (index, OrderedInput::Directory(path.clone()))),
        )
        .collect();
    inputs.sort_by_key(|(index, _)| *index);
    if inputs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "at least one --input-file or --input-directory is required",
        ));
    }
    Ok(inputs.into_iter().map(|(_, input)| input).collect())
}

enum OrderedInput {
    File(PathBuf),
    Directory(PathBuf),
}

#[derive(Debug)]
struct StructuredFailure {
    stage: FailedStage,
    diagnostics: Vec<ReportDiagnostic>,
}

impl std::fmt::Display for StructuredFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Compose preprocessing failed")
    }
}

impl Error for StructuredFailure {}

#[allow(clippy::too_many_lines)]
fn generic_convert(
    arguments: &GenericConversion,
    ordered: Vec<OrderedInput>,
    output_directory: Option<&Path>,
    validate_only: bool,
) -> Result<(ConversionReport, ExitCode, Vec<ResolvedInput>), Box<dyn Error>> {
    if !matches!(
        (arguments.input_type, arguments.output_type),
        (InputType::Compose, OutputType::Quadlet)
    ) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the requested generic route is not implemented",
        )
        .into());
    }
    if arguments.pod_name.is_some() && !matches!(arguments.grouping, Grouping::Pod) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--pod-name requires --quadlet-grouping pod",
        )
        .into());
    }
    let discovered = resolve_compose_inputs(ordered)?;
    let project_root = resolve_project_root(arguments.project_directory.as_deref(), &discovered)?;
    let fallback_name = arguments
        .project_name
        .as_deref()
        .map_or_else(|| derive_project_name(&project_root), str::to_owned);
    let (minimum, maximum) = resolve_versions(&arguments.podman_minimum_version, &arguments.podman_maximum_version)?;
    let interpolation = generic_interpolation_environment(arguments)?;
    let conversion = ComposeConversion {
        inputs: &discovered,
        project_root: &project_root,
        fallback_name: &fallback_name,
        profiles: &arguments.profiles,
        all_profiles: arguments.all_profiles,
        interpolation: interpolation.as_ref(),
        grouping: arguments.grouping,
        pod_name: arguments.pod_name.as_deref(),
        minimum,
        maximum: Some(maximum),
        policy: arguments.loss_policy.into(),
    };
    let mut aliases = ReportAliases::for_invocation(arguments, output_directory);
    aliases.add_resolved(&project_root, &discovered);
    let loaded_conversion = convert_loaded_compose(&conversion, &aliases)?;
    let result = loaded_conversion.result;
    let diagnostics = result.diagnostics();
    let output = result.output();
    let mut report = new_report(arguments);
    report.application = Some(loaded_conversion.application);
    report.status = if output.is_some() {
        ReportStatus::Success
    } else {
        ReportStatus::Blocked
    };
    report.exit_category = if output.is_some() {
        ExitCategory::Success
    } else {
        ExitCategory::PolicyBlocked
    };
    report.failed_stage = if output.is_some() {
        None
    } else {
        Some(FailedStage::Conversion)
    };
    report.resolved_versions = VersionBounds {
        minimum: minimum.to_string(),
        maximum: maximum.to_string(),
    };
    report.inputs = discovered
        .iter()
        .enumerate()
        .map(|(index, input)| ReportInput {
            alias: if matches!(input, ResolvedInput::Stdin) {
                "<stdin>".into()
            } else {
                format!("<input-{}>", index + 1)
            },
            kind: if matches!(input, ResolvedInput::Discovered { .. }) {
                "discovered".into()
            } else {
                "file".into()
            },
        })
        .collect();
    report.discovery = discovered
        .iter()
        .enumerate()
        .filter_map(|(index, input)| match input {
            ResolvedInput::Discovered { ignored, .. } => Some(DiscoveryDecision {
                selected: format!("<input-{}>", index + 1),
                ignored: ignored
                    .iter()
                    .enumerate()
                    .map(|(ignored_index, _)| format!("<input-{}-ignored-{}>", index + 1, ignored_index + 1))
                    .collect(),
            }),
            _ => None,
        })
        .collect();
    report.diagnostics = loaded_conversion.diagnostics;
    report.diagnostics.extend(
        diagnostics
            .iter()
            .map(|diagnostic| report_diagnostic(diagnostic, &aliases)),
    );
    report.fidelity = fidelity_counts(result.outcomes());
    deduplicate_report_diagnostics(&mut report.diagnostics);
    report.output_artifacts = output.zip(output_directory).map_or_else(Vec::new, |(value, _)| {
        value
            .files()
            .iter()
            .map(|file| OutputArtifact {
                name: file.name().as_str().to_owned(),
                size: u64::try_from(file.text().len()).unwrap_or(u64::MAX),
            })
            .collect()
    });
    let Some(output) = output else {
        return Ok((report, ExitCode::from(2), discovered));
    };
    if !validate_only {
        let output_directory = output_directory.ok_or_else(|| io::Error::other("missing convert output directory"))?;
        if let Err(error) = write_new_output_directory(output_directory, output.files()) {
            report.diagnostics.push(sanitized_diagnostic(
                "BFC0021",
                "error",
                &format!("output could not be written: {error}"),
                &[],
                &aliases,
            ));
            report.events.push("output-write-failed".into());
            report.status = ReportStatus::Failure;
            report.exit_category = ExitCategory::OutputWrite;
            report.failed_stage = Some(FailedStage::OutputWrite);
            return Ok((report, ExitCode::FAILURE, discovered));
        }
    }
    report.status = ReportStatus::Success;
    Ok((report, ExitCode::SUCCESS, discovered))
}

fn fidelity_counts(outcomes: &[boxferry::ConversionOutcome]) -> FidelityCounts {
    let mut counts = FidelityCounts::default();
    for outcome in outcomes {
        match outcome.kind() {
            ConversionKind::Exact => counts.exact += 1,
            ConversionKind::Approximate => counts.approximate += 1,
            ConversionKind::Unsupported => counts.unsupported += 1,
            ConversionKind::Invalid => counts.invalid += 1,
            _ => counts.other += 1,
        }
    }
    counts
}

fn run_generic(
    arguments: &GenericConversion,
    matches: &clap::ArgMatches,
    output_directory: Option<&Path>,
    validate_only: bool,
) -> Result<ExitCode, Box<dyn Error>> {
    let aliases = ReportAliases::for_invocation(arguments, output_directory);
    let result = generic_input_order(matches)
        .map_err(Box::<dyn Error>::from)
        .and_then(|ordered| generic_convert(arguments, ordered, output_directory, validate_only));
    let (mut report, primary_code, inputs) = match result {
        Ok(result) => result,
        Err(error) => {
            if let Some(structured) = error.downcast_ref::<StructuredFailure>() {
                let mut report = report_failure(arguments, &error.to_string(), structured.stage, &aliases);
                report.diagnostics.clone_from(&structured.diagnostics);
                if structured.stage == FailedStage::OutputWrite {
                    report.exit_category = ExitCategory::OutputWrite;
                }
                (report, ExitCode::FAILURE, Vec::new())
            } else {
                (
                    report_failure(
                        arguments,
                        &error.to_string(),
                        failure_stage(&error.to_string()),
                        &aliases,
                    ),
                    ExitCode::FAILURE,
                    Vec::new(),
                )
            }
        }
    };
    report.invocation = sanitized_invocation(matches, if validate_only { "validate" } else { "convert" });
    let mut final_code = primary_code;
    if let Some(path) = &arguments.report_file {
        let encoded = serialize_report(&mut report)?;
        if let Err(error) = write_report_file(path, &encoded) {
            report.diagnostics.push(sanitized_diagnostic(
                "BFC0020",
                "error",
                &format!("report file could not be written: {error}"),
                &[],
                &aliases,
            ));
            report.events.push("report-file-write-failed".into());
            if primary_code == ExitCode::SUCCESS {
                report.failed_stage = Some(FailedStage::ReportWrite);
                report.status = ReportStatus::Failure;
                report.exit_category = ExitCategory::ReportWrite;
                final_code = ExitCode::FAILURE;
            }
        }
    }
    if let Some(path) = &arguments.generate_error_report {
        let encoded = serialize_report(&mut report)?;
        if let Err(error) = write_support_bundle(path, &encoded) {
            report.diagnostics.push(sanitized_diagnostic(
                "BFC0022",
                "error",
                &format!("diagnostic support bundle could not be written: {error}"),
                &[],
                &aliases,
            ));
            report.events.push("support-bundle-write-failed".into());
            if final_code == ExitCode::SUCCESS {
                report.failed_stage = Some(FailedStage::ReportWrite);
                report.status = ReportStatus::Failure;
                report.exit_category = ExitCategory::ReportWrite;
                final_code = ExitCode::FAILURE;
            }
        }
    }
    present(arguments, &mut report, &inputs, validate_only)?;
    Ok(final_code)
}

fn failure_stage(message: &str) -> FailedStage {
    let message = message.to_ascii_lowercase();
    if message.contains("env") || message.contains("interpolation") {
        FailedStage::Interpolation
    } else if message.contains("read") {
        FailedStage::InputRead
    } else if message.contains("compose") {
        FailedStage::ComposeLoad
    } else {
        FailedStage::InputDiscovery
    }
}

fn present(
    arguments: &GenericConversion,
    report: &mut ConversionReport,
    inputs: &[ResolvedInput],
    validate_only: bool,
) -> Result<(), Box<dyn Error>> {
    let presentation = arguments.presentation;
    if matches!(presentation.console_format, Some(ConsoleFormat::Json)) {
        println!("{}", serialize_report(report)?);
        return Ok(());
    }
    if !presentation.quiet {
        println!("route: {} -> {}", report.source_type, report.target_type);
        for input in inputs {
            println!("input: {}", input.label());
        }
        if presentation.verbose {
            println!(
                "Podman requested: {} through {}",
                report.requested_versions.minimum, report.requested_versions.maximum
            );
            println!(
                "Podman resolved: {} through {}",
                report.resolved_versions.minimum, report.resolved_versions.maximum
            );
            if arguments.all_profiles {
                println!("profiles: all");
            } else {
                for profile in &arguments.profiles {
                    println!("profile: {profile}");
                }
            }
            for path in &arguments.env_files {
                println!("interpolation source: env-file {}", path.display());
            }
            for value in &arguments.environment {
                match value {
                    EnvironmentInput::Literal(value) => println!("interpolation source: explicit {}", value.name),
                    EnvironmentInput::Process(name) => println!("interpolation source: process {name}"),
                }
            }
            for input in inputs {
                for ignored in input.ignored() {
                    println!("ignored candidate: {}", ignored.display());
                }
            }
        }
        if let Some(application) = &report.application {
            println!("application: {application}");
        }
        match report.status {
            ReportStatus::Success => println!("stage: conversion complete"),
            ReportStatus::Blocked => println!("stage: conversion blocked"),
            ReportStatus::Failure => println!(
                "stage: {} failed",
                report.failed_stage.map_or("unknown", failed_stage_name)
            ),
        }
        if report.status == ReportStatus::Success {
            if validate_only {
                println!("result: validation succeeded");
            } else {
                println!(
                    "result: wrote {} file(s) to output directory",
                    report.output_artifacts.len()
                );
            }
            if presentation.verbose {
                for artifact in &report.output_artifacts {
                    println!("wrote: {}", artifact.name);
                }
            }
        }
    }
    print_report_diagnostics(&report.diagnostics);
    if report.exit_category == ExitCategory::PolicyBlocked {
        eprintln!("boxferry: output blocked by the selected loss policy");
    }
    Ok(())
}

const fn failed_stage_name(stage: FailedStage) -> &'static str {
    match stage {
        FailedStage::InputDiscovery => "input discovery",
        FailedStage::InputRead => "input read",
        FailedStage::Interpolation => "interpolation",
        FailedStage::ComposeLoad => "Compose load",
        FailedStage::ComposeMerge => "Compose merge",
        FailedStage::ProfileSelection => "profile selection",
        FailedStage::Conversion => "conversion",
        FailedStage::OutputWrite => "output write",
        FailedStage::ReportWrite => "report write",
    }
}

struct ComposeConversion<'a> {
    inputs: &'a [ResolvedInput],
    project_root: &'a Path,
    fallback_name: &'a str,
    profiles: &'a [String],
    all_profiles: bool,
    interpolation: Option<&'a MapEnvironment>,
    grouping: Grouping,
    pod_name: Option<&'a str>,
    minimum: PlatformVersion,
    maximum: Option<PlatformVersion>,
    policy: LossPolicy,
}

struct LoadedComposeConversion {
    result: boxferry::ConversionResult<boxferry::QuadletOutput>,
    diagnostics: Vec<ReportDiagnostic>,
    application: String,
}

fn convert_loaded_compose(
    conversion: &ComposeConversion<'_>,
    aliases: &ReportAliases,
) -> Result<LoadedComposeConversion, Box<dyn Error>> {
    let mut identities = Vec::with_capacity(conversion.inputs.len());
    let mut documents = Vec::with_capacity(conversion.inputs.len());
    for (index, input) in conversion.inputs.iter().enumerate() {
        let id = ComposeSourceId::new(
            u32::try_from(index + 1).map_err(|_| io::Error::other("too many Compose input files"))?,
        );
        let (label, directory, text) = input.read(conversion.project_root)?;
        documents.push(DocumentInput::new(
            id,
            DocumentOrigin::new(label.clone(), directory),
            text,
        ));
        identities.push((id, SourceId::new(label)?));
    }
    let loaded = LoadedProject::load(documents)?;
    let interpolated = conversion
        .interpolation
        .map(|environment| loaded.interpolate(environment));
    let merged = merge_project(&loaded, interpolated.as_ref());
    if !merged.is_valid() {
        let mut diagnostics = merged
            .diagnostics()
            .iter()
            .map(|diagnostic| compose_diagnostic(diagnostic, aliases))
            .collect();
        deduplicate_report_diagnostics(&mut diagnostics);
        return Err(Box::new(StructuredFailure {
            stage: FailedStage::ComposeMerge,
            diagnostics,
        }));
    }
    let project = merged
        .project()
        .ok_or_else(|| io::Error::other("Compose merge produced no project"))?
        .clone();
    let request = if conversion.all_profiles {
        ProfileRequest::all()
    } else {
        conversion
            .profiles
            .iter()
            .fold(ProfileRequest::new(), ProfileRequest::with_profile)
    };
    let selection = select_profiles(&project, &request);
    if !selection.is_valid() {
        let mut diagnostics = merged
            .diagnostics()
            .iter()
            .map(|diagnostic| compose_diagnostic(diagnostic, aliases))
            .collect::<Vec<_>>();
        diagnostics.extend(
            selection
                .diagnostics()
                .iter()
                .map(|diagnostic| compose_diagnostic(diagnostic, aliases)),
        );
        deduplicate_report_diagnostics(&mut diagnostics);
        return Err(Box::new(StructuredFailure {
            stage: FailedStage::ProfileSelection,
            diagnostics,
        }));
    }
    let mut preprocessing_diagnostics = merged
        .diagnostics()
        .iter()
        .map(|diagnostic| compose_diagnostic(diagnostic, aliases))
        .collect::<Vec<_>>();
    preprocessing_diagnostics.extend(
        selection
            .diagnostics()
            .iter()
            .map(|diagnostic| compose_diagnostic(diagnostic, aliases)),
    );
    deduplicate_report_diagnostics(&mut preprocessing_diagnostics);
    let application = boxferry::compose::compose_lens::project::build_project_view(&project, Some(&selection))
        .view()
        .and_then(|view| view.name())
        .and_then(|name| Identifier::new(name.value().clone()).ok())
        .map_or_else(|| conversion.fallback_name.to_owned(), |name| name.as_str().to_owned());
    let mut source = ComposeSource::new(project, Identifier::new(conversion.fallback_name)?)?;
    for (compose, neutral) in identities {
        source = source.with_source_id(compose, neutral);
    }
    source = source.with_profile_selection(selection);
    let mut exporter = QuadletExporter::new()?
        .with_relative_host_path_root(conversion.project_root.to_string_lossy().into_owned())?
        .with_grouping_policy(conversion.grouping.into());
    if let Some(pod_name) = conversion.pod_name {
        exporter = exporter.with_pod_name(pod_name);
    }
    let target = TargetProfile::new("podman", conversion.minimum, conversion.maximum)?;
    Ok(LoadedComposeConversion {
        result: convert(&ComposeImporter::new()?, &source, &exporter, &target, conversion.policy)?,
        diagnostics: preprocessing_diagnostics,
        application,
    })
}

fn resolve_compose_inputs(ordered: Vec<OrderedInput>) -> io::Result<Vec<ResolvedInput>> {
    let mut resolved = Vec::new();
    let mut seen = BTreeSet::new();
    let mut stdin_count = 0_u8;
    for item in ordered {
        let input = match item {
            OrderedInput::File(path) if path == Path::new("-") => {
                stdin_count = stdin_count.saturating_add(1);
                if stdin_count > 1 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "stdin may be supplied only once",
                    ));
                }
                ResolvedInput::Stdin
            }
            OrderedInput::File(path) => ResolvedInput::File(resolve_regular_file(&path)?),
            OrderedInput::Directory(path) => discover_compose_directory(&path)?,
        };
        if let Some(path) = input.path() {
            let canonical = fs::canonicalize(path)?;
            if !seen.insert(canonical) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("duplicate resolved input: {}", path.display()),
                ));
            }
        }
        resolved.push(input);
    }
    Ok(resolved)
}

enum ResolvedInput {
    File(PathBuf),
    Discovered { selected: PathBuf, ignored: Vec<PathBuf> },
    Stdin,
}

impl ResolvedInput {
    fn path(&self) -> Option<&Path> {
        match self {
            Self::File(path) => Some(path),
            Self::Discovered { selected, .. } => Some(selected),
            Self::Stdin => None,
        }
    }
    fn label(&self) -> String {
        self.path()
            .map_or_else(|| "<stdin>".to_owned(), |path| path.display().to_string())
    }
    fn ignored(&self) -> &[PathBuf] {
        match self {
            Self::Discovered { ignored, .. } => ignored,
            _ => &[],
        }
    }
    fn read(&self, project_root: &Path) -> io::Result<(String, PathBuf, String)> {
        match self {
            Self::Stdin => {
                let mut text = String::new();
                io::stdin().read_to_string(&mut text)?;
                Ok(("<stdin>".to_owned(), project_root.to_path_buf(), text))
            }
            Self::File(path) | Self::Discovered { selected: path, .. } => Ok((
                path.display().to_string(),
                absolute_parent(path)?,
                fs::read_to_string(path)?,
            )),
        }
    }
}

fn resolve_regular_file(path: &Path) -> io::Result<PathBuf> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("input must be a regular non-symlink file: {}", path.display()),
        ));
    }
    Ok(path.to_path_buf())
}

fn discover_compose_directory(directory: &Path) -> io::Result<ResolvedInput> {
    let metadata = fs::symlink_metadata(directory)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "input directory must be a non-symlink directory: {}",
                directory.display()
            ),
        ));
    }
    let mut candidates = Vec::new();
    let mut ignored = Vec::new();
    for name in CONVENTIONAL_COMPOSE_FILES {
        let candidate = directory.join(name);
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.is_file() => candidates.push(candidate),
            Ok(_) => ignored.push(candidate),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    let Some(selected) = candidates.first().cloned() else {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no conventional Compose file in {}", directory.display()),
        ));
    };
    Ok(ResolvedInput::Discovered {
        selected,
        ignored: ignored.into_iter().chain(candidates.into_iter().skip(1)).collect(),
    })
}

fn resolve_project_root(explicit: Option<&Path>, inputs: &[ResolvedInput]) -> io::Result<PathBuf> {
    if explicit.is_none() && inputs.iter().any(|input| matches!(input, ResolvedInput::Stdin)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--project-directory is required when using stdin",
        ));
    }
    match explicit {
        Some(path) => {
            let metadata = fs::symlink_metadata(path)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "--project-directory must be a non-symlink directory",
                ));
            }
            fs::canonicalize(path)
        }
        None => inputs
            .first()
            .and_then(ResolvedInput::path)
            .map(absolute_parent)
            .transpose()?
            .ok_or_else(|| io::Error::other("no resolved input directory")),
    }
}

fn derive_project_name(project_root: &Path) -> String {
    project_root
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("compose")
        .to_owned()
}

fn generic_interpolation_environment(arguments: &GenericConversion) -> Result<Option<MapEnvironment>, Box<dyn Error>> {
    if !arguments.interpolate {
        return Ok(None);
    }
    let mut environment = MapEnvironment::new();
    for path in &arguments.env_files {
        for (name, value) in parse_environment_file(path)? {
            let _ = environment.insert_sensitive(name, value);
        }
    }
    for input in &arguments.environment {
        match input {
            EnvironmentInput::Literal(assignment) => {
                let _ = environment.insert_sensitive(assignment.name.clone(), assignment.value.clone());
            }
            EnvironmentInput::Process(name) => {
                let value = env::var(name).map_err(|error| authorized_environment_error(name, &error))?;
                let _ = environment.insert_sensitive(name.clone(), value);
            }
        }
    }
    Ok(Some(environment))
}

fn parse_environment_file(path: &Path) -> io::Result<Vec<(String, String)>> {
    let text = fs::read_to_string(path)?;
    let mut assignments = Vec::new();
    for (line_number, line) in text.lines().enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let (name, value) = line.split_once('=').ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}:{} must use NAME=VALUE", path.display(), line_number + 1),
            )
        })?;
        let name = parse_variable_name(name).map_err(|message| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("{}:{} {message}", path.display(), line_number + 1),
            )
        })?;
        assignments.push((name, value.to_owned()));
    }
    Ok(assignments)
}

fn resolve_versions(
    minimum: &PodmanSelector,
    maximum: &PodmanSelector,
) -> io::Result<(PlatformVersion, PlatformVersion)> {
    let exporter = QuadletExporter::new().map_err(io::Error::other)?;
    let coverage = exporter.catalogue().coverage();
    let coverage_minimum = PlatformVersion::new(
        coverage.minimum().major(),
        coverage.minimum().minor(),
        coverage.minimum().patch(),
    );
    let coverage_maximum = PlatformVersion::new(
        coverage.maximum().major(),
        coverage.maximum().minor(),
        coverage.maximum().patch(),
    );
    let evidence_versions: Vec<_> = exporter
        .catalogue()
        .evidence()
        .iter()
        .flat_map(|record| [record.versions().minimum(), record.versions().maximum()])
        .collect();
    let resolve = |selector: &PodmanSelector, lower: bool| -> io::Result<PlatformVersion> {
        let resolved = if let Some(patch) = selector.patch {
            PlatformVersion::new(selector.major, selector.minor, patch)
        } else {
            let mut patches: Vec<_> = evidence_versions
                .iter()
                .filter(|version| version.major() == selector.major && version.minor() == selector.minor)
                .map(|version| version.patch())
                .collect();
            if selector.major == coverage_minimum.major() && selector.minor == coverage_minimum.minor() {
                patches.push(coverage_minimum.patch());
            }
            if selector.major == coverage_maximum.major() && selector.minor == coverage_maximum.minor() {
                patches.push(coverage_maximum.patch());
            }
            let patch = if lower {
                patches.into_iter().min()
            } else {
                patches.into_iter().max()
            }
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "Podman selector `{}` has no reviewed patch in the Quadlet catalogue",
                        selector.requested
                    ),
                )
            })?;
            PlatformVersion::new(selector.major, selector.minor, patch)
        };
        if resolved < coverage_minimum || resolved > coverage_maximum {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Podman selector `{}` is outside the reviewed Quadlet catalogue",
                    selector.requested
                ),
            ));
        }
        Ok(resolved)
    };
    let minimum = resolve(minimum, true)?;
    let maximum = resolve(maximum, false)?;
    if maximum < minimum {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "resolved Podman maximum version is before minimum version",
        ));
    }
    Ok((minimum, maximum))
}

fn report_diagnostic(diagnostic: &Diagnostic, aliases: &ReportAliases) -> ReportDiagnostic {
    let fields: Vec<_> = diagnostic
        .fields()
        .iter()
        .map(|field| (field.name(), field.value().expose(), field.value().is_sensitive()))
        .collect();
    sanitized_diagnostic(
        diagnostic.code().as_str(),
        &format!("{:?}", diagnostic.severity()).to_lowercase(),
        diagnostic.summary(),
        &fields,
        aliases,
    )
}

fn compose_diagnostic(
    diagnostic: &boxferry::compose::compose_lens::diagnostic::Diagnostic,
    aliases: &ReportAliases,
) -> ReportDiagnostic {
    let (summary, _) = redact_text("summary", &aliases.value(diagnostic.message()), false);
    let spans = diagnostic
        .labels()
        .iter()
        .map(|label| boxferry::report::ReportSpan {
            source: format!("<input-{}>", label.span().source_id().get()),
            start: label.span().start(),
            end: label.span().end(),
        })
        .collect();
    ReportDiagnostic {
        code: diagnostic.code().as_str().into(),
        severity: format!("{:?}", diagnostic.severity()).to_lowercase(),
        summary,
        fields: Vec::new(),
        spans,
    }
}

fn deduplicate_report_diagnostics(diagnostics: &mut Vec<ReportDiagnostic>) {
    let mut seen = BTreeSet::new();
    diagnostics.retain(|diagnostic| seen.insert(format!("{diagnostic:?}")));
}

fn print_report_diagnostics(diagnostics: &[ReportDiagnostic]) {
    for diagnostic in diagnostics {
        eprintln!("{}: {}", diagnostic.code, diagnostic.summary);
        for field in &diagnostic.fields {
            eprintln!("  {}: {}", field.name, field.value);
        }
    }
}

fn serialize_report(report: &mut ConversionReport) -> Result<String, Box<dyn Error>> {
    report.enforce_v1_limits();
    let redacted_summaries = report
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.summary == boxferry::report::REDACTED)
        .count();
    let redacted_fields = report
        .diagnostics
        .iter()
        .flat_map(|diagnostic| diagnostic.fields.iter())
        .filter(|field| field.value == boxferry::report::REDACTED)
        .count();
    report.redaction.count = redacted_summaries + redacted_fields;
    report.redaction.classes = match (redacted_summaries > 0, redacted_fields > 0) {
        (true, true) => vec!["plain-text-pattern".into(), "protected-value".into()],
        (true, false) => vec!["plain-text-pattern".into()],
        (false, true) => vec!["protected-value".into()],
        (false, false) => Vec::new(),
    };
    let mut encoded = serde_json::to_string(report)?;
    while encoded.len() > boxferry::report::MAX_JSON_BYTES && report.reduce_for_json() {
        encoded = serde_json::to_string(report)?;
    }
    if encoded.len() > boxferry::report::MAX_JSON_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "report exceeds the v1 JSON size cap").into());
    }
    Ok(encoded)
}

fn write_report_file(path: &Path, report: &str) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let metadata = fs::symlink_metadata(parent)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "report-file parent must be an existing non-symlink directory",
        ));
    }
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(report.as_bytes())?;
    file.write_all(b"\n")
}

fn write_support_bundle(path: &Path, report: &str) -> io::Result<()> {
    validate_support_bundle_entries(SUPPORT_BUNDLE_README.as_bytes(), report.as_bytes())?;
    let archive = build_support_bundle(SUPPORT_BUNDLE_README.as_bytes(), report.as_bytes())?;
    publish_new_file(path, &archive)
}

fn validate_support_bundle_entries(readme: &[u8], report: &[u8]) -> io::Result<()> {
    if report.len() > boxferry::report::MAX_JSON_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "report.json exceeds the v1 size cap",
        ));
    }
    if readme.len() > MAX_README_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "README.md exceeds the v1 size cap",
        ));
    }
    Ok(())
}

fn build_support_bundle(readme: &[u8], report: &[u8]) -> io::Result<Vec<u8>> {
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file("README.md", options)
        .map_err(|error| zip_error(&error))?;
    writer.write_all(readme)?;
    writer
        .start_file("report.json", options)
        .map_err(|error| zip_error(&error))?;
    writer.write_all(report)?;
    let archive = writer.finish().map_err(|error| zip_error(&error))?.into_inner();
    if archive.len() > MAX_ARCHIVE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "stored support bundle exceeds the v1 size cap",
        ));
    }
    Ok(archive)
}

fn zip_error(error: &zip::result::ZipError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("ZIP construction failed: {error}"))
}

fn publish_new_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let metadata = fs::symlink_metadata(parent)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "support-bundle parent must be an existing non-symlink directory",
        ));
    }
    let mut temporary = SupportBundleTemporary::create(parent, path)?;
    {
        let file = temporary
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("temporary bundle file missing"))?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    temporary.file.take();
    fs::hard_link(temporary.path(), path)?;
    temporary.remove()
}

struct SupportBundleTemporary {
    path: PathBuf,
    file: Option<fs::File>,
}

impl SupportBundleTemporary {
    fn create(parent: &Path, destination: &Path) -> io::Result<Self> {
        let name = destination
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "support-bundle path must name a file"))?;
        for _ in 0..128 {
            let counter = SUPPORT_BUNDLE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let temporary_name = format!(
                ".{}.boxferry-{}-{counter}.tmp",
                name.to_string_lossy(),
                std::process::id()
            );
            let path = parent.join(temporary_name);
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => return Ok(Self { path, file: Some(file) }),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not reserve a unique support-bundle temporary path",
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn remove(mut self) -> io::Result<()> {
        self.file.take();
        fs::remove_file(&self.path)?;
        self.path.clear();
        Ok(())
    }
}

impl Drop for SupportBundleTemporary {
    fn drop(&mut self) {
        self.file.take();
        if !self.path.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn convert_compose_to_quadlet(arguments: &ComposeToQuadlet) -> Result<ExitCode, Box<dyn Error>> {
    let legacy_diagnostic = Diagnostic::new(
        boxferry::DiagnosticCode::new("BFC0018")?,
        boxferry::Severity::Warning,
        "compose-to-quadlet is deprecated; use the generic convert command",
    )
    .with_field(boxferry::DiagnosticField::new(
        "migration",
        boxferry::DiagnosticValue::plain(
            "boxferry convert --input-type compose --output-type quadlet --input-file <FILE> --output-directory <DIR>",
        ),
    ));
    eprintln!("{legacy_diagnostic}");
    let project_root = absolute_parent(&arguments.files[0])?;
    let inputs = arguments
        .files
        .iter()
        .cloned()
        .map(ResolvedInput::File)
        .collect::<Vec<_>>();
    let interpolation = legacy_interpolation_environment(arguments)?;
    let conversion = ComposeConversion {
        inputs: &inputs,
        project_root: &project_root,
        fallback_name: &arguments.project_name,
        profiles: &arguments.profiles,
        all_profiles: arguments.all_profiles,
        interpolation: interpolation.as_ref(),
        grouping: arguments.grouping,
        pod_name: None,
        minimum: arguments.podman_minimum_version,
        maximum: arguments.podman_maximum_version,
        policy: arguments.loss_policy.into(),
    };
    let loaded = match convert_loaded_compose(&conversion, &ReportAliases::default()) {
        Ok(loaded) => loaded,
        Err(error) => {
            if let Some(structured) = error.downcast_ref::<StructuredFailure>() {
                print_report_diagnostics(&structured.diagnostics);
                return Ok(ExitCode::from(2));
            }
            return Err(error);
        }
    };
    let result = loaded.result;
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

fn legacy_interpolation_environment(arguments: &ComposeToQuadlet) -> Result<Option<MapEnvironment>, Box<dyn Error>> {
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
        let value = env::var(name).map_err(|error| authorized_environment_error(name, &error))?;
        let _ = environment.insert_sensitive(name.clone(), value);
    }
    Ok(Some(environment))
}

fn authorized_environment_error(name: &str, error: &env::VarError) -> io::Error {
    match error {
        env::VarError::NotPresent => io::Error::new(
            io::ErrorKind::NotFound,
            format!("authorized interpolation variable `{name}` is not present in the process environment"),
        ),
        env::VarError::NotUnicode(_) => io::Error::new(
            io::ErrorKind::InvalidData,
            format!("authorized interpolation variable `{name}` is not valid Unicode"),
        ),
    }
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

fn print_diagnostics(diagnostics: &[Diagnostic]) {
    for diagnostic in diagnostics {
        eprintln!("{diagnostic}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use boxferry::compose::compose_lens::{
        interpolation::EnvironmentProvider,
        project::{ProjectValue, build_project_view},
    };

    #[test]
    fn generic_interpolation_inputs_propagate_sensitivity_to_compose_values() -> Result<(), Box<dyn Error>> {
        let environment_file = env::temp_dir().join(format!(
            "boxferry-generic-interpolation-sensitivity-{}-{}.env",
            std::process::id(),
            SUPPORT_BUNDLE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::write(&environment_file, "FILE_VALUE=file-value-canary\n")?;
        let arguments = GenericConversion {
            presentation: Presentation {
                verbose: false,
                quiet: false,
                console_format: None,
            },
            report_file: None,
            generate_error_report: None,
            input_type: InputType::Compose,
            output_type: OutputType::Quadlet,
            input_files: Vec::new(),
            input_directories: Vec::new(),
            project_directory: None,
            project_name: None,
            interpolate: true,
            env_files: vec![environment_file.clone()],
            environment: vec![
                EnvironmentInput::Literal(VariableAssignment {
                    name: "LITERAL_VALUE".into(),
                    value: "literal-value-canary".into(),
                }),
                EnvironmentInput::Process("PATH".into()),
            ],
            profiles: Vec::new(),
            all_profiles: false,
            podman_minimum_version: "5.4".parse()?,
            podman_maximum_version: "6.0".parse()?,
            grouping: Grouping::Separate,
            pod_name: None,
            output_layout: OutputLayout::Files,
            loss_policy: CliLossPolicy::Exact,
        };
        let environment = generic_interpolation_environment(&arguments)?.ok_or("interpolation environment missing")?;
        for name in ["FILE_VALUE", "LITERAL_VALUE", "PATH"] {
            assert!(
                environment.get(name).is_some_and(|value| value.is_sensitive()),
                "{name} was not protected"
            );
        }

        let document = DocumentInput::new(
            ComposeSourceId::new(1),
            DocumentOrigin::new("memory", env::temp_dir()),
            concat!(
                "services:\n",
                "  from-file:\n    image: example.invalid/app:1\n    command: \"${FILE_VALUE}\"\n",
                "  literal:\n    image: example.invalid/app:1\n    command: \"${LITERAL_VALUE}\"\n",
                "  process:\n    image: example.invalid/app:1\n    command: \"${PATH}\"\n",
            ),
        );
        let loaded = LoadedProject::load(vec![document])?;
        let interpolated = loaded.interpolate(&environment);
        let merged = merge_project(&loaded, Some(&interpolated));
        let project = merged.project().ok_or("merged project missing")?;
        let selection = select_profiles(project, &ProfileRequest::new());
        let project_view = build_project_view(project, Some(&selection));
        let view = project_view.view().ok_or("project view missing")?;
        for name in ["from-file", "literal", "process"] {
            assert!(
                view.service(name)
                    .and_then(|service| service.command())
                    .is_some_and(ProjectValue::is_sensitive),
                "interpolated command for {name} was not protected"
            );
        }
        fs::remove_file(environment_file)?;
        Ok(())
    }

    #[test]
    fn support_bundle_entry_and_archive_limits_are_enforced() -> io::Result<()> {
        let maximum_readme = vec![b'r'; MAX_README_BYTES];
        let maximum_report = vec![b'j'; boxferry::report::MAX_JSON_BYTES];
        assert!(validate_support_bundle_entries(&maximum_readme, &maximum_report).is_ok());
        assert!(validate_support_bundle_entries(&vec![b'r'; MAX_README_BYTES + 1], b"{}").is_err());
        assert!(validate_support_bundle_entries(b"readme", &vec![b'j'; boxferry::report::MAX_JSON_BYTES + 1]).is_err());

        let overhead = build_support_bundle(b"", b"")?.len();
        let archive_at_limit = vec![b'a'; MAX_ARCHIVE_BYTES - overhead];
        let archive = build_support_bundle(&archive_at_limit, b"")?;
        assert_eq!(archive.len(), MAX_ARCHIVE_BYTES);
        assert!(build_support_bundle(&vec![b'a'; MAX_ARCHIVE_BYTES - overhead + 1], b"").is_err());
        Ok(())
    }

    #[test]
    fn failed_temporary_removal_retains_the_path_for_drop_retry() -> io::Result<()> {
        let path = env::temp_dir().join(format!(
            "boxferry-support-bundle-remove-test-{}-{}",
            std::process::id(),
            SUPPORT_BUNDLE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        let temporary = SupportBundleTemporary {
            path: path.clone(),
            file: None,
        };
        assert!(temporary.remove().is_err());
        assert!(path.exists());
        fs::remove_dir(path)
    }
}
