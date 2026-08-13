//! Command-line interface for `BoxFerry`.

#[cfg(target_os = "windows")]
compile_error!("the native Windows BoxFerry CLI is unsupported; install and run BoxFerry inside WSL2");

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
    diagnostic::Diagnostic as ComposeDiagnostic,
    interpolation::MapEnvironment,
    loader::{DocumentInput, DocumentOrigin, LoadedProject, ProjectInterpolation},
    merge::{MergeResult, MergedProject, merge_project},
    profiles::{ProfileRequest, ProfileSelection, select_profiles},
    source::SourceId as ComposeSourceId,
};
use boxferry::report::{
    ConversionReport, DiscoveryDecision, ExitCategory, FailedStage, FidelityCounts, FixFirst, HostMetadata,
    OutputArtifact, ReportChoice, ReportDiagnostic, ReportField, ReportInput, ReportNativeFinding, ReportNativeLabel,
    ReportStatus, SanitizedInvocation, VersionBounds, redact_text,
};
use boxferry::{
    COMPOSE_SPECIFICATION_PROFILE_REVISION, COMPOSE_SPECIFICATION_TARGET, ComposeExporter, ComposeFindingStage,
    ComposeImporter, ComposeSource, ConversionError, ConversionKind, Diagnostic, Identifier, LossPolicy, NativeFinding,
    NativeFindingLabelKind, PlatformVersion, QuadletDocumentInput, QuadletExporter, QuadletGroupingPolicy,
    QuadletImporter, QuadletParseDiagnostic, QuadletParseDiagnosticOrigin, QuadletParseError, QuadletSource, RULES,
    RuleId, SourceId, TargetProfile, convert, find_rule,
};
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum, parser::ValueSource};
use jiff::{Timestamp, Zoned, tz::TimeZone};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

mod route;
use route::{InputOptions, InputType, OutputType, RouteExecutor, RouteSpec, TargetSelector};

const CONVENTIONAL_COMPOSE_FILES: [&str; 6] = [
    "compose.yaml",
    "compose.yml",
    "podman-compose.yaml",
    "podman-compose.yml",
    "docker-compose.yaml",
    "docker-compose.yml",
];
const QUADLET_EXTENSIONS: [&str; 6] = ["container", "pod", "network", "volume", "build", "image"];
const MAX_README_BYTES: usize = 128 * 1024;
const MAX_ARCHIVE_BYTES: usize = 5 * 1024 * 1024;
const MAX_ERROR_REPORT_COLLISIONS: u8 = 99;
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
    /// Convert one explicit input type into one explicit output type.
    Convert(ConvertCommand),
    /// Parse and plan one explicit route without writing output.
    Validate(ConversionCommand),
    /// List document conversion routes implemented by this build.
    Capabilities(Presentation),
    /// List every `BoxFerry` diagnostic rule in stable code order.
    Rules(CataloguePresentation),
    /// Explain one diagnostic rule by code or human-readable name.
    Explain(ExplainCommand),
    /// Show contextual command help.
    Help {
        /// Command path whose contextual help should be shown.
        #[arg(value_name = "COMMAND", num_args = 0..)]
        command: Vec<String>,
    },
    /// Print `BoxFerry` version information.
    Version,
}

#[derive(Clone, Copy, Debug, Args)]
struct Presentation {
    /// Include discovery and output-version resolution detail in human output.
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

#[derive(Clone, Copy, Debug, Args)]
struct CataloguePresentation {
    /// Select machine-readable console output.
    #[arg(long, value_enum)]
    console_format: Option<ConsoleFormat>,
}

#[derive(Debug, Args)]
struct ExplainCommand {
    /// Exact rule code or human-readable rule name.
    rule: String,
    #[command(flatten)]
    presentation: CataloguePresentation,
}

#[derive(Debug, Args)]
struct InputDocuments {
    /// Explicit input document in input order; repeat as needed.
    #[arg(long = "input-file", value_name = "PATH")]
    input_files: Vec<PathBuf>,
    /// Route-specific input directory expanded at this position.
    #[arg(long = "input-directory", value_name = "PATH")]
    input_directories: Vec<PathBuf>,
}

#[derive(Debug, Args)]
struct ComposeInputOptions {
    /// Compose project root for resolving paths referenced by input documents.
    #[arg(long)]
    project_directory: Option<PathBuf>,
    /// Fallback Compose project name when the input does not declare one.
    #[arg(long)]
    project_name: Option<String>,
    /// Interpolate Compose input using explicitly supplied variables.
    #[arg(long)]
    interpolate: bool,
    /// Compose interpolation assignments; later files override earlier files.
    #[arg(long = "env-file", value_name = "PATH", requires = "interpolate")]
    env_files: Vec<PathBuf>,
    /// Compose interpolation NAME=VALUE or authorized process NAME.
    #[arg(
        long = "env",
        value_name = "NAME[=VALUE]",
        num_args = 0..=1,
        default_missing_value = "",
        requires = "interpolate"
    )]
    environment: Vec<EnvironmentInput>,
    /// Active Compose profile; repeat to activate more than one.
    #[arg(long = "profile", conflicts_with = "all_profiles")]
    profiles: Vec<String>,
    /// Activate every declared Compose profile.
    #[arg(long)]
    all_profiles: bool,
}

#[derive(Debug, Args)]
struct QuadletInputOptions {
    /// Neutral application name assigned to the Quadlet input document set.
    #[arg(long)]
    application_name: String,
}

#[derive(Debug, Args)]
struct QuadletOutputOptions {
    /// Minimum Podman version, as major.minor or major.minor.patch.
    #[arg(long, default_value = "5.4")]
    podman_minimum_version: PodmanSelector,
    /// Maximum Podman version, as major.minor or major.minor.patch.
    #[arg(long, default_value = "6.0")]
    podman_maximum_version: PodmanSelector,
    /// Quadlet service grouping request.
    #[arg(long = "quadlet-grouping", value_enum, default_value_t = Grouping::Separate)]
    grouping: Grouping,
    /// Native name for the requested single Podman pod.
    #[arg(long)]
    pod_name: Option<String>,
    /// Quadlet physical output layout.
    #[arg(long, value_enum, default_value_t = OutputLayout::Files)]
    output_layout: OutputLayout,
}

#[derive(Debug, Args)]
struct ComposeOutputOptions {
    /// Compose physical output layout.
    #[arg(long, value_enum, default_value_t = OutputLayout::Files)]
    output_layout: OutputLayout,
}

#[derive(Clone, Copy, Debug, Args)]
struct ConversionPolicyOptions {
    /// Authorization for documented non-exact conversion outcomes.
    #[arg(long, value_enum, default_value_t = CliLossPolicy::Exact)]
    loss_policy: CliLossPolicy,
}

#[derive(Debug, Args)]
struct DiagnosticOptions {
    #[command(flatten)]
    presentation: Presentation,
    /// Write the complete privacy-safe structured result to a new file.
    #[arg(long, value_name = "PATH")]
    report_file: Option<PathBuf>,
    /// Write a locally reviewable ZIP with fixed README.md and report.json entries.
    #[arg(long)]
    generate_error_report: bool,
    /// Existing non-symlink directory for the generated error report.
    #[arg(long, value_name = "DIR", requires = "generate_error_report")]
    error_report_directory: Option<PathBuf>,
}

#[derive(Debug, Args)]
#[command(subcommand_help_heading = "Input types", subcommand_value_name = "INPUT_TYPE")]
struct ConversionCommand {
    #[command(subcommand)]
    input: ConversionInput,
}

#[derive(Debug, Subcommand)]
enum ConversionInput {
    /// Use Compose input documents.
    Compose(ComposeInputCommand),
    /// Use a Quadlet input document set.
    Quadlet(QuadletInputCommand),
}

#[derive(Debug, Args)]
#[command(subcommand_help_heading = "Output types", subcommand_value_name = "OUTPUT_TYPE")]
struct ComposeInputCommand {
    #[command(subcommand)]
    output: ComposeOutput,
}

#[derive(Debug, Subcommand)]
enum ComposeOutput {
    /// Convert Compose input to canonical Compose output.
    Compose(ComposeToCompose),
    /// Convert Compose input to canonical Quadlet output.
    Quadlet(ComposeToQuadlet),
}

#[derive(Debug, Args)]
#[command(subcommand_help_heading = "Output types", subcommand_value_name = "OUTPUT_TYPE")]
struct QuadletInputCommand {
    #[command(subcommand)]
    output: QuadletOutput,
}

#[derive(Debug, Subcommand)]
enum QuadletOutput {
    /// Convert Quadlet input to canonical Compose output.
    Compose(QuadletToCompose),
    /// Convert Quadlet input to canonical Quadlet output.
    Quadlet(QuadletToQuadlet),
}

#[derive(Debug, Args)]
#[command(about = "Read Compose input and write canonical BoxFerry Compose YAML")]
struct ComposeToCompose {
    #[command(flatten, next_help_heading = "Input documents")]
    documents: InputDocuments,
    #[command(flatten, next_help_heading = "Compose input")]
    input: ComposeInputOptions,
    #[command(flatten, next_help_heading = "Compose output")]
    output: ComposeOutputOptions,
    #[command(flatten, next_help_heading = "Conversion policy")]
    policy: ConversionPolicyOptions,
    #[command(flatten, next_help_heading = "Diagnostics and reports")]
    diagnostics: DiagnosticOptions,
}

#[derive(Debug, Args)]
#[command(about = "Read Compose input and write canonical BoxFerry Quadlet files")]
struct ComposeToQuadlet {
    #[command(flatten, next_help_heading = "Input documents")]
    documents: InputDocuments,
    #[command(flatten, next_help_heading = "Compose input")]
    input: ComposeInputOptions,
    #[command(flatten, next_help_heading = "Quadlet output")]
    output: QuadletOutputOptions,
    #[command(flatten, next_help_heading = "Conversion policy")]
    policy: ConversionPolicyOptions,
    #[command(flatten, next_help_heading = "Diagnostics and reports")]
    diagnostics: DiagnosticOptions,
}

#[derive(Debug, Args)]
#[command(about = "Read Quadlet input and write canonical BoxFerry Compose YAML")]
struct QuadletToCompose {
    #[command(flatten, next_help_heading = "Input documents")]
    documents: InputDocuments,
    #[command(flatten, next_help_heading = "Quadlet input")]
    input: QuadletInputOptions,
    #[command(flatten, next_help_heading = "Compose output")]
    output: ComposeOutputOptions,
    #[command(flatten, next_help_heading = "Conversion policy")]
    policy: ConversionPolicyOptions,
    #[command(flatten, next_help_heading = "Diagnostics and reports")]
    diagnostics: DiagnosticOptions,
}

#[derive(Debug, Args)]
#[command(about = "Read Quadlet input and write canonical BoxFerry Quadlet files")]
struct QuadletToQuadlet {
    #[command(flatten, next_help_heading = "Input documents")]
    documents: InputDocuments,
    #[command(flatten, next_help_heading = "Quadlet input")]
    input: QuadletInputOptions,
    #[command(flatten, next_help_heading = "Quadlet output")]
    output: QuadletOutputOptions,
    #[command(flatten, next_help_heading = "Conversion policy")]
    policy: ConversionPolicyOptions,
    #[command(flatten, next_help_heading = "Diagnostics and reports")]
    diagnostics: DiagnosticOptions,
}

#[derive(Debug, Args)]
#[command(subcommand_help_heading = "Input types", subcommand_value_name = "INPUT_TYPE")]
struct ConvertCommand {
    #[command(subcommand)]
    input: ConvertInput,
}

#[derive(Debug, Subcommand)]
enum ConvertInput {
    /// Use Compose input documents.
    Compose(ConvertComposeInputCommand),
    /// Use a Quadlet input document set.
    Quadlet(ConvertQuadletInputCommand),
}

#[derive(Debug, Args)]
#[command(subcommand_help_heading = "Output types", subcommand_value_name = "OUTPUT_TYPE")]
struct ConvertComposeInputCommand {
    #[command(subcommand)]
    output: ConvertComposeOutput,
}

#[derive(Debug, Subcommand)]
enum ConvertComposeOutput {
    /// Convert Compose input to canonical Compose output.
    Compose(ConvertComposeToCompose),
    /// Convert Compose input to canonical Quadlet output.
    Quadlet(ConvertComposeToQuadlet),
}

#[derive(Debug, Args)]
#[command(subcommand_help_heading = "Output types", subcommand_value_name = "OUTPUT_TYPE")]
struct ConvertQuadletInputCommand {
    #[command(subcommand)]
    output: ConvertQuadletOutput,
}

#[derive(Debug, Subcommand)]
enum ConvertQuadletOutput {
    /// Convert Quadlet input to canonical Compose output.
    Compose(ConvertQuadletToCompose),
    /// Convert Quadlet input to canonical Quadlet output.
    Quadlet(ConvertQuadletToQuadlet),
}

#[derive(Debug, Args)]
struct OutputDestination {
    /// Absent or existing empty directory that will receive generated route artifacts.
    #[arg(long, required = true, value_name = "DIR")]
    output_directory: PathBuf,
}

#[derive(Debug, Args)]
#[command(about = "Read Compose input and write canonical BoxFerry Compose YAML")]
struct ConvertComposeToCompose {
    #[command(flatten, next_help_heading = "Input documents")]
    documents: InputDocuments,
    #[command(flatten, next_help_heading = "Compose input")]
    input: ComposeInputOptions,
    #[command(flatten, next_help_heading = "Compose output")]
    output: ComposeOutputOptions,
    #[command(flatten, next_help_heading = "Conversion policy")]
    policy: ConversionPolicyOptions,
    #[command(flatten, next_help_heading = "Compose output")]
    destination: OutputDestination,
    #[command(flatten, next_help_heading = "Diagnostics and reports")]
    diagnostics: DiagnosticOptions,
}

#[derive(Debug, Args)]
#[command(about = "Read Compose input and write canonical BoxFerry Quadlet files")]
struct ConvertComposeToQuadlet {
    #[command(flatten, next_help_heading = "Input documents")]
    documents: InputDocuments,
    #[command(flatten, next_help_heading = "Compose input")]
    input: ComposeInputOptions,
    #[command(flatten, next_help_heading = "Quadlet output")]
    output: QuadletOutputOptions,
    #[command(flatten, next_help_heading = "Conversion policy")]
    policy: ConversionPolicyOptions,
    #[command(flatten, next_help_heading = "Quadlet output")]
    destination: OutputDestination,
    #[command(flatten, next_help_heading = "Diagnostics and reports")]
    diagnostics: DiagnosticOptions,
}

#[derive(Debug, Args)]
#[command(about = "Read Quadlet input and write canonical BoxFerry Compose YAML")]
struct ConvertQuadletToCompose {
    #[command(flatten, next_help_heading = "Input documents")]
    documents: InputDocuments,
    #[command(flatten, next_help_heading = "Quadlet input")]
    input: QuadletInputOptions,
    #[command(flatten, next_help_heading = "Compose output")]
    output: ComposeOutputOptions,
    #[command(flatten, next_help_heading = "Conversion policy")]
    policy: ConversionPolicyOptions,
    #[command(flatten, next_help_heading = "Compose output")]
    destination: OutputDestination,
    #[command(flatten, next_help_heading = "Diagnostics and reports")]
    diagnostics: DiagnosticOptions,
}

#[derive(Debug, Args)]
#[command(about = "Read Quadlet input and write canonical BoxFerry Quadlet files")]
struct ConvertQuadletToQuadlet {
    #[command(flatten, next_help_heading = "Input documents")]
    documents: InputDocuments,
    #[command(flatten, next_help_heading = "Quadlet input")]
    input: QuadletInputOptions,
    #[command(flatten, next_help_heading = "Quadlet output")]
    output: QuadletOutputOptions,
    #[command(flatten, next_help_heading = "Conversion policy")]
    policy: ConversionPolicyOptions,
    #[command(flatten, next_help_heading = "Quadlet output")]
    destination: OutputDestination,
    #[command(flatten, next_help_heading = "Diagnostics and reports")]
    diagnostics: DiagnosticOptions,
}

struct GenericConversion {
    presentation: Presentation,
    report_file: Option<PathBuf>,
    generate_error_report: bool,
    error_report_directory: Option<PathBuf>,
    input_type: InputType,
    output_type: OutputType,
    input_files: Vec<PathBuf>,
    input_directories: Vec<PathBuf>,
    project_directory: Option<PathBuf>,
    application_name: Option<String>,
    interpolate: bool,
    env_files: Vec<PathBuf>,
    environment: Vec<EnvironmentInput>,
    profiles: Vec<String>,
    all_profiles: bool,
    podman_minimum_version: PodmanSelector,
    podman_maximum_version: PodmanSelector,
    grouping: Grouping,
    pod_name: Option<String>,
    output_layout: OutputLayout,
    loss_policy: CliLossPolicy,
}

impl ConversionInput {
    fn into_generic(self) -> GenericConversion {
        match self {
            Self::Compose(command) => match command.output {
                ComposeOutput::Compose(arguments) => GenericConversion::from_compose_to_compose(arguments),
                ComposeOutput::Quadlet(arguments) => GenericConversion::from_compose_to_quadlet(arguments),
            },
            Self::Quadlet(command) => match command.output {
                QuadletOutput::Compose(arguments) => GenericConversion::from_quadlet_to_compose(arguments),
                QuadletOutput::Quadlet(arguments) => GenericConversion::from_quadlet_to_quadlet(arguments),
            },
        }
    }
}

impl ConvertInput {
    fn into_invocation(self) -> (GenericConversion, PathBuf) {
        match self {
            Self::Compose(command) => match command.output {
                ConvertComposeOutput::Compose(arguments) => {
                    let destination = arguments.destination.output_directory;
                    let conversion = GenericConversion::from_route(
                        InputType::Compose,
                        OutputType::Compose,
                        arguments.documents,
                        InputRouteOptions::Compose(arguments.input),
                        OutputRouteOptions::Compose(arguments.output),
                        arguments.policy,
                        arguments.diagnostics,
                    );
                    (conversion, destination)
                }
                ConvertComposeOutput::Quadlet(arguments) => {
                    let destination = arguments.destination.output_directory;
                    let conversion = GenericConversion::from_route(
                        InputType::Compose,
                        OutputType::Quadlet,
                        arguments.documents,
                        InputRouteOptions::Compose(arguments.input),
                        OutputRouteOptions::Quadlet(arguments.output),
                        arguments.policy,
                        arguments.diagnostics,
                    );
                    (conversion, destination)
                }
            },
            Self::Quadlet(command) => match command.output {
                ConvertQuadletOutput::Compose(arguments) => {
                    let destination = arguments.destination.output_directory;
                    let conversion = GenericConversion::from_route(
                        InputType::Quadlet,
                        OutputType::Compose,
                        arguments.documents,
                        InputRouteOptions::Quadlet(arguments.input),
                        OutputRouteOptions::Compose(arguments.output),
                        arguments.policy,
                        arguments.diagnostics,
                    );
                    (conversion, destination)
                }
                ConvertQuadletOutput::Quadlet(arguments) => {
                    let destination = arguments.destination.output_directory;
                    let conversion = GenericConversion::from_route(
                        InputType::Quadlet,
                        OutputType::Quadlet,
                        arguments.documents,
                        InputRouteOptions::Quadlet(arguments.input),
                        OutputRouteOptions::Quadlet(arguments.output),
                        arguments.policy,
                        arguments.diagnostics,
                    );
                    (conversion, destination)
                }
            },
        }
    }
}

impl ConversionCommand {
    fn into_generic(self) -> GenericConversion {
        self.input.into_generic()
    }
}

impl GenericConversion {
    fn from_compose_to_compose(arguments: ComposeToCompose) -> Self {
        Self::from_route(
            InputType::Compose,
            OutputType::Compose,
            arguments.documents,
            InputRouteOptions::Compose(arguments.input),
            OutputRouteOptions::Compose(arguments.output),
            arguments.policy,
            arguments.diagnostics,
        )
    }

    fn from_compose_to_quadlet(arguments: ComposeToQuadlet) -> Self {
        Self::from_route(
            InputType::Compose,
            OutputType::Quadlet,
            arguments.documents,
            InputRouteOptions::Compose(arguments.input),
            OutputRouteOptions::Quadlet(arguments.output),
            arguments.policy,
            arguments.diagnostics,
        )
    }

    fn from_quadlet_to_compose(arguments: QuadletToCompose) -> Self {
        Self::from_route(
            InputType::Quadlet,
            OutputType::Compose,
            arguments.documents,
            InputRouteOptions::Quadlet(arguments.input),
            OutputRouteOptions::Compose(arguments.output),
            arguments.policy,
            arguments.diagnostics,
        )
    }

    fn from_quadlet_to_quadlet(arguments: QuadletToQuadlet) -> Self {
        Self::from_route(
            InputType::Quadlet,
            OutputType::Quadlet,
            arguments.documents,
            InputRouteOptions::Quadlet(arguments.input),
            OutputRouteOptions::Quadlet(arguments.output),
            arguments.policy,
            arguments.diagnostics,
        )
    }

    fn from_route(
        input_type: InputType,
        output_type: OutputType,
        documents: InputDocuments,
        input: InputRouteOptions,
        output: OutputRouteOptions,
        policy: ConversionPolicyOptions,
        diagnostics: DiagnosticOptions,
    ) -> Self {
        let (project_directory, application_name, interpolate, env_files, environment, profiles, all_profiles) =
            match input {
                InputRouteOptions::Compose(input) => (
                    input.project_directory,
                    input.project_name,
                    input.interpolate,
                    input.env_files,
                    input.environment,
                    input.profiles,
                    input.all_profiles,
                ),
                InputRouteOptions::Quadlet(input) => (
                    None,
                    Some(input.application_name),
                    false,
                    Vec::new(),
                    Vec::new(),
                    Vec::new(),
                    false,
                ),
            };
        let (podman_minimum_version, podman_maximum_version, grouping, pod_name, output_layout) = match output {
            OutputRouteOptions::Compose(output) => (
                PodmanSelector::minimum_default(),
                PodmanSelector::maximum_default(),
                Grouping::Separate,
                None,
                output.output_layout,
            ),
            OutputRouteOptions::Quadlet(output) => (
                output.podman_minimum_version,
                output.podman_maximum_version,
                output.grouping,
                output.pod_name,
                output.output_layout,
            ),
        };
        Self {
            presentation: diagnostics.presentation,
            report_file: diagnostics.report_file,
            generate_error_report: diagnostics.generate_error_report,
            error_report_directory: diagnostics.error_report_directory,
            input_type,
            output_type,
            input_files: documents.input_files,
            input_directories: documents.input_directories,
            project_directory,
            application_name,
            interpolate,
            env_files,
            environment,
            profiles,
            all_profiles,
            podman_minimum_version,
            podman_maximum_version,
            grouping,
            pod_name,
            output_layout,
            loss_policy: policy.loss_policy,
        }
    }
}

enum InputRouteOptions {
    Compose(ComposeInputOptions),
    Quadlet(QuadletInputOptions),
}

enum OutputRouteOptions {
    Compose(ComposeOutputOptions),
    Quadlet(QuadletOutputOptions),
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum OutputLayout {
    Files,
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

impl PodmanSelector {
    fn minimum_default() -> Self {
        Self {
            requested: "5.4".into(),
            major: 5,
            minor: 4,
            patch: None,
        }
    }

    fn maximum_default() -> Self {
        Self {
            requested: "6.0".into(),
            major: 6,
            minor: 0,
            patch: None,
        }
    }
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
        if let Some(path) = &arguments.report_file {
            values.push((path.display().to_string(), "<report-file>".to_owned()));
        }
        if let Some(path) = &arguments.error_report_directory {
            values.push((path.display().to_string(), "<error-report-directory>".to_owned()));
        }
        for (index, path) in arguments.input_files.iter().enumerate() {
            if path != Path::new("-") {
                values.push((path.display().to_string(), format!("<input-{}>", index + 1)));
                if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                    values.push((name.to_owned(), format!("<input-{}>", index + 1)));
                }
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
        self.add_inputs(inputs);
    }

    fn add_inputs(&mut self, inputs: &[ResolvedInput]) {
        for (index, input) in inputs.iter().enumerate() {
            if let Some(path) = input.path() {
                self.values
                    .push((path.display().to_string(), format!("<input-{}>", index + 1)));
                if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                    self.values.push((name.to_owned(), format!("<input-{}>", index + 1)));
                }
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

fn new_report(arguments: &GenericConversion, route: RouteSpec) -> ConversionReport {
    let requested_versions = match route.target_selector {
        TargetSelector::PodmanRange => VersionBounds {
            minimum: arguments.podman_minimum_version.requested.clone(),
            maximum: arguments.podman_maximum_version.requested.clone(),
        },
        TargetSelector::ComposeSpecification => VersionBounds {
            minimum: "rolling".into(),
            maximum: "rolling".into(),
        },
    };
    let mut report = ConversionReport::new(
        env!("CARGO_PKG_VERSION"),
        route.source_name(),
        route.target_name(),
        requested_versions,
    );
    report.choices.push(ReportChoice {
        name: "loss_policy".into(),
        value: format!("{:?}", arguments.loss_policy).to_lowercase(),
    });
    report.choices.push(ReportChoice {
        name: "output_layout".into(),
        value: format!("{:?}", arguments.output_layout).to_lowercase(),
    });
    match route.input {
        InputType::Compose => report.choices.push(ReportChoice {
            name: "profiles".into(),
            value: if arguments.all_profiles {
                "all".into()
            } else {
                arguments.profiles.join(",")
            },
        }),
        InputType::Quadlet => {}
    }
    match route.target_selector {
        TargetSelector::PodmanRange => report.choices.push(ReportChoice {
            name: "grouping".into(),
            value: format!("{:?}", arguments.grouping).to_lowercase(),
        }),
        TargetSelector::ComposeSpecification => {}
    }
    report.host = HostMetadata {
        os_family: env::consts::FAMILY.into(),
        architecture: env::consts::ARCH.into(),
    };
    report
}

fn sanitized_invocation(matches: &clap::ArgMatches, command_kind: &str) -> SanitizedInvocation {
    let mut option_names: Vec<_> = [
        ("input_files", "--input-file"),
        ("input_directories", "--input-directory"),
        ("project_directory", "--project-directory"),
        ("project_name", "--project-name"),
        ("application_name", "--application-name"),
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
        ("error_report_directory", "--error-report-directory"),
        ("verbose", "--verbose"),
        ("quiet", "--quiet"),
        ("console_format", "--console-format"),
    ]
    .into_iter()
    .filter_map(|(id, name)| explicitly_supplied(matches, id).then_some(name.into()))
    .collect();
    if command_kind == "convert" && explicitly_supplied(matches, "output_directory") {
        option_names.push("--output-directory".into());
    }
    SanitizedInvocation {
        command_kind: command_kind.into(),
        provided_option_names: option_names,
    }
}

fn report_failure(
    arguments: &GenericConversion,
    route: Option<RouteSpec>,
    summary: &str,
    stage: FailedStage,
    aliases: &ReportAliases,
) -> ConversionReport {
    let mut report = route.map_or_else(
        || new_unavailable_report(arguments),
        |route| new_report(arguments, route),
    );
    report.failed_stage = Some(stage);
    report.diagnostics.push(sanitized_diagnostic(
        RuleId::OrchestrationFailed,
        "error",
        summary,
        &[],
        aliases,
    ));
    report
}

fn new_unavailable_report(arguments: &GenericConversion) -> ConversionReport {
    let mut report = ConversionReport::new(
        env!("CARGO_PKG_VERSION"),
        arguments.input_type.name(),
        arguments.output_type.name(),
        VersionBounds {
            minimum: "unavailable".into(),
            maximum: "unavailable".into(),
        },
    );
    report.host = HostMetadata {
        os_family: env::consts::FAMILY.into(),
        architecture: env::consts::ARCH.into(),
    };
    report
}

fn sanitized_diagnostic(
    rule: RuleId,
    severity: &str,
    summary: &str,
    fields: &[(&str, &str, bool)],
    aliases: &ReportAliases,
) -> ReportDiagnostic {
    let definition = rule.definition();
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
        code: definition.code().into(),
        name: definition.name().into(),
        source_code: None,
        severity: severity.into(),
        summary,
        help: definition.help().into(),
        fields: safe_fields,
        spans: Vec::new(),
        native_finding: None,
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
        Command::Convert(arguments) => {
            let (conversion, output_directory) = arguments.input.into_invocation();
            run_generic(&conversion, matches, Some(&output_directory), false)
        }
        Command::Validate(arguments) => run_generic(&arguments.into_generic(), matches, None, true),
        Command::Capabilities(presentation) => print_capabilities(presentation),
        Command::Rules(presentation) => print_rules(presentation),
        Command::Explain(arguments) => print_rule_explanation(&arguments),
        Command::Help { command } => print_help(&command),
        Command::Version => {
            println!("boxferry {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn sorted_rules() -> Vec<&'static boxferry::DiagnosticRule> {
    let mut rules = RULES.iter().collect::<Vec<_>>();
    rules.sort_by_key(|rule| rule.code());
    rules
}

fn rule_json(rule: &boxferry::DiagnosticRule) -> serde_json::Value {
    serde_json::json!({
        "code": rule.code(),
        "name": rule.name(),
        "owner": rule.owner(),
        "default_severity": severity_name(rule.default_severity()),
        "description": rule.description(),
        "help": rule.help(),
    })
}

fn print_rules(presentation: CataloguePresentation) -> Result<ExitCode, Box<dyn Error>> {
    let rules = sorted_rules();
    if matches!(presentation.console_format, Some(ConsoleFormat::Json)) {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "schema_version": 1,
                "rules": rules.into_iter().map(rule_json).collect::<Vec<_>>(),
            }))?
        );
    } else {
        for rule in rules {
            println!(
                "{} {} [{}]",
                rule.code(),
                rule.name(),
                severity_name(rule.default_severity())
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn print_rule_explanation(arguments: &ExplainCommand) -> Result<ExitCode, Box<dyn Error>> {
    let rule = find_rule(&arguments.rule).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "unknown diagnostic rule `{}`; run `boxferry rules` to list valid rules",
                arguments.rule
            ),
        )
    })?;
    if matches!(arguments.presentation.console_format, Some(ConsoleFormat::Json)) {
        println!("{}", serde_json::to_string(&rule_json(rule))?);
    } else {
        println!("{} {}", rule.code(), rule.name());
        println!("owner: {}", rule.owner());
        println!("severity: {}", severity_name(rule.default_severity()));
        println!();
        println!("{}", rule.description());
        println!();
        println!("help: {}", rule.help());
    }
    Ok(ExitCode::SUCCESS)
}

fn print_help(command: &[String]) -> Result<ExitCode, Box<dyn Error>> {
    let mut root = Cli::command();
    let mut selected = &mut root;
    for component in command {
        selected = selected.find_subcommand_mut(component).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("unknown command path component `{component}`"),
            )
        })?;
    }
    let display_name = std::iter::once("boxferry")
        .chain(command.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    let mut help = selected
        .clone()
        .bin_name(display_name)
        .version(env!("CARGO_PKG_VERSION"));
    help.print_long_help()?;
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
                "routes": route::routes().iter().map(|route| capability_json(*route, &minimum, &maximum)).collect::<Vec<_>>()
            }))?
        );
    } else if !presentation.quiet {
        for route in route::routes() {
            match route.target_selector {
                TargetSelector::PodmanRange => println!(
                    "{} -> {} (Podman {minimum} through {maximum})",
                    route.source_name(),
                    route.target_name()
                ),
                TargetSelector::ComposeSpecification => println!(
                    "{} -> {} (rolling Compose Specification)",
                    route.source_name(),
                    route.target_name()
                ),
            }
            if presentation.verbose {
                println!("fidelity: exact for {}", route.exact_boundary);
                println!(
                    "fidelity boundary: approximate {:?}; policy controlled {:?}",
                    route.approximate_boundaries, route.policy_controlled_boundaries
                );
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn capability_json(route: RouteSpec, minimum: &str, maximum: &str) -> serde_json::Value {
    let fidelity = serde_json::json!({
        "exact": route.exact_boundary,
        "approximate": route.approximate_boundaries,
        "policy_controlled": route.policy_controlled_boundaries,
    });
    match route.target_selector {
        TargetSelector::PodmanRange => serde_json::json!({
            "input_type": route.source_name(), "output_type": route.target_name(),
            "target_selector": "podman-range", "podman_minimum": minimum,
            "podman_maximum": maximum, "fidelity_boundaries": fidelity,
        }),
        TargetSelector::ComposeSpecification => serde_json::json!({
            "input_type": route.source_name(), "output_type": route.target_name(),
            "target_selector": "compose-specification-rolling", "requested_version": "rolling",
            "resolved_version": "rolling", "fidelity_boundaries": fidelity,
        }),
    }
}

fn generic_input_order(matches: &clap::ArgMatches) -> io::Result<Vec<OrderedInput>> {
    let subcommand = deepest_command_matches(matches);
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
    inputs: Vec<ReportInput>,
    discovery: Vec<DiscoveryDecision>,
    presentation_inputs: Vec<ResolvedInput>,
}

struct RenderedFile {
    name: String,
    text: String,
}

struct RenderedConversion {
    output: Option<Vec<RenderedFile>>,
    outcomes: Vec<boxferry::ConversionOutcome>,
    diagnostics: Vec<Diagnostic>,
}

impl RenderedConversion {
    fn from_result<T>(result: boxferry::ConversionResult<T>, render: impl FnOnce(T) -> Vec<RenderedFile>) -> Self {
        let (output, outcomes, diagnostics) = result.into_parts();
        Self {
            output: output.map(render),
            outcomes,
            diagnostics,
        }
    }
}

struct DocumentExecution {
    discovered: Vec<ResolvedInput>,
    aliases: ReportAliases,
    application_name: String,
    resolved_versions: VersionBounds,
    conversion: RenderedConversion,
}

impl StructuredFailure {
    fn with_context(stage: FailedStage, diagnostics: Vec<ReportDiagnostic>, inputs: &[ResolvedInput]) -> Self {
        let presentation_inputs = inputs.to_vec();
        let (inputs, discovery) = resolved_report_context(inputs);
        Self {
            stage,
            diagnostics,
            inputs,
            discovery,
            presentation_inputs,
        }
    }
}

impl std::fmt::Display for StructuredFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("input preprocessing failed")
    }
}

impl Error for StructuredFailure {}

fn resolved_report_context(resolved: &[ResolvedInput]) -> (Vec<ReportInput>, Vec<DiscoveryDecision>) {
    let inputs = resolved
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
    let discovery = resolved
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
    (inputs, discovery)
}

fn post_discovery_failure(
    stage: FailedStage,
    rule: RuleId,
    summary: &str,
    error: &dyn Error,
    aliases: &ReportAliases,
    inputs: &[ResolvedInput],
) -> Box<dyn Error> {
    Box::new(StructuredFailure::with_context(
        stage,
        vec![sanitized_diagnostic(
            rule,
            "error",
            summary,
            &[("reason", &error.to_string(), false)],
            aliases,
        )],
        inputs,
    ))
}

#[allow(clippy::too_many_lines)]
fn generic_convert(
    arguments: &GenericConversion,
    ordered: Vec<OrderedInput>,
    output_directory: Option<&Path>,
    validate_only: bool,
) -> Result<(ConversionReport, ExitCode, Vec<ResolvedInput>), Box<dyn Error>> {
    let route = validate_route(arguments, &ordered)?;
    if matches!(
        route.executor,
        RouteExecutor::QuadletToCompose | RouteExecutor::QuadletToQuadlet
    ) {
        return generic_quadlet_convert(arguments, ordered, route, output_directory, validate_only);
    }
    if arguments.pod_name.is_some() && !matches!(arguments.grouping, Grouping::Pod) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "--pod-name requires --quadlet-grouping pod",
        )
        .into());
    }
    let discovered = resolve_compose_inputs(ordered)?;
    let mut aliases = ReportAliases::for_invocation(arguments, output_directory);
    aliases.add_inputs(&discovered);
    let project_root = resolve_project_root(arguments.project_directory.as_deref(), &discovered).map_err(|error| {
        post_discovery_failure(
            FailedStage::InputDiscovery,
            RuleId::ComposeProjectRootInvalid,
            "Compose project root could not be resolved",
            &error,
            &aliases,
            &discovered,
        )
    })?;
    aliases.add_resolved(&project_root, &discovered);
    let fallback_name = arguments
        .application_name
        .as_deref()
        .map_or_else(|| derive_project_name(&project_root), str::to_owned);
    let podman_versions = if route.target_selector == TargetSelector::PodmanRange {
        Some(
            resolve_versions(&arguments.podman_minimum_version, &arguments.podman_maximum_version).map_err(
                |error| {
                    post_discovery_failure(
                        FailedStage::Conversion,
                        RuleId::PodmanTargetSelectionInvalid,
                        "Podman output version selection failed",
                        &error,
                        &aliases,
                        &discovered,
                    )
                },
            )?,
        )
    } else {
        None
    };
    let interpolation = generic_interpolation_environment(arguments).map_err(|error| {
        post_discovery_failure(
            FailedStage::Interpolation,
            RuleId::InterpolationInputInvalid,
            "Compose interpolation inputs could not be resolved",
            error.as_ref(),
            &aliases,
            &discovered,
        )
    })?;
    let conversion = ComposeConversion {
        inputs: &discovered,
        project_root: &project_root,
        fallback_name: &fallback_name,
        profiles: &arguments.profiles,
        all_profiles: arguments.all_profiles,
        interpolation: interpolation.as_ref(),
    };
    let loaded = load_compose_source(&conversion, &aliases)?;
    let (conversion, resolved_versions) = match route.executor {
        RouteExecutor::ComposeToQuadlet => {
            let (minimum, maximum) = podman_versions.ok_or_else(|| io::Error::other("missing Podman output range"))?;
            let mut exporter = QuadletExporter::new()?
                .with_relative_host_path_root(project_root.to_string_lossy().into_owned())?
                .with_grouping_policy(arguments.grouping.into());
            if let Some(pod_name) = arguments.pod_name.as_deref() {
                exporter = exporter.with_pod_name(pod_name);
            }
            let target = TargetProfile::new("podman", minimum, Some(maximum))?;
            let result = convert(
                &ComposeImporter::new()?,
                &loaded.source,
                &exporter,
                &target,
                arguments.loss_policy.into(),
            )
            .map_err(|error| {
                compose_conversion_failure(&error, &loaded.preprocessing_diagnostics, &aliases, &discovered)
            })?;
            (
                RenderedConversion::from_result(result, |output| {
                    output
                        .files()
                        .iter()
                        .map(|file| RenderedFile {
                            name: file.name().as_str().to_owned(),
                            text: file.text().to_owned(),
                        })
                        .collect()
                }),
                VersionBounds {
                    minimum: minimum.to_string(),
                    maximum: maximum.to_string(),
                },
            )
        }
        RouteExecutor::ComposeToCompose => {
            let (document, diagnostics) = loaded.source.canonicalize()?.into_parts();
            (
                RenderedConversion {
                    output: document.map(|document| {
                        vec![RenderedFile {
                            name: "compose.yaml".into(),
                            text: document.text().to_owned(),
                        }]
                    }),
                    outcomes: Vec::new(),
                    diagnostics,
                },
                VersionBounds {
                    minimum: "rolling".into(),
                    maximum: "rolling".into(),
                },
            )
        }
        _ => return Err(io::Error::other("Compose input selected a non-Compose route executor").into()),
    };
    finish_document_conversion(
        arguments,
        route,
        output_directory,
        validate_only,
        DocumentExecution {
            discovered,
            aliases,
            application_name: loaded.application,
            resolved_versions,
            conversion,
        },
    )
}

fn deepest_command_matches(matches: &clap::ArgMatches) -> &clap::ArgMatches {
    matches
        .subcommand()
        .map_or(matches, |(_, command)| deepest_command_matches(command))
}

fn explicitly_supplied(matches: &clap::ArgMatches, id: &str) -> bool {
    let present_here = matches.ids().any(|candidate| candidate.as_str() == id)
        && matches.value_source(id) == Some(ValueSource::CommandLine);
    present_here
        || matches
            .subcommand()
            .is_some_and(|(_, command)| explicitly_supplied(command, id))
}

fn validate_route(arguments: &GenericConversion, ordered: &[OrderedInput]) -> io::Result<RouteSpec> {
    let route = route::find(arguments.input_type, arguments.output_type).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "the requested input/output route is not implemented",
        )
    })?;
    match route.input_options {
        InputOptions::Compose => {}
        InputOptions::Quadlet => {
            let name = arguments.application_name.as_deref().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "--application-name is required for Quadlet input",
                )
            })?;
            Identifier::new(name).map_err(io::Error::other)?;
            if ordered
                .iter()
                .any(|input| matches!(input, OrderedInput::File(path) if path == Path::new("-")))
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "stdin is not supported for Quadlet input",
                ));
            }
        }
    }
    match route.target_selector {
        TargetSelector::PodmanRange => {
            if arguments.pod_name.is_some() && !matches!(arguments.grouping, Grouping::Pod) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "--pod-name requires --quadlet-grouping pod",
                ));
            }
        }
        TargetSelector::ComposeSpecification => {}
    }
    Ok(route)
}

fn generic_quadlet_convert(
    arguments: &GenericConversion,
    ordered: Vec<OrderedInput>,
    route: RouteSpec,
    output_directory: Option<&Path>,
    validate_only: bool,
) -> Result<(ConversionReport, ExitCode, Vec<ResolvedInput>), Box<dyn Error>> {
    let discovered = resolve_quadlet_inputs(ordered)?;
    let mut aliases = ReportAliases::for_invocation(arguments, output_directory);
    aliases.add_inputs(&discovered);
    let (application_name, source) = load_quadlet_source(arguments, &discovered, &aliases)?;
    let (conversion, resolved_versions) = match route.executor {
        RouteExecutor::QuadletToCompose => {
            let target = TargetProfile::new(
                COMPOSE_SPECIFICATION_TARGET,
                COMPOSE_SPECIFICATION_PROFILE_REVISION,
                Some(COMPOSE_SPECIFICATION_PROFILE_REVISION),
            )?;
            let result = convert(
                &QuadletImporter::new()?,
                &source,
                &ComposeExporter::new()?,
                &target,
                arguments.loss_policy.into(),
            )
            .map_err(|error| quadlet_conversion_failure(&error, &aliases, &discovered))?;
            (
                RenderedConversion::from_result(result, |output| {
                    vec![RenderedFile {
                        name: "compose.yaml".into(),
                        text: output.text().to_owned(),
                    }]
                }),
                VersionBounds {
                    minimum: "rolling".into(),
                    maximum: "rolling".into(),
                },
            )
        }
        RouteExecutor::QuadletToQuadlet => {
            let (minimum, maximum) =
                resolve_versions(&arguments.podman_minimum_version, &arguments.podman_maximum_version).map_err(
                    |error| {
                        post_discovery_failure(
                            FailedStage::Conversion,
                            RuleId::PodmanTargetSelectionInvalid,
                            "Podman output version selection failed",
                            &error,
                            &aliases,
                            &discovered,
                        )
                    },
                )?;
            let mut exporter = QuadletExporter::new()?.with_grouping_policy(arguments.grouping.into());
            if let Some(pod_name) = arguments.pod_name.as_deref() {
                exporter = exporter.with_pod_name(pod_name);
            }
            let target = TargetProfile::new("podman", minimum, Some(maximum))?;
            let result = convert(
                &QuadletImporter::new()?,
                &source,
                &exporter,
                &target,
                arguments.loss_policy.into(),
            )
            .map_err(|error| quadlet_conversion_failure(&error, &aliases, &discovered))?;
            (
                RenderedConversion::from_result(result, |output| {
                    output
                        .files()
                        .iter()
                        .map(|file| RenderedFile {
                            name: file.name().as_str().to_owned(),
                            text: file.text().to_owned(),
                        })
                        .collect()
                }),
                VersionBounds {
                    minimum: minimum.to_string(),
                    maximum: maximum.to_string(),
                },
            )
        }
        _ => return Err(io::Error::other("Quadlet input selected a non-Quadlet route executor").into()),
    };
    finish_document_conversion(
        arguments,
        route,
        output_directory,
        validate_only,
        DocumentExecution {
            discovered,
            aliases,
            application_name: application_name.as_str().into(),
            resolved_versions,
            conversion,
        },
    )
}

fn quadlet_conversion_failure(
    error: &ConversionError,
    aliases: &ReportAliases,
    discovered: &[ResolvedInput],
) -> Box<dyn Error> {
    let mut diagnostics = Vec::new();
    match error {
        ConversionError::Import(import_diagnostics) => diagnostics.extend(
            import_diagnostics
                .iter()
                .map(|diagnostic| report_diagnostic(diagnostic, aliases)),
        ),
        ConversionError::InvalidPlan(_) => diagnostics.push(sanitized_diagnostic(
            RuleId::ConversionFailed,
            "error",
            "the Quadlet conversion produced an invalid conversion plan",
            &[],
            aliases,
        )),
        _ => diagnostics.push(sanitized_diagnostic(
            RuleId::ConversionFailed,
            "error",
            "the Quadlet conversion could not produce a valid plan",
            &[],
            aliases,
        )),
    }
    Box::new(StructuredFailure::with_context(
        FailedStage::Conversion,
        diagnostics,
        discovered,
    ))
}

fn load_quadlet_source(
    arguments: &GenericConversion,
    discovered: &[ResolvedInput],
    aliases: &ReportAliases,
) -> Result<(Identifier, QuadletSource), Box<dyn Error>> {
    let application_name = Identifier::new(
        arguments
            .application_name
            .as_deref()
            .ok_or_else(|| io::Error::other("missing application name"))?,
    )?;
    let mut documents = Vec::with_capacity(discovered.len());
    for (index, input) in discovered.iter().enumerate() {
        let path = input
            .path()
            .ok_or_else(|| io::Error::other("Quadlet input has no path"))?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Quadlet input filename is not UTF-8"))?;
        let text = fs::read_to_string(path).map_err(|error| {
            Box::new(StructuredFailure::with_context(
                FailedStage::InputRead,
                vec![sanitized_diagnostic(
                    RuleId::InputReadFailed,
                    "error",
                    "Quadlet input could not be read",
                    &[("input", name, false), ("reason", &error.to_string(), false)],
                    aliases,
                )],
                discovered,
            )) as Box<dyn Error>
        })?;
        documents.push(QuadletDocumentInput::new(
            name,
            boxferry::quadlet::quadlet_lens::source::SourceId::new(
                u32::try_from(index + 1).map_err(|_| io::Error::other("too many Quadlet input files"))?,
            ),
            text,
        ));
    }
    let parsed = QuadletSource::parse(application_name.clone(), documents)
        .map_err(|error| Box::new(quadlet_source_failure(&error, aliases, discovered)) as Box<dyn Error>)?;
    Ok((application_name, parsed.into_source()))
}

fn quadlet_source_failure(
    error: &QuadletParseError,
    aliases: &ReportAliases,
    inputs: &[ResolvedInput],
) -> StructuredFailure {
    let document_set_only = error
        .diagnostics()
        .iter()
        .all(|diagnostic| diagnostic.origin() == QuadletParseDiagnosticOrigin::DocumentSet)
        && error
            .failures()
            .iter()
            .all(|failure| matches!(failure.stage(), boxferry::QuadletParseFailureStage::DocumentSet));
    let stage = if document_set_only {
        FailedStage::QuadletDocumentSet
    } else {
        FailedStage::QuadletParse
    };
    let mut diagnostics = error
        .diagnostics()
        .iter()
        .map(|diagnostic| report_quadlet_native_diagnostic(diagnostic, aliases))
        .collect::<Vec<_>>();
    diagnostics.extend(error.failures().iter().map(|failure| {
        let native_stage = format!("{:?}", failure.stage()).to_lowercase();
        let input = quadlet_input_alias(failure.source_id(), failure.input_index());
        let mut report = sanitized_diagnostic(
            RuleId::QuadletParseFailed,
            "error",
            failure.summary(),
            &[("native_stage", &native_stage, false), ("input", &input, false)],
            aliases,
        );
        report.spans = failure.span().map_or_else(Vec::new, |label| {
            vec![boxferry::report::ReportSpan {
                source: format!("<input-{}>", label.source_id()),
                start: label.start(),
                end: label.end(),
            }]
        });
        report
    }));
    StructuredFailure::with_context(stage, diagnostics, inputs)
}

fn report_quadlet_native_diagnostic(diagnostic: &QuadletParseDiagnostic, aliases: &ReportAliases) -> ReportDiagnostic {
    let rule = match diagnostic.code().get(..3) {
        Some("QLS") => RuleId::QuadletNativeSyntax,
        Some("QLM") => RuleId::QuadletNativeModel,
        Some("QLG") => RuleId::QuadletNativeDocumentSet,
        _ => RuleId::QuadletNativeFailure,
    };
    let finding = diagnostic.native_finding();
    let mut report = sanitized_diagnostic(rule, severity_name(finding.severity()), finding.summary(), &[], aliases);
    let (native, fields, spans) = report_native_finding(&finding, aliases);
    report.source_code = Some(diagnostic.code().into());
    report.fields = fields;
    report.spans = spans;
    report.native_finding = Some(native);
    report
}

fn quadlet_input_alias(source_id: Option<u32>, input_index: Option<usize>) -> String {
    source_id.map_or_else(
        || input_index.map_or_else(|| "unknown".into(), |index| format!("<input-{}>", index + 1)),
        |source_id| format!("<input-{source_id}>"),
    )
}

fn finish_document_conversion(
    arguments: &GenericConversion,
    route: RouteSpec,
    output_directory: Option<&Path>,
    validate_only: bool,
    execution: DocumentExecution,
) -> Result<(ConversionReport, ExitCode, Vec<ResolvedInput>), Box<dyn Error>> {
    let DocumentExecution {
        discovered,
        aliases,
        application_name,
        resolved_versions,
        conversion,
    } = execution;
    let RenderedConversion {
        output,
        outcomes,
        diagnostics,
    } = conversion;
    let mut report = new_report(arguments, route);
    report.application = Some(application_name);
    report.resolved_versions = resolved_versions;
    (report.inputs, report.discovery) = resolved_report_context(&discovered);
    report.diagnostics = diagnostics
        .iter()
        .map(|diagnostic| report_diagnostic(diagnostic, &aliases))
        .collect();
    report.fidelity = fidelity_counts(&outcomes);
    let invalid = conversion_is_invalid(&outcomes, &diagnostics);
    if output.is_none() {
        report.primary_diagnostic_code = blocking_diagnostic_code(&outcomes, arguments.loss_policy);
    }
    if let Some(output) = &output {
        report.output_artifacts = output
            .iter()
            .map(|file| OutputArtifact {
                name: file.name.clone(),
                size: u64::try_from(file.text.len()).unwrap_or(u64::MAX),
            })
            .collect();
        if !validate_only {
            let directory = output_directory.ok_or_else(|| io::Error::other("missing convert output directory"))?;
            if let Err(error) = write_rendered_output(directory, output) {
                report.diagnostics.push(output_write_diagnostic(&error, &aliases));
                report.events.push("output-write-failed".into());
                report.status = ReportStatus::Failure;
                report.exit_category = ExitCategory::OutputWrite;
                report.failed_stage = Some(FailedStage::OutputWrite);
                return Ok((report, ExitCode::FAILURE, discovered));
            }
        }
        report.status = ReportStatus::Success;
        report.exit_category = ExitCategory::Success;
        Ok((report, ExitCode::SUCCESS, discovered))
    } else {
        report.status = if invalid {
            ReportStatus::Failure
        } else {
            ReportStatus::Blocked
        };
        report.exit_category = if invalid {
            ExitCategory::InputOrExecution
        } else {
            ExitCategory::PolicyBlocked
        };
        report.failed_stage = Some(FailedStage::Conversion);
        Ok((
            report,
            if invalid { ExitCode::FAILURE } else { ExitCode::from(2) },
            discovered,
        ))
    }
}

fn output_write_diagnostic(error: &io::Error, aliases: &ReportAliases) -> ReportDiagnostic {
    let message = error.to_string();
    let rule = if message.contains("output directory is not empty") {
        RuleId::OutputDirectoryNotEmpty
    } else if error.kind() == io::ErrorKind::InvalidInput
        || message.contains("not a non-symlink directory")
        || message.contains("output path")
    {
        RuleId::OutputPathInvalid
    } else {
        RuleId::OutputWriteFailed
    };
    sanitized_diagnostic(
        rule,
        "error",
        rule.definition().description(),
        &[("reason", &message, false)],
        aliases,
    )
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

fn conversion_is_invalid(outcomes: &[boxferry::ConversionOutcome], diagnostics: &[Diagnostic]) -> bool {
    outcomes.iter().any(|outcome| outcome.kind() == ConversionKind::Invalid)
        || diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity() == boxferry::Severity::Error)
}

fn blocking_diagnostic_code(outcomes: &[boxferry::ConversionOutcome], policy: CliLossPolicy) -> Option<String> {
    outcomes
        .iter()
        .find(|outcome| match outcome.kind() {
            ConversionKind::Exact => false,
            ConversionKind::Approximate => matches!(policy, CliLossPolicy::Exact),
            ConversionKind::Unsupported => !matches!(policy, CliLossPolicy::Partial),
            _ => true,
        })
        .and_then(|outcome| outcome.diagnostic())
        .map(|code| code.as_str().to_owned())
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
                let mut report = report_failure(
                    arguments,
                    route::find(arguments.input_type, arguments.output_type),
                    &error.to_string(),
                    structured.stage,
                    &aliases,
                );
                report.diagnostics.clone_from(&structured.diagnostics);
                report.inputs.clone_from(&structured.inputs);
                report.discovery.clone_from(&structured.discovery);
                if structured.stage == FailedStage::OutputWrite {
                    report.exit_category = ExitCategory::OutputWrite;
                }
                (report, ExitCode::FAILURE, structured.presentation_inputs.clone())
            } else {
                (
                    report_failure(
                        arguments,
                        route::find(arguments.input_type, arguments.output_type),
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
                RuleId::ReportWriteFailed,
                "error",
                RuleId::ReportWriteFailed.definition().description(),
                &[("reason", &error.to_string(), false)],
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
    let mut error_report_path = None;
    if arguments.generate_error_report {
        let encoded = serialize_report(&mut report)?;
        match write_generated_error_report(
            arguments.error_report_directory.as_deref(),
            &encoded,
            local_error_report_time,
            env::current_dir,
        ) {
            Ok(path) => error_report_path = Some(path),
            Err(error) => {
                report.diagnostics.push(sanitized_diagnostic(
                    RuleId::SupportBundleWriteFailed,
                    "error",
                    RuleId::SupportBundleWriteFailed.definition().description(),
                    &[("reason", &error.to_string(), false)],
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
    }
    present(
        arguments,
        &mut report,
        &inputs,
        validate_only,
        error_report_path.as_deref(),
    )?;
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
    error_report_path: Option<&Path>,
) -> Result<(), Box<dyn Error>> {
    refresh_report_outcome(report);
    let presentation = arguments.presentation;
    if matches!(presentation.console_format, Some(ConsoleFormat::Json)) {
        println!("{}", serialize_console_report(report, error_report_path)?);
        return Ok(());
    }
    if !presentation.quiet {
        println!("route: {} -> {}", report.source_type, report.target_type);
        for input in inputs {
            println!("input: {}", input.label());
        }
        if presentation.verbose {
            match route::find(arguments.input_type, arguments.output_type).map(|route| route.target_selector) {
                Some(TargetSelector::PodmanRange) => {
                    println!(
                        "Podman requested: {} through {}",
                        report.requested_versions.minimum, report.requested_versions.maximum
                    );
                    println!(
                        "Podman resolved: {} through {}",
                        report.resolved_versions.minimum, report.resolved_versions.maximum
                    );
                }
                Some(TargetSelector::ComposeSpecification) => println!("Compose Specification: rolling"),
                None => {}
            }
            if arguments.input_type == InputType::Compose {
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
        if report.status == ReportStatus::Success && presentation.verbose && !validate_only {
            for artifact in &report.output_artifacts {
                println!("wrote: {}", artifact.name);
            }
        }
    }
    if presentation.quiet {
        if let Some(path) = error_report_path {
            println!("{}", path.display());
        }
    } else if let Some(path) = error_report_path {
        println!("error report: {}", path.display());
    }
    if !presentation.quiet && !report.diagnostics.is_empty() {
        println!();
        io::stdout().flush()?;
    }
    print_report_diagnostics(report);
    print_fix_first(report);
    io::stderr().flush()?;
    if !presentation.quiet {
        println!();
        print_final_result(report, validate_only);
        io::stdout().flush()?;
    }
    Ok(())
}

fn print_final_result(report: &ConversionReport, validate_only: bool) {
    match report.status {
        ReportStatus::Success if validate_only => println!("boxferry: command succeeded; validation complete"),
        ReportStatus::Success => println!(
            "boxferry: command succeeded; wrote {} file(s) to output directory",
            report.output_artifacts.len()
        ),
        ReportStatus::Blocked => println!(
            "boxferry: command blocked by the selected loss policy: {}",
            report
                .failure_summary
                .as_deref()
                .unwrap_or("no structured cause was recorded")
        ),
        ReportStatus::Failure => println!(
            "boxferry: command failed during {}: {}",
            report.failed_stage.map_or("unknown stage", failed_stage_name),
            report
                .failure_summary
                .as_deref()
                .unwrap_or("no structured cause was recorded")
        ),
    }
}

const fn failed_stage_name(stage: FailedStage) -> &'static str {
    match stage {
        FailedStage::InputDiscovery => "input discovery",
        FailedStage::InputRead => "input read",
        FailedStage::Interpolation => "interpolation",
        FailedStage::ComposeLoad => "Compose load",
        FailedStage::ComposeMerge => "Compose merge",
        FailedStage::ProfileSelection => "profile selection",
        FailedStage::QuadletParse => "Quadlet parse",
        FailedStage::QuadletDocumentSet => "Quadlet document set",
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
}

struct LoadedComposeSource {
    source: ComposeSource,
    application: String,
    preprocessing_diagnostics: Vec<ReportDiagnostic>,
}

struct LoadedComposeInputs {
    documents: Vec<DocumentInput>,
    identities: Vec<(ComposeSourceId, SourceId)>,
}

fn load_compose_inputs(
    conversion: &ComposeConversion<'_>,
    aliases: &ReportAliases,
) -> Result<LoadedComposeInputs, Box<dyn Error>> {
    let mut identities = Vec::with_capacity(conversion.inputs.len());
    let mut documents = Vec::with_capacity(conversion.inputs.len());
    for (index, input) in conversion.inputs.iter().enumerate() {
        let id = ComposeSourceId::new(
            u32::try_from(index + 1).map_err(|_| io::Error::other("too many Compose input files"))?,
        );
        let (label, directory, text) = input.read(conversion.project_root).map_err(|error| {
            Box::new(StructuredFailure::with_context(
                FailedStage::InputRead,
                vec![sanitized_diagnostic(
                    RuleId::InputReadFailed,
                    "error",
                    "Compose input could not be read",
                    &[("reason", &error.to_string(), false)],
                    aliases,
                )],
                conversion.inputs,
            )) as Box<dyn Error>
        })?;
        documents.push(DocumentInput::new(
            id,
            DocumentOrigin::new(label.clone(), directory),
            text,
        ));
        identities.push((id, SourceId::new(label)?));
    }
    Ok(LoadedComposeInputs { documents, identities })
}

fn load_compose_source(
    conversion: &ComposeConversion<'_>,
    aliases: &ReportAliases,
) -> Result<LoadedComposeSource, Box<dyn Error>> {
    let inputs = load_compose_inputs(conversion, aliases)?;
    let loaded = LoadedProject::load(inputs.documents).map_err(|error| {
        Box::new(StructuredFailure::with_context(
            FailedStage::ComposeLoad,
            vec![sanitized_diagnostic(
                RuleId::ComposeLoadFailed,
                "error",
                "Compose input could not be loaded",
                &[("reason", &error.to_string(), false)],
                aliases,
            )],
            conversion.inputs,
        )) as Box<dyn Error>
    })?;
    let interpolated = conversion
        .interpolation
        .map(|environment| loaded.interpolate(environment));
    let merged = merge_project(&loaded, interpolated.as_ref());
    if !merged.is_valid() {
        let mut diagnostics = compose_diagnostics(
            merged.diagnostics(),
            &loaded,
            interpolated.as_ref(),
            ComposeFindingStage::Merge,
            aliases,
        );
        deduplicate_report_diagnostics(&mut diagnostics);
        return Err(Box::new(StructuredFailure::with_context(
            FailedStage::ComposeMerge,
            diagnostics,
            conversion.inputs,
        )));
    }
    let project = merged
        .project()
        .ok_or_else(|| io::Error::other("Compose merge produced no project"))?
        .clone();
    let request = compose_profile_request(conversion);
    let selection = select_profiles(&project, &request);
    if !selection.is_valid() {
        let mut diagnostics = compose_diagnostics(
            merged.diagnostics(),
            &loaded,
            interpolated.as_ref(),
            ComposeFindingStage::Merge,
            aliases,
        );
        diagnostics.extend(
            selection
                .diagnostics()
                .iter()
                .map(|diagnostic| compose_diagnostic(diagnostic, ComposeFindingStage::ProfileSelection, aliases)),
        );
        deduplicate_report_diagnostics(&mut diagnostics);
        return Err(Box::new(StructuredFailure::with_context(
            FailedStage::ProfileSelection,
            diagnostics,
            conversion.inputs,
        )));
    }
    let mut preprocessing_diagnostics = compose_diagnostics(
        merged.diagnostics(),
        &loaded,
        interpolated.as_ref(),
        ComposeFindingStage::Merge,
        aliases,
    );
    preprocessing_diagnostics.extend(
        selection
            .diagnostics()
            .iter()
            .map(|diagnostic| compose_diagnostic(diagnostic, ComposeFindingStage::ProfileSelection, aliases)),
    );
    deduplicate_report_diagnostics(&mut preprocessing_diagnostics);
    let application = boxferry::compose::compose_lens::project::build_project_view(&project, Some(&selection))
        .view()
        .and_then(|view| view.name())
        .and_then(|name| Identifier::new(name.value().clone()).ok())
        .map_or_else(|| conversion.fallback_name.to_owned(), |name| name.as_str().to_owned());
    let source = compose_source(
        project,
        conversion.fallback_name,
        inputs.identities,
        selection,
        &loaded,
        interpolated.as_ref(),
        &merged,
    )?;
    Ok(LoadedComposeSource {
        source,
        application,
        preprocessing_diagnostics,
    })
}

fn compose_profile_request(conversion: &ComposeConversion<'_>) -> ProfileRequest {
    if conversion.all_profiles {
        ProfileRequest::all()
    } else {
        conversion
            .profiles
            .iter()
            .fold(ProfileRequest::new(), ProfileRequest::with_profile)
    }
}

fn compose_diagnostics(
    diagnostics: &[ComposeDiagnostic],
    loaded: &LoadedProject,
    interpolated: Option<&ProjectInterpolation>,
    fallback: ComposeFindingStage,
    aliases: &ReportAliases,
) -> Vec<ReportDiagnostic> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            compose_diagnostic(
                diagnostic,
                compose_finding_stage(diagnostic, loaded, interpolated, fallback),
                aliases,
            )
        })
        .collect()
}

fn compose_source(
    project: MergedProject,
    fallback_name: &str,
    identities: Vec<(ComposeSourceId, SourceId)>,
    selection: ProfileSelection,
    loaded: &LoadedProject,
    interpolated: Option<&ProjectInterpolation>,
    merged: &MergeResult,
) -> Result<ComposeSource, boxferry::ModelError> {
    let interpolation_diagnostics = interpolated.map_or(&[][..], ProjectInterpolation::diagnostics);
    let merge_diagnostics = merged.diagnostics().iter().filter(|diagnostic| {
        !loaded.diagnostics().contains(diagnostic) && !interpolation_diagnostics.contains(diagnostic)
    });
    let mut source = ComposeSource::new(project, Identifier::new(fallback_name)?)?;
    for (compose, neutral) in identities {
        source = source.with_source_id(compose, neutral);
    }
    Ok(source
        .with_native_diagnostics(ComposeFindingStage::Load, loaded.diagnostics())
        .with_native_diagnostics(ComposeFindingStage::Interpolation, interpolation_diagnostics)
        .with_native_diagnostics(ComposeFindingStage::Merge, merge_diagnostics)
        .with_native_diagnostics(ComposeFindingStage::ProfileSelection, selection.diagnostics())
        .with_profile_selection(selection))
}

fn compose_finding_stage(
    diagnostic: &ComposeDiagnostic,
    loaded: &LoadedProject,
    interpolated: Option<&ProjectInterpolation>,
    fallback: ComposeFindingStage,
) -> ComposeFindingStage {
    if loaded.diagnostics().contains(diagnostic) {
        ComposeFindingStage::Load
    } else if interpolated.is_some_and(|result| result.diagnostics().contains(diagnostic)) {
        ComposeFindingStage::Interpolation
    } else {
        fallback
    }
}

fn compose_conversion_failure(
    error: &ConversionError,
    preprocessing_diagnostics: &[ReportDiagnostic],
    aliases: &ReportAliases,
    inputs: &[ResolvedInput],
) -> Box<dyn Error> {
    let mut diagnostics = Vec::new();
    if let ConversionError::Import(import_diagnostics) = error {
        diagnostics.extend(
            import_diagnostics
                .iter()
                .map(|diagnostic| report_diagnostic(diagnostic, aliases)),
        );
    } else {
        diagnostics.extend_from_slice(preprocessing_diagnostics);
        diagnostics.push(sanitized_diagnostic(
            RuleId::ConversionFailed,
            "error",
            &error.to_string(),
            &[],
            aliases,
        ));
    }
    deduplicate_report_diagnostics(&mut diagnostics);
    Box::new(StructuredFailure::with_context(
        FailedStage::Conversion,
        diagnostics,
        inputs,
    ))
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

fn resolve_quadlet_inputs(ordered: Vec<OrderedInput>) -> io::Result<Vec<ResolvedInput>> {
    let mut resolved = Vec::new();
    let mut paths = BTreeSet::new();
    let mut basenames = BTreeSet::new();
    for item in ordered {
        match item {
            OrderedInput::File(path) => {
                if path == Path::new("-") {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "stdin is not supported for Quadlet input",
                    ));
                }
                let path = resolve_regular_file(&path)?;
                if !is_supported_quadlet_path(&path) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Quadlet input must use a supported lower-case unit extension",
                    ));
                }
                resolved.push(ResolvedInput::File(path));
            }
            OrderedInput::Directory(path) => resolved.extend(discover_quadlet_directory(&path)?),
        }
    }
    for input in &resolved {
        let path = input
            .path()
            .ok_or_else(|| io::Error::other("Quadlet input has no path"))?;
        let canonical = fs::canonicalize(path)?;
        if !paths.insert(canonical) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "duplicate resolved Quadlet input",
            ));
        }
        let basename = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Quadlet input filename is not UTF-8"))?;
        if !basenames.insert(basename.to_owned()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "duplicate Quadlet unit basename",
            ));
        }
    }
    Ok(resolved)
}

fn discover_quadlet_directory(directory: &Path) -> io::Result<Vec<ResolvedInput>> {
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
    let mut selected = Vec::new();
    let mut ignored = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let supported = is_supported_quadlet_path(&path);
        let metadata = fs::symlink_metadata(&path)?;
        if supported && metadata.is_file() && !metadata.file_type().is_symlink() {
            selected.push(path);
        } else {
            ignored.push(path);
        }
    }
    selected.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
    ignored.sort();
    if selected.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no supported Quadlet unit files in {}", directory.display()),
        ));
    }
    Ok(selected
        .into_iter()
        .enumerate()
        .map(|(index, path)| ResolvedInput::Discovered {
            selected: path,
            ignored: if index == 0 { ignored.clone() } else { Vec::new() },
        })
        .collect())
}

fn is_supported_quadlet_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| QUADLET_EXTENSIONS.contains(&extension))
}

#[derive(Clone, Debug)]
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
    let catalogue_rule = find_rule(diagnostic.code().as_str()).map(|rule| rule.id());
    let rule = catalogue_rule.unwrap_or(RuleId::OrchestrationFailed);
    let mut report = sanitized_diagnostic(
        rule,
        severity_name(diagnostic.severity()),
        diagnostic
            .native_finding()
            .map_or_else(|| diagnostic.summary(), NativeFinding::summary),
        &fields,
        aliases,
    );
    if let Some(finding) = diagnostic.native_finding() {
        let (native, native_fields, spans) = report_native_finding(finding, aliases);
        for field in native_fields {
            if !report.fields.contains(&field) {
                report.fields.push(field);
            }
        }
        report.spans = spans;
        report.source_code = Some(finding.code().to_owned());
        report.native_finding = Some(native);
    } else if catalogue_rule.is_none() {
        report.source_code = Some(diagnostic.code().as_str().to_owned());
    }
    report
}

fn report_native_finding(
    finding: &NativeFinding,
    aliases: &ReportAliases,
) -> (ReportNativeFinding, Vec<ReportField>, Vec<boxferry::report::ReportSpan>) {
    let safe_text = |name: &str, value: &str| redact_text(name, &aliases.value(value), false).0;
    let fields = finding
        .fields()
        .iter()
        .map(|field| ReportField {
            name: field.name().to_owned(),
            value: redact_text(
                field.name(),
                &aliases.value(field.value().expose()),
                field.value().is_sensitive(),
            )
            .0,
        })
        .collect::<Vec<_>>();
    let labels = finding
        .labels()
        .iter()
        .map(|label| ReportNativeLabel {
            kind: match label.kind() {
                NativeFindingLabelKind::Primary => "primary",
                NativeFindingLabelKind::Secondary => "secondary",
                _ => "unknown",
            }
            .into(),
            source: format!("<input-{}>", label.source_id()),
            start: label.start(),
            end: label.end(),
            message: safe_text("label_message", label.message()),
        })
        .collect::<Vec<_>>();
    let spans = labels
        .iter()
        .map(|label| boxferry::report::ReportSpan {
            source: label.source.clone(),
            start: label.start,
            end: label.end,
        })
        .collect();
    let mut human_fields = fields.clone();
    for label in &labels {
        let field = ReportField {
            name: "label_message".into(),
            value: label.message.clone(),
        };
        if !human_fields.contains(&field) {
            human_fields.push(field);
        }
    }
    (
        ReportNativeFinding {
            source_format: safe_text("source_format", finding.source_format()),
            producer: safe_text("producer", finding.producer()),
            producer_version: finding
                .producer_version()
                .map(|version| safe_text("producer_version", version)),
            code: safe_text("source_code", finding.code()),
            stage: safe_text("native_stage", finding.stage()),
            severity: severity_name(finding.severity()).into(),
            summary: safe_text("summary", finding.summary()),
            fields,
            labels,
            notes: finding.notes().iter().map(|note| safe_text("note", note)).collect(),
            help: finding.help().map(|help| safe_text("help", help)),
        },
        human_fields,
        spans,
    )
}

fn compose_diagnostic(
    diagnostic: &boxferry::compose::compose_lens::diagnostic::Diagnostic,
    stage: ComposeFindingStage,
    aliases: &ReportAliases,
) -> ReportDiagnostic {
    let finding = stage.native_finding(diagnostic);
    let severity = severity_name(finding.severity());
    let rule = compose_native_rule(finding.code(), severity);
    let mut report = sanitized_diagnostic(rule, severity, finding.summary(), &[], aliases);
    let (native, fields, spans) = report_native_finding(&finding, aliases);
    report.source_code = Some(finding.code().into());
    report.fields = fields;
    report.spans = spans;
    report.native_finding = Some(native);
    report
}

fn severity_name(severity: boxferry::Severity) -> &'static str {
    match severity {
        boxferry::Severity::Error => "error",
        boxferry::Severity::Note => "note",
        _ => "warning",
    }
}

fn compose_native_rule(code: &str, severity: &str) -> RuleId {
    match code {
        "compose.interpolation.unset-variable" => RuleId::ComposeUnsetVariable,
        "compose.interpolation.required-variable" => RuleId::ComposeRequiredVariable,
        "compose.interpolation.invalid-expression" => RuleId::ComposeInterpolationInvalid,
        "compose.interpolation.nesting-limit" => RuleId::ComposeInterpolationNestingLimit,
        _ => match severity {
            "error" => RuleId::ComposeNativeError,
            "note" => RuleId::ComposeNativeNote,
            _ => RuleId::ComposeNativeWarning,
        },
    }
}

fn deduplicate_report_diagnostics(diagnostics: &mut Vec<ReportDiagnostic>) {
    let mut seen = BTreeSet::new();
    diagnostics.retain(|diagnostic| seen.insert(format!("{diagnostic:?}")));
}

fn diagnostic_occurrence_key(diagnostic: &ReportDiagnostic) -> (usize, usize) {
    diagnostic.spans.first().map_or((usize::MAX, usize::MAX), |span| {
        let input = span
            .source
            .strip_prefix("<input-")
            .and_then(|value| value.strip_suffix('>'))
            .and_then(|value| value.parse().ok())
            .unwrap_or(usize::MAX);
        (input, span.start)
    })
}

fn normalize_report_diagnostics(diagnostics: &mut Vec<ReportDiagnostic>) {
    deduplicate_report_diagnostics(diagnostics);
    diagnostics.sort_by(|left, right| {
        left.code
            .cmp(&right.code)
            .then_with(|| diagnostic_occurrence_key(left).cmp(&diagnostic_occurrence_key(right)))
    });
}

fn refresh_report_outcome(report: &mut ConversionReport) {
    normalize_report_diagnostics(&mut report.diagnostics);
    if report.status == ReportStatus::Success {
        report.primary_diagnostic_code = None;
        report.failure_summary = None;
        report.fix_first = None;
        return;
    }

    let stage_code = match report.failed_stage {
        Some(FailedStage::InputRead) => Some("BFO1001"),
        Some(FailedStage::Interpolation) => Some("BFO1004"),
        Some(FailedStage::ComposeLoad | FailedStage::ComposeMerge | FailedStage::ProfileSelection) => Some("BFO1005"),
        Some(FailedStage::QuadletParse | FailedStage::QuadletDocumentSet) => Some("BFO1007"),
        Some(FailedStage::OutputWrite) => Some("BFO2"),
        Some(FailedStage::ReportWrite) => Some("BFO3"),
        _ => None,
    };
    let retained = report
        .primary_diagnostic_code
        .as_deref()
        .and_then(|code| report.diagnostics.iter().find(|diagnostic| diagnostic.code == code));
    let primary = if report.status == ReportStatus::Blocked {
        retained.or_else(|| {
            report
                .diagnostics
                .iter()
                .find(|diagnostic| matches!(diagnostic.severity.as_str(), "error" | "warning"))
        })
    } else {
        stage_code
            .and_then(|code| {
                report
                    .diagnostics
                    .iter()
                    .find(|diagnostic| diagnostic.code == code || diagnostic.code.starts_with(code))
            })
            .or_else(|| {
                report
                    .diagnostics
                    .iter()
                    .find(|diagnostic| diagnostic.severity == "error")
            })
            .or(retained)
    };
    if let Some(primary) = primary.cloned() {
        let rule = find_rule(&primary.code);
        let description = rule.map_or_else(|| primary.summary.clone(), |rule| rule.description().to_owned());
        let help = rule.map_or_else(|| primary.help.clone(), |rule| rule.help().to_owned());
        report.primary_diagnostic_code = Some(primary.code.clone());
        report.failure_summary = Some(format!("{} {} — {}", primary.code, primary.name, primary.summary));
        report.fix_first = Some(FixFirst {
            code: primary.code,
            name: primary.name,
            description,
            help,
            next_step: "Apply this help, then rerun BoxFerry; remaining findings may disappear or change.".into(),
        });
    } else {
        report.primary_diagnostic_code = None;
        report.failure_summary = None;
        report.fix_first = None;
    }
}

fn print_report_diagnostics(report: &ConversionReport) {
    if let Some(context) = loss_policy_context(report) {
        eprintln!("policy: {context}");
        if !report.diagnostics.is_empty() {
            eprintln!();
        }
    }
    let diagnostics = &report.diagnostics;
    let mut start = 0;
    while start < diagnostics.len() {
        let code = &diagnostics[start].code;
        let mut end = start + 1;
        while end < diagnostics.len() && diagnostics[end].code == *code {
            end += 1;
        }
        if start > 0 {
            eprintln!();
        }
        let group = &diagnostics[start..end];
        let first = &group[0];
        if group.len() == 1 {
            eprintln!("{} {} [{}]", first.code, first.name, first.severity);
        } else {
            eprintln!(
                "{} {} [{}] ({} findings)",
                first.code,
                first.name,
                first.severity,
                group.len()
            );
        }

        let summary_is_common = group.iter().all(|diagnostic| diagnostic.summary == first.summary);
        let summary = if summary_is_common {
            first.summary.as_str()
        } else {
            find_rule(&first.code).map_or(first.summary.as_str(), |rule| rule.description())
        };
        eprintln!("  {summary}");

        let common_fields = common_report_fields(group);
        for field in &common_fields {
            eprintln!("  {}: {}", human_field_name(&field.name), field.value);
        }

        eprintln!("  findings:");
        for (offset, diagnostic) in group.iter().enumerate() {
            let mut details = diagnostic
                .fields
                .iter()
                .filter(|field| !common_fields.contains(field))
                .map(|field| (human_field_name(&field.name), field.value.clone()))
                .collect::<Vec<_>>();
            if !summary_is_common && !summary_is_represented_by_fields(diagnostic) {
                details.push(("message", diagnostic.summary.clone()));
            }
            for span in &diagnostic.spans {
                details.push(("location", format!("{}:{}-{}", span.source, span.start, span.end)));
            }

            if let Some((name, value)) = details.first() {
                eprintln!("    {}. {name}: {value}", offset + 1);
                for (name, value) in &details[1..] {
                    eprintln!("       {name}: {value}");
                }
            } else {
                eprintln!("    {}. condition reported", offset + 1);
            }
        }
        eprintln!("  help: {}", first.help);
        eprintln!("  explain: boxferry explain {}", first.code);
        start = end;
    }
}

fn print_fix_first(report: &ConversionReport) {
    let Some(fix_first) = &report.fix_first else {
        return;
    };
    if !report.diagnostics.is_empty() || loss_policy_context(report).is_some() {
        eprintln!();
    }
    eprintln!("fix first:");
    eprintln!("  {} {}", fix_first.code, fix_first.name);
    eprintln!("  {}", fix_first.description);
    eprintln!("  help: {}", fix_first.help);
    eprintln!("  next: {}", fix_first.next_step);
}

fn loss_policy_context(report: &ConversionReport) -> Option<String> {
    let policy = report
        .choices
        .iter()
        .find(|choice| choice.name == "loss_policy")
        .map(|choice| choice.value.as_str())?;
    if report.fidelity.invalid > 0 {
        return Some(format!(
            "invalid findings always block output and cannot be authorized by --loss-policy {policy}"
        ));
    }
    if report.status == ReportStatus::Blocked {
        return Some(format!(
            "--loss-policy {policy} does not authorize every reported non-exact finding"
        ));
    }
    ((report.fidelity.approximate > 0 || report.fidelity.unsupported > 0) && report.status == ReportStatus::Success)
        .then(|| format!("--loss-policy {policy} authorized output; non-exact findings remain visible"))
}

fn common_report_fields(group: &[ReportDiagnostic]) -> Vec<ReportField> {
    if group.len() < 2 {
        return Vec::new();
    }
    group[0]
        .fields
        .iter()
        .filter(|field| group[1..].iter().all(|diagnostic| diagnostic.fields.contains(field)))
        .cloned()
        .collect()
}

fn summary_is_represented_by_fields(diagnostic: &ReportDiagnostic) -> bool {
    matches!(diagnostic.code.as_str(), "BFC0101" | "BFC0102")
        && diagnostic.fields.iter().any(|field| field.name == "variable")
}

fn human_field_name(name: &str) -> &str {
    match name {
        "label_message" => "detail",
        "native_stage" => "native stage",
        other => other,
    }
}

fn serialize_report(report: &mut ConversionReport) -> Result<String, Box<dyn Error>> {
    refresh_report_outcome(report);
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
    let redacted_native = report
        .diagnostics
        .iter()
        .filter_map(|diagnostic| diagnostic.native_finding.as_ref())
        .map(|native| {
            usize::from(native.summary == boxferry::report::REDACTED)
                + usize::from(native.help.as_deref() == Some(boxferry::report::REDACTED))
                + native
                    .fields
                    .iter()
                    .filter(|field| field.value == boxferry::report::REDACTED)
                    .count()
                + native
                    .labels
                    .iter()
                    .filter(|label| label.message == boxferry::report::REDACTED)
                    .count()
                + native
                    .notes
                    .iter()
                    .filter(|note| note.as_str() == boxferry::report::REDACTED)
                    .count()
        })
        .sum::<usize>();
    report.redaction.count = redacted_summaries + redacted_fields + redacted_native;
    report.redaction.classes = match (redacted_summaries > 0, redacted_fields + redacted_native > 0) {
        (true, true) => vec!["plain-text-pattern".into(), "protected-value".into()],
        (true, false) => vec!["plain-text-pattern".into()],
        (false, true) => vec!["protected-value".into()],
        (false, false) => Vec::new(),
    };
    let mut encoded = serde_json::to_string(report)?;
    while encoded.len() > boxferry::report::MAX_JSON_BYTES && report.reduce_for_json() {
        refresh_report_outcome(report);
        encoded = serde_json::to_string(report)?;
    }
    if encoded.len() > boxferry::report::MAX_JSON_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "report exceeds the v1 JSON size cap").into());
    }
    Ok(encoded)
}

fn serialize_console_report(
    report: &mut ConversionReport,
    error_report_path: Option<&Path>,
) -> Result<String, Box<dyn Error>> {
    let encoded = serialize_report(report)?;
    let Some(path) = error_report_path else {
        return Ok(encoded);
    };
    let mut value: serde_json::Value = serde_json::from_str(&encoded)?;
    let object = value
        .as_object_mut()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "serialized report is not a JSON object"))?;
    object.insert(
        "error_report_path".into(),
        serde_json::Value::String(path.display().to_string()),
    );
    Ok(serde_json::to_string(&value)?)
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

fn local_error_report_time() -> io::Result<Zoned> {
    let time_zone = TimeZone::try_system().map_err(|error| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            format!("local wall-clock time is unavailable: {error}"),
        )
    })?;
    let timestamp = Timestamp::try_from(std::time::SystemTime::now()).map_err(|error| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            format!("local wall-clock time is unavailable: {error}"),
        )
    })?;
    Ok(timestamp.to_zoned(time_zone))
}

fn write_generated_error_report<Clock, CurrentDirectory>(
    configured_directory: Option<&Path>,
    report: &str,
    clock: Clock,
    current_directory: CurrentDirectory,
) -> io::Result<PathBuf>
where
    Clock: FnOnce() -> io::Result<Zoned>,
    CurrentDirectory: FnOnce() -> io::Result<PathBuf>,
{
    let directory = resolve_error_report_directory(configured_directory, current_directory)?;
    let time = clock()?;
    for collision in 0..=MAX_ERROR_REPORT_COLLISIONS {
        let path = directory.join(error_report_filename(&time, collision));
        match write_support_bundle(&path, report) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not publish a unique error report after exhausting all 100 same-second candidates",
    ))
}

fn resolve_error_report_directory<CurrentDirectory>(
    configured_directory: Option<&Path>,
    current_directory: CurrentDirectory,
) -> io::Result<PathBuf>
where
    CurrentDirectory: FnOnce() -> io::Result<PathBuf>,
{
    let directory = match configured_directory {
        Some(directory) => directory.to_path_buf(),
        None => current_directory()?,
    };
    let metadata = fs::symlink_metadata(&directory)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "error-report directory must be an existing non-symlink directory",
        ));
    }
    fs::canonicalize(directory)
}

fn error_report_filename(time: &Zoned, collision: u8) -> String {
    let suffix = if collision == 0 {
        String::new()
    } else {
        format!("-{collision:02}")
    };
    format!(
        "boxferry-error-report-{:04}-{:02}-{:02}_{:02}-{:02}-{:02}{suffix}.zip",
        time.year(),
        time.month(),
        time.day(),
        time.hour(),
        time.minute(),
        time.second(),
    )
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

fn prepare_output_directory(directory: &Path) -> io::Result<bool> {
    match fs::create_dir(directory) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(directory)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("output path is not a non-symlink directory: {}", directory.display()),
                ));
            }
            if fs::read_dir(directory)?.next().transpose()?.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("output directory is not empty: {}", directory.display()),
                ));
            }
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

fn cleanup_output_write(directory: &Path, created_directory: bool, created_files: &[PathBuf]) {
    for path in created_files {
        let _ = fs::remove_file(path);
    }
    if created_directory {
        let _ = fs::remove_dir(directory);
    }
}

fn write_rendered_output(directory: &Path, files: &[RenderedFile]) -> io::Result<()> {
    let created_directory = prepare_output_directory(directory)?;
    let mut created = Vec::with_capacity(files.len());
    for file in files {
        let path = directory.join(&file.name);
        let mut destination = match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(destination) => destination,
            Err(error) => {
                cleanup_output_write(directory, created_directory, &created);
                return Err(error);
            }
        };
        if let Err(error) = destination.write_all(file.text.as_bytes()) {
            created.push(path);
            cleanup_output_write(directory, created_directory, &created);
            return Err(error);
        }
        created.push(path);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use boxferry::compose::compose_lens::{
        interpolation::EnvironmentProvider,
        project::{ProjectValue, build_project_view},
    };
    fn fixed_error_report_time() -> io::Result<Zoned> {
        "2026-08-11T09:07:05Z"
            .parse::<Timestamp>()
            .map(|timestamp| timestamp.to_zoned(TimeZone::UTC))
            .map_err(io::Error::other)
    }

    #[test]
    fn cli_definition_is_internally_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn output_write_conditions_have_distinct_catalogued_rules() {
        let aliases = ReportAliases::default();
        for (error, expected) in [
            (
                io::Error::new(io::ErrorKind::AlreadyExists, "output directory is not empty"),
                "BFO2001",
            ),
            (
                io::Error::new(io::ErrorKind::InvalidInput, "output path is not a directory"),
                "BFO2002",
            ),
            (
                io::Error::new(io::ErrorKind::PermissionDenied, "write denied"),
                "BFO2003",
            ),
        ] {
            let diagnostic = output_write_diagnostic(&error, &aliases);
            assert_eq!(diagnostic.code, expected);
            assert_ne!(diagnostic.name, "uncatalogued-diagnostic");
            assert!(!diagnostic.help.is_empty());
        }
    }

    #[test]
    fn unknown_adapter_codes_fail_into_orchestration_with_source_provenance() -> Result<(), Box<dyn Error>> {
        let diagnostic = Diagnostic::new(
            boxferry::DiagnosticCode::new("BFX0001")?,
            boxferry::Severity::Warning,
            "extension diagnostic",
        );
        let report = report_diagnostic(&diagnostic, &ReportAliases::default());
        assert_eq!(report.code, "BFO1000");
        assert_eq!(report.source_code.as_deref(), Some("BFX0001"));
        Ok(())
    }

    #[test]
    fn finite_cli_values_accept_every_documented_spelling_and_reject_unknown_values() {
        fn value_names<T: ValueEnum>() -> Vec<String> {
            T::value_variants()
                .iter()
                .map(|value| {
                    value
                        .to_possible_value()
                        .map_or_else(|| "<hidden>".to_owned(), |value| value.get_name().to_owned())
                })
                .collect()
        }

        assert_eq!(value_names::<ConsoleFormat>(), vec!["json"]);
        assert_eq!(value_names::<OutputLayout>(), vec!["files"]);
        assert_eq!(value_names::<Grouping>(), vec!["separate", "pod"]);
        assert_eq!(value_names::<CliLossPolicy>(), vec!["exact", "approximate", "partial"]);

        let parse = |option: &str, value: &str| {
            let mut arguments = vec!["boxferry", "validate", "compose", "quadlet"];
            arguments.extend(["--input-file", "compose.yaml", option, value]);
            Cli::try_parse_from(arguments)
        };
        for (option, values) in [
            ("--console-format", ["json"].as_slice()),
            ("--output-layout", ["files"].as_slice()),
            ("--quadlet-grouping", ["separate", "pod"].as_slice()),
            ("--loss-policy", ["exact", "approximate", "partial"].as_slice()),
        ] {
            for value in values {
                assert!(parse(option, value).is_ok(), "{option}={value}");
            }
            assert!(parse(option, "unknown").is_err(), "{option} accepted an unknown value");
        }
        for (input, output) in [
            ("compose", "compose"),
            ("compose", "quadlet"),
            ("quadlet", "compose"),
            ("quadlet", "quadlet"),
        ] {
            let mut arguments = vec!["boxferry", "validate", input, output, "--input-file", "input"];
            if input == "quadlet" {
                arguments.extend(["--application-name", "example"]);
            }
            assert!(Cli::try_parse_from(arguments).is_ok(), "{input} -> {output}");
        }
        for arguments in [
            vec!["boxferry", "validate", "unknown", "quadlet"],
            vec!["boxferry", "validate", "compose", "unknown"],
            vec!["boxferry", "validate", "compose", "quadlet", "--input-type", "compose"],
            vec!["boxferry", "validate", "compose", "quadlet", "--output-type", "quadlet"],
        ] {
            assert!(Cli::try_parse_from(arguments).is_err());
        }
        for option in ["--podman-minimum-version", "--podman-maximum-version"] {
            for value in ["5.4", "5.4.0", "6.0", "6.0.2"] {
                assert!(parse(option, value).is_ok(), "{option}={value}");
            }
            for value in ["5", "5.4.0.1", "five.four"] {
                assert!(parse(option, value).is_err(), "{option} accepted {value}");
            }
        }
        for value in ["NAME", "NAME=value", "NAME="] {
            assert!(value.parse::<EnvironmentInput>().is_ok(), "--env={value}");
        }
        for value in ["", "=value", "1NAME", "NAME-WITH-DASH"] {
            assert!(value.parse::<EnvironmentInput>().is_err(), "--env accepted {value:?}");
        }
    }

    #[test]
    fn report_file_creation_is_cross_platform_and_never_overwrites() -> io::Result<()> {
        let directory = env::temp_dir().join(format!(
            "boxferry-report-create-new-{}-{}",
            std::process::id(),
            SUPPORT_BUNDLE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory)?;
        let report = directory.join("report.json");
        fs::write(&report, "original")?;
        let error = match write_report_file(&report, "replacement") {
            Ok(()) => return Err(io::Error::other("existing report was unexpectedly replaced")),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&report)?, "original");
        fs::remove_dir_all(directory)
    }

    #[test]
    fn generated_error_report_names_are_local_and_collision_safe() -> io::Result<()> {
        let directory = env::temp_dir().join(format!(
            "boxferry-generated-error-report-{}-{}",
            std::process::id(),
            SUPPORT_BUNDLE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory)?;
        let time = fixed_error_report_time()?;
        assert_eq!(
            error_report_filename(&time, 0),
            "boxferry-error-report-2026-08-11_09-07-05.zip"
        );
        assert_eq!(
            error_report_filename(&time, 1),
            "boxferry-error-report-2026-08-11_09-07-05-01.zip"
        );
        fs::write(directory.join(error_report_filename(&time, 0)), "existing")?;
        fs::write(directory.join(error_report_filename(&time, 1)), "existing")?;
        let report = write_generated_error_report(
            Some(&directory),
            "{}",
            || Ok(time.clone()),
            || unreachable!("configured directory does not use cwd"),
        )?;
        assert_eq!(
            report.file_name().and_then(|name| name.to_str()),
            Some("boxferry-error-report-2026-08-11_09-07-05-02.zip")
        );
        assert!(report.is_absolute());
        assert_eq!(
            fs::read_to_string(directory.join(error_report_filename(&time, 0)))?,
            "existing"
        );
        for collision in 0..=MAX_ERROR_REPORT_COLLISIONS {
            let path = directory.join(error_report_filename(&time, collision));
            if !path.exists() {
                fs::write(path, "existing")?;
            }
        }
        let exhaustion = match write_generated_error_report(
            Some(&directory),
            "{}",
            || Ok(time.clone()),
            || unreachable!("configured directory does not use cwd"),
        ) {
            Ok(path) => return Err(io::Error::other(format!("unexpected report at {}", path.display()))),
            Err(error) => error,
        };
        assert_eq!(exhaustion.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(
            exhaustion.to_string(),
            "could not publish a unique error report after exhausting all 100 same-second candidates"
        );
        fs::remove_dir_all(directory)
    }

    #[test]
    fn generated_error_report_rejects_bad_directories_and_clock_failures() -> io::Result<()> {
        let directory = env::temp_dir().join(format!(
            "boxferry-generated-error-report-failure-{}-{}",
            std::process::id(),
            SUPPORT_BUNDLE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory)?;
        let clock_failure = write_generated_error_report(
            Some(&directory),
            "{}",
            || Err(io::Error::other("clock unavailable")),
            || unreachable!("configured directory does not use cwd"),
        );
        assert!(clock_failure.is_err());
        assert!(fs::read_dir(&directory)?.next().is_none());
        let file = directory.join("not-a-directory");
        fs::write(&file, "not a directory")?;
        let invalid = write_generated_error_report(Some(&file), "{}", fixed_error_report_time, || {
            unreachable!("configured directory does not use cwd")
        });
        assert!(invalid.is_err());
        fs::remove_dir_all(directory)
    }

    #[test]
    fn generated_error_report_uses_the_current_directory_when_not_configured() -> io::Result<()> {
        let directory = env::temp_dir().join(format!(
            "boxferry-generated-error-report-cwd-{}-{}",
            std::process::id(),
            SUPPORT_BUNDLE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory)?;
        let report = write_generated_error_report(None, "{}", fixed_error_report_time, || Ok(directory.clone()))?;
        let canonical = fs::canonicalize(&directory)?;
        assert_eq!(report.parent(), Some(canonical.as_path()));
        fs::remove_dir_all(directory)
    }

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
            generate_error_report: false,
            error_report_directory: None,
            input_type: InputType::Compose,
            output_type: OutputType::Quadlet,
            input_files: Vec::new(),
            input_directories: Vec::new(),
            project_directory: None,
            application_name: None,
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
