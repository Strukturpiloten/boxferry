//! Stable `BoxFerry` diagnostic rule catalogue.

use crate::{DiagnosticCode, InvalidDiagnosticCode, Severity};

/// Stable identifier for one `BoxFerry` diagnostic rule.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[repr(u32)]
#[non_exhaustive]
#[allow(
    missing_docs,
    reason = "each variant is documented by its public catalogue definition"
)]
pub enum RuleId {
    ComposeModelInvalid = 1_000_001,
    ComposeProfileRequired = 1_000_002,
    ComposeProfileMismatch = 1_000_003,
    ComposeIntentUnsupported = 1_000_004,
    ComposeValueInvalid = 1_000_005,
    ComposeTargetInvalid = 1_000_006,
    ComposeOutputUnsupported = 1_000_007,
    ComposeGenerationFailed = 1_000_008,
    ComposeCompatibilityConstraint = 1_000_009,
    ComposeUnsetVariable = 1_000_101,
    ComposeRequiredVariable = 1_000_102,
    ComposeInterpolationInvalid = 1_000_103,
    ComposeInterpolationNestingLimit = 1_000_104,
    ComposeUnresolvedVariable = 1_000_105,
    ComposeNativeError = 1_000_197,
    ComposeNativeWarning = 1_000_198,
    ComposeNativeNote = 1_000_199,
    OrchestrationFailed = 3_001_000,
    InputReadFailed = 3_001_001,
    ComposeProjectRootInvalid = 3_001_002,
    PodmanTargetSelectionInvalid = 3_001_003,
    InterpolationInputInvalid = 3_001_004,
    ComposeLoadFailed = 3_001_005,
    ConversionFailed = 3_001_006,
    QuadletParseFailed = 3_001_007,
    OutputDirectoryNotEmpty = 3_002_001,
    OutputPathInvalid = 3_002_002,
    OutputWriteFailed = 3_002_003,
    ReportWriteFailed = 3_003_001,
    SupportBundleWriteFailed = 3_003_002,
    PodmanSourceInvalid = 4_000_001,
    PodmanInputUnmodelled = 4_000_002,
    PodmanPromotionRequired = 4_000_003,
    PodmanIdentityConflict = 4_000_004,
    PodmanSecretGrantIncomplete = 4_000_005,
    PodmanTargetInvalid = 4_000_006,
    PodmanOutputUnsupported = 4_000_007,
    PodmanPlanningFailed = 4_000_008,
    QuadletTargetInvalid = 5_000_001,
    QuadletOpenEndedTarget = 5_000_002,
    QuadletOutputUnsupported = 5_000_003,
    QuadletValueInvalid = 5_000_004,
    QuadletGenerationFailed = 5_000_005,
    QuadletCapabilityUnavailable = 5_000_006,
    QuadletGroupingApproximation = 5_000_007,
    QuadletDependencyUnsupported = 5_000_008,
    QuadletRestartApproximation = 5_000_009,
    QuadletEnvironmentFileApproximation = 5_000_010,
    QuadletGroupingInvalid = 5_000_011,
    QuadletDependencyInvalid = 5_000_012,
    QuadletCapabilityDeprecated = 5_000_013,
    QuadletUnresolvedVariable = 5_000_014,
    QuadletSourceInvalid = 5_001_001,
    QuadletModelInvalid = 5_001_002,
    QuadletInputUnsupported = 5_001_003,
    QuadletInputApproximation = 5_001_004,
    QuadletNativeSyntax = 5_001_101,
    QuadletNativeModel = 5_001_102,
    QuadletNativeDocumentSet = 5_001_103,
    QuadletNativeFailure = 5_001_104,
    PodmanStateReconstructed = 4_000_009,
    QuadletExternalEnvironmentUnverified = 5_000_015,
    DockerSourceInvalid = 2_000_001,
    DockerInputUnsupported = 2_000_002,
    DockerStateReconstructed = 2_000_003,
    DockerInputApproximation = 2_000_004,
    DockerTargetInvalid = 2_000_005,
    DockerOutputUnsupported = 2_000_006,
    DockerPlanningFailed = 2_000_007,
}

/// Immutable metadata shared by human, JSON, and support-bundle presentation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiagnosticRule {
    id: RuleId,
    code: &'static str,
    name: &'static str,
    default_severity: Severity,
    description: &'static str,
    help: &'static str,
}

impl DiagnosticRule {
    /// Returns the typed rule identifier.
    #[must_use]
    pub const fn id(self) -> RuleId {
        self.id
    }

    /// Returns the stable machine-readable rule code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        self.code
    }

    /// Returns the stable human-readable rule name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.name
    }

    /// Returns the rule's normal severity.
    #[must_use]
    pub const fn default_severity(self) -> Severity {
        self.default_severity
    }

    /// Returns the subsystem that owns this rule namespace.
    #[must_use]
    pub fn owner(self) -> &'static str {
        match self.code.as_bytes().get(..3) {
            Some(b"BFC") => "Compose adapter",
            Some(b"BFD") => match self.id {
                RuleId::DockerSourceInvalid
                | RuleId::DockerInputUnsupported
                | RuleId::DockerStateReconstructed
                | RuleId::DockerInputApproximation => "Docker source adapter",
                _ => "Docker target adapter",
            },
            Some(b"BFO") => "BoxFerry orchestration",
            Some(b"BFP") => "Podman adapter",
            Some(b"BFQ") => "Quadlet adapter",
            _ => "BoxFerry engine",
        }
    }

    /// Returns the value-free explanation of the condition.
    #[must_use]
    pub const fn description(self) -> &'static str {
        self.description
    }

    /// Returns static remediation guidance that never contains user data.
    #[must_use]
    pub const fn help(self) -> &'static str {
        self.help
    }

    /// Constructs the validated code used by structured engine diagnostics.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidDiagnosticCode`] if the repository-owned catalogue is malformed.
    pub fn diagnostic_code(self) -> Result<DiagnosticCode, InvalidDiagnosticCode> {
        DiagnosticCode::new(self.code)
    }
}

macro_rules! rule {
    ($id:ident, $code:literal, $name:literal, $severity:ident, $description:literal, $help:literal) => {
        DiagnosticRule {
            id: RuleId::$id,
            code: $code,
            name: $name,
            default_severity: Severity::$severity,
            description: $description,
            help: $help,
        }
    };
}

/// Complete, code-sorted catalogue for this `BoxFerry` build.
pub const RULES: &[DiagnosticRule] = &[
    rule!(
        ComposeModelInvalid,
        "BFC0001",
        "compose-model-invalid",
        Error,
        "Compose intent could not be represented by the neutral application model.",
        "Correct the reported Compose value or remove the conflicting declaration."
    ),
    rule!(
        ComposeProfileRequired,
        "BFC0002",
        "compose-profile-selection-required",
        Error,
        "Profiled Compose services require an explicit profile selection.",
        "Select profiles explicitly with --profile, or use --all-profiles only after reviewing mutually exclusive services."
    ),
    rule!(
        ComposeProfileMismatch,
        "BFC0003",
        "compose-profile-selection-mismatch",
        Error,
        "The Compose profile selection is invalid or belongs to another merged project.",
        "Recreate the profile selection from the same merged Compose project."
    ),
    rule!(
        ComposeIntentUnsupported,
        "BFC0004",
        "compose-intent-unsupported",
        Warning,
        "Compose intent is not represented by the current neutral-model subset.",
        "Review the named feature; use --loss-policy partial only when omitting that intent is acceptable."
    ),
    rule!(
        ComposeValueInvalid,
        "BFC0005",
        "compose-value-invalid",
        Error,
        "A Compose value cannot be represented safely in the neutral model.",
        "Correct the reported value before converting again."
    ),
    rule!(
        ComposeTargetInvalid,
        "BFC0006",
        "compose-target-invalid",
        Error,
        "The requested Compose output target is invalid.",
        "Select a supported Compose target profile and compatible version inputs."
    ),
    rule!(
        ComposeOutputUnsupported,
        "BFC0007",
        "compose-output-unsupported",
        Warning,
        "Neutral application intent is not represented in generated Compose output.",
        "Review the named subject. Protected service network aliases block every loss policy; keep private alias configuration outside the generated document. Protected vendor metadata also blocks every loss policy because fresh Compose text cannot retain sensitivity. Otherwise, use --loss-policy partial only when a candidate exists and omitted intent is acceptable."
    ),
    rule!(
        ComposeGenerationFailed,
        "BFC0008",
        "compose-generation-failed",
        Error,
        "ComposeLens rejected generated Compose output.",
        "Correct the reported neutral value or target choice and retry."
    ),
    rule!(
        ComposeCompatibilityConstraint,
        "BFC0009",
        "compose-compatibility-constraint",
        Warning,
        "Generated Compose syntax has a target-specific compatibility constraint.",
        "Review the target-specific classification before authorizing non-exact output."
    ),
    rule!(
        ComposeUnsetVariable,
        "BFC0101",
        "compose-unset-variable",
        Warning,
        "A Compose interpolation variable is not set.",
        "Provide the missing value with --env-file FILE or --env NAME=VALUE."
    ),
    rule!(
        ComposeRequiredVariable,
        "BFC0102",
        "compose-required-variable",
        Error,
        "A required Compose interpolation variable is not set.",
        "Provide the required value with --env-file FILE or --env NAME=VALUE."
    ),
    rule!(
        ComposeInterpolationInvalid,
        "BFC0103",
        "compose-interpolation-invalid",
        Error,
        "A Compose interpolation expression is invalid.",
        "Correct the interpolation expression and retry."
    ),
    rule!(
        ComposeInterpolationNestingLimit,
        "BFC0104",
        "compose-interpolation-nesting-limit",
        Error,
        "A Compose interpolation expression exceeds the supported nesting limit.",
        "Simplify the interpolation expression before converting."
    ),
    rule!(
        ComposeUnresolvedVariable,
        "BFC0105",
        "compose-unresolved-variable",
        Warning,
        "A Compose variable expression remains unresolved at the adapter boundary.",
        "Provide missing values with --env-file FILE or --env NAME=VALUE; --interpolate also permits process-variable fallback. Partial authorization applies only when the affected intent can be omitted."
    ),
    rule!(
        ComposeNativeError,
        "BFC0197",
        "compose-native-error",
        Error,
        "ComposeLens reported a native Compose error.",
        "Use the retained source code and source span to correct the Compose document."
    ),
    rule!(
        ComposeNativeWarning,
        "BFC0198",
        "compose-native-warning",
        Warning,
        "ComposeLens reported a native Compose warning.",
        "Review the retained source code and source span before accepting the document."
    ),
    rule!(
        ComposeNativeNote,
        "BFC0199",
        "compose-native-note",
        Note,
        "ComposeLens reported native Compose information.",
        "Review the retained source code when more context is needed."
    ),
    rule!(
        DockerSourceInvalid,
        "BFD0001",
        "docker-source-invalid",
        Error,
        "A selected Docker observation cannot be mapped safely to neutral intent.",
        "Correct the selected source or inspect the reported native field before retrying."
    ),
    rule!(
        DockerInputUnsupported,
        "BFD0002",
        "docker-input-unsupported",
        Warning,
        "A Docker observation remains evidence because its portable intent is unproved or unmapped.",
        "Review and author the affected field before accepting a target conversion."
    ),
    rule!(
        DockerStateReconstructed,
        "BFD0003",
        "docker-state-reconstructed",
        Note,
        "A reviewed effective Docker observation was reconstructed as a current-state candidate, not proven authorship.",
        "Review the original authored-versus-default distinction before treating the candidate as maintained intent."
    ),
    rule!(
        DockerInputApproximation,
        "BFD0004",
        "docker-input-approximation",
        Warning,
        "A Docker source decision requires an explicit assumption or may change behavior.",
        "Review the reported source field and accept or repair the approximation before target conversion."
    ),
    rule!(
        DockerTargetInvalid,
        "BFD0005",
        "docker-target-invalid",
        Error,
        "The Docker target does not match an exact reviewed native profile or the requested version range.",
        "Select one DockerLens reviewed target profile and a compatible BoxFerry version range."
    ),
    rule!(
        DockerOutputUnsupported,
        "BFD0006",
        "docker-output-unsupported",
        Warning,
        "The Docker target cannot represent this neutral application field under the reviewed contract.",
        "Review the subject and author a supported standalone shape, or explicitly allow partial loss."
    ),
    rule!(
        DockerPlanningFailed,
        "BFD0007",
        "docker-planning-failed",
        Error,
        "DockerLens rejected the typed standalone target or inert request rendering.",
        "Correct the reported subject and select a reviewed native target capability profile."
    ),
    rule!(
        OrchestrationFailed,
        "BFO1000",
        "orchestration-failed",
        Error,
        "BoxFerry could not complete the requested orchestration stage.",
        "Review the reason and correct the command inputs before retrying."
    ),
    rule!(
        InputReadFailed,
        "BFO1001",
        "input-read-failed",
        Error,
        "An explicitly selected input could not be read.",
        "Check that the input exists, is a readable regular file, and is not a symlink."
    ),
    rule!(
        ComposeProjectRootInvalid,
        "BFO1002",
        "compose-project-root-invalid",
        Error,
        "The Compose project root could not be resolved.",
        "Provide an existing absolute --project-directory or correct the input location."
    ),
    rule!(
        PodmanTargetSelectionInvalid,
        "BFO1003",
        "podman-target-selection-invalid",
        Error,
        "The requested Podman target range could not be resolved.",
        "Use major.minor or major.minor.patch values within the reviewed range."
    ),
    rule!(
        InterpolationInputInvalid,
        "BFO1004",
        "interpolation-input-invalid",
        Error,
        "Compose interpolation inputs could not be resolved safely.",
        "Correct the named --env-file or --env input and retry."
    ),
    rule!(
        ComposeLoadFailed,
        "BFO1005",
        "compose-load-failed",
        Error,
        "Compose input could not be loaded or merged.",
        "Correct the reported Compose source diagnostic before retrying."
    ),
    rule!(
        ConversionFailed,
        "BFO1006",
        "conversion-failed",
        Error,
        "Source import or target conversion failed.",
        "Correct the reported rule occurrences before retrying."
    ),
    rule!(
        QuadletParseFailed,
        "BFO1007",
        "quadlet-parse-failed",
        Error,
        "Quadlet input could not be parsed into the native document boundary.",
        "Correct the retained native diagnostics before retrying."
    ),
    rule!(
        OutputDirectoryNotEmpty,
        "BFO2001",
        "output-directory-not-empty",
        Error,
        "The selected output directory is not empty.",
        "Empty the selected --output-directory or choose a new path."
    ),
    rule!(
        OutputPathInvalid,
        "BFO2002",
        "output-path-invalid",
        Error,
        "The selected output path is not a usable non-symlink directory.",
        "Choose an absent path or an existing empty, non-symlink directory."
    ),
    rule!(
        OutputWriteFailed,
        "BFO2003",
        "output-write-failed",
        Error,
        "Generated output could not be written safely.",
        "Check parent-directory existence, permissions, free space, and path conflicts."
    ),
    rule!(
        ReportWriteFailed,
        "BFO3001",
        "report-write-failed",
        Error,
        "The structured report file could not be written safely.",
        "Choose a new writable --report-file path."
    ),
    rule!(
        SupportBundleWriteFailed,
        "BFO3002",
        "support-bundle-write-failed",
        Error,
        "The diagnostic support bundle could not be written safely.",
        "Choose a writable --error-report-directory with space for a new archive."
    ),
    rule!(
        PodmanSourceInvalid,
        "BFP0001",
        "podman-source-invalid",
        Error,
        "Inspected Podman evidence is malformed, incomplete, or unavailable for a required mapping.",
        "Correct the selected Podman resources or selectors, then acquire a complete read-only inventory again."
    ),
    rule!(
        PodmanInputUnmodelled,
        "BFP0002",
        "podman-input-unmodelled",
        Warning,
        "Inspected Podman intent is not represented by the current neutral application model.",
        "Review the named field; use --loss-policy partial only when omitting that intent is acceptable."
    ),
    rule!(
        PodmanPromotionRequired,
        "BFP0003",
        "podman-promotion-required",
        Warning,
        "Effective or locally resolved Podman evidence requires an explicit promotion decision.",
        "Choose an explicit promotion policy only after confirming the observed value is portable desired state."
    ),
    rule!(
        PodmanIdentityConflict,
        "BFP0004",
        "podman-identity-conflict",
        Error,
        "Podman resource identities or references conflict in the neutral namespace.",
        "Narrow the selectors or rename the conflicting resources before importing again."
    ),
    rule!(
        PodmanSecretGrantIncomplete,
        "BFP0005",
        "podman-secret-grant-incomplete",
        Warning,
        "Podman inspection lacks authored secret-delivery information required for a portable grant.",
        "Author the secret delivery form and target separately; inspection metadata alone is insufficient."
    ),
    rule!(
        PodmanTargetInvalid,
        "BFP0006",
        "podman-target-invalid",
        Error,
        "The requested Podman output target is outside the reviewed exact version and execution-context catalogue.",
        "Select a reviewed --podman-max-version and state the target rootful, rootless, or unknown context explicitly."
    ),
    rule!(
        PodmanOutputUnsupported,
        "BFP0007",
        "podman-output-unsupported",
        Warning,
        "Neutral application intent is not represented by the current Podman deployment subset.",
        "Review the named subject; use --loss-policy partial only when the omitted intent is acceptable."
    ),
    rule!(
        PodmanPlanningFailed,
        "BFP0008",
        "podman-planning-failed",
        Error,
        "PodmanLens could not produce or render a complete reviewable deployment plan.",
        "Review retained PodmanLens findings, correct the neutral intent or target, and retry."
    ),
    rule!(
        PodmanStateReconstructed,
        "BFP0009",
        "podman-state-reconstructed",
        Note,
        "Reviewed effective Podman state was reconstructed as portable application intent without a known behavior change.",
        "Review the named field and target prerequisites; the original authored versus default choice cannot be recovered from inspection."
    ),
    rule!(
        QuadletTargetInvalid,
        "BFQ0001",
        "quadlet-target-invalid",
        Error,
        "The requested Quadlet target or Podman range is invalid.",
        "Select the podman target and a version range covered by QuadletLens."
    ),
    rule!(
        QuadletOpenEndedTarget,
        "BFQ0002",
        "quadlet-open-ended-target",
        Note,
        "An omitted Podman maximum extends beyond verified compatibility evidence.",
        "Set --podman-maximum-version for a finite compatibility claim."
    ),
    rule!(
        QuadletOutputUnsupported,
        "BFQ0003",
        "quadlet-output-unsupported",
        Warning,
        "Neutral application intent is not represented by the current Quadlet subset.",
        "Review the named subject; use --loss-policy partial only when omission is acceptable."
    ),
    rule!(
        QuadletValueInvalid,
        "BFQ0004",
        "quadlet-value-invalid",
        Error,
        "A neutral value cannot be emitted safely as native Quadlet syntax.",
        "Correct or explicitly map the reported value before converting again."
    ),
    rule!(
        QuadletGenerationFailed,
        "BFQ0005",
        "quadlet-generation-failed",
        Error,
        "QuadletLens rejected generated native output.",
        "Correct the reported value or dependency and retry."
    ),
    rule!(
        QuadletCapabilityUnavailable,
        "BFQ0006",
        "quadlet-capability-unavailable",
        Warning,
        "A required Quadlet capability is unavailable for part of the target range.",
        "Narrow the Podman target range or accept the documented partial result."
    ),
    rule!(
        QuadletGroupingApproximation,
        "BFQ0007",
        "quadlet-grouping-approximation",
        Warning,
        "Generated Quadlet grouping changes source service isolation.",
        "Use --quadlet-grouping pod only when the shared-pod semantics are acceptable."
    ),
    rule!(
        QuadletDependencyUnsupported,
        "BFQ0008",
        "quadlet-dependency-unsupported",
        Warning,
        "A service dependency cannot be represented exactly by Quadlet and systemd.",
        "Review the dependency condition and complete unsupported behavior manually."
    ),
    rule!(
        QuadletRestartApproximation,
        "BFQ0009",
        "quadlet-restart-policy-approximation",
        Warning,
        "Container restart behavior is approximated by the systemd service manager.",
        "Use --loss-policy approximate only after accepting the documented systemd behavior difference."
    ),
    rule!(
        QuadletEnvironmentFileApproximation,
        "BFQ0010",
        "quadlet-environment-file-approximation",
        Warning,
        "Environment-file parsing is delegated to Podman.",
        "Verify the environment file with the target Podman version before deployment."
    ),
    rule!(
        QuadletGroupingInvalid,
        "BFQ0011",
        "quadlet-grouping-invalid",
        Error,
        "The requested Quadlet grouping cannot preserve the application topology.",
        "Keep separate containers or correct the reported grouping conflict."
    ),
    rule!(
        QuadletDependencyInvalid,
        "BFQ0012",
        "quadlet-dependency-invalid",
        Error,
        "The service or image-artifact dependency graph is invalid.",
        "Correct missing or cyclic dependencies before converting."
    ),
    rule!(
        QuadletCapabilityDeprecated,
        "BFQ0013",
        "quadlet-capability-deprecated",
        Note,
        "A required Quadlet capability is deprecated for part of the target range.",
        "Review the target range and plan migration away from the deprecated capability."
    ),
    rule!(
        QuadletUnresolvedVariable,
        "BFQ0014",
        "quadlet-unresolved-source-variable",
        Error,
        "A source variable expression cannot be emitted as a Quadlet value.",
        "Resolve source variables before conversion; for Compose input, supply --env-file FILE or --env NAME=VALUE, or use --interpolate for process-variable fallback."
    ),
    rule!(
        QuadletExternalEnvironmentUnverified,
        "BFQ0015",
        "quadlet-external-environment-unverified",
        Note,
        "Service execution environment prerequisites are external-unverified.",
        "Provision mandatory service environment files and reference values on the target; bind roots must resolve to nonempty absolute paths without colons or control characters. Conversion never reads providers or verifies runtime availability."
    ),
    rule!(
        QuadletSourceInvalid,
        "BFQ1001",
        "quadlet-source-invalid",
        Error,
        "The Quadlet document set is invalid.",
        "Correct native Quadlet syntax and document-set errors before converting."
    ),
    rule!(
        QuadletModelInvalid,
        "BFQ1002",
        "quadlet-model-invalid",
        Error,
        "Quadlet intent cannot be represented by the neutral application model.",
        "Correct the reported native value or relationship."
    ),
    rule!(
        QuadletInputUnsupported,
        "BFQ1003",
        "quadlet-input-unsupported",
        Warning,
        "Quadlet intent is outside the current neutral-model importer subset.",
        "Review the named key; use --loss-policy partial only when omission is acceptable."
    ),
    rule!(
        QuadletInputApproximation,
        "BFQ1004",
        "quadlet-input-approximation",
        Warning,
        "Quadlet intent is approximated in the neutral model.",
        "Review the documented semantic difference before authorizing approximate output."
    ),
    rule!(
        QuadletNativeSyntax,
        "BFQ1101",
        "quadlet-native-syntax",
        Error,
        "QuadletLens reported a native syntax diagnostic.",
        "Use the retained source code and source span to correct the Quadlet unit."
    ),
    rule!(
        QuadletNativeModel,
        "BFQ1102",
        "quadlet-native-model",
        Error,
        "QuadletLens reported a native model diagnostic.",
        "Use the retained source code and source span to correct the Quadlet value."
    ),
    rule!(
        QuadletNativeDocumentSet,
        "BFQ1103",
        "quadlet-native-document-set",
        Error,
        "QuadletLens reported a native document-set diagnostic.",
        "Correct missing, ambiguous, or invalid cross-document relationships."
    ),
    rule!(
        QuadletNativeFailure,
        "BFQ1104",
        "quadlet-native-failure",
        Error,
        "QuadletLens could not construct the native input boundary.",
        "Correct the reported input and native stage before retrying."
    ),
];

impl RuleId {
    /// Returns this rule's immutable catalogue definition.
    #[must_use]
    pub const fn definition(self) -> &'static DiagnosticRule {
        match self {
            Self::ComposeModelInvalid => &RULES[0],
            Self::ComposeProfileRequired => &RULES[1],
            Self::ComposeProfileMismatch => &RULES[2],
            Self::ComposeIntentUnsupported => &RULES[3],
            Self::ComposeValueInvalid => &RULES[4],
            Self::ComposeTargetInvalid => &RULES[5],
            Self::ComposeOutputUnsupported => &RULES[6],
            Self::ComposeGenerationFailed => &RULES[7],
            Self::ComposeCompatibilityConstraint => &RULES[8],
            Self::ComposeUnsetVariable => &RULES[9],
            Self::ComposeRequiredVariable => &RULES[10],
            Self::ComposeInterpolationInvalid => &RULES[11],
            Self::ComposeInterpolationNestingLimit => &RULES[12],
            Self::ComposeUnresolvedVariable => &RULES[13],
            Self::ComposeNativeError => &RULES[14],
            Self::ComposeNativeWarning => &RULES[15],
            Self::ComposeNativeNote => &RULES[16],
            Self::DockerSourceInvalid => &RULES[17],
            Self::DockerInputUnsupported => &RULES[18],
            Self::DockerStateReconstructed => &RULES[19],
            Self::DockerInputApproximation => &RULES[20],
            Self::DockerTargetInvalid => &RULES[21],
            Self::DockerOutputUnsupported => &RULES[22],
            Self::DockerPlanningFailed => &RULES[23],
            Self::OrchestrationFailed => &RULES[24],
            Self::InputReadFailed => &RULES[25],
            Self::ComposeProjectRootInvalid => &RULES[26],
            Self::PodmanTargetSelectionInvalid => &RULES[27],
            Self::InterpolationInputInvalid => &RULES[28],
            Self::ComposeLoadFailed => &RULES[29],
            Self::ConversionFailed => &RULES[30],
            Self::QuadletParseFailed => &RULES[31],
            Self::OutputDirectoryNotEmpty => &RULES[32],
            Self::OutputPathInvalid => &RULES[33],
            Self::OutputWriteFailed => &RULES[34],
            Self::ReportWriteFailed => &RULES[35],
            Self::SupportBundleWriteFailed => &RULES[36],
            Self::PodmanSourceInvalid => &RULES[37],
            Self::PodmanInputUnmodelled => &RULES[38],
            Self::PodmanPromotionRequired => &RULES[39],
            Self::PodmanIdentityConflict => &RULES[40],
            Self::PodmanSecretGrantIncomplete => &RULES[41],
            Self::PodmanTargetInvalid => &RULES[42],
            Self::PodmanOutputUnsupported => &RULES[43],
            Self::PodmanPlanningFailed => &RULES[44],
            Self::PodmanStateReconstructed => &RULES[45],
            Self::QuadletTargetInvalid => &RULES[46],
            Self::QuadletOpenEndedTarget => &RULES[47],
            Self::QuadletOutputUnsupported => &RULES[48],
            Self::QuadletValueInvalid => &RULES[49],
            Self::QuadletGenerationFailed => &RULES[50],
            Self::QuadletCapabilityUnavailable => &RULES[51],
            Self::QuadletGroupingApproximation => &RULES[52],
            Self::QuadletDependencyUnsupported => &RULES[53],
            Self::QuadletRestartApproximation => &RULES[54],
            Self::QuadletEnvironmentFileApproximation => &RULES[55],
            Self::QuadletGroupingInvalid => &RULES[56],
            Self::QuadletDependencyInvalid => &RULES[57],
            Self::QuadletCapabilityDeprecated => &RULES[58],
            Self::QuadletUnresolvedVariable => &RULES[59],
            Self::QuadletSourceInvalid => &RULES[61],
            Self::QuadletModelInvalid => &RULES[62],
            Self::QuadletInputUnsupported => &RULES[63],
            Self::QuadletInputApproximation => &RULES[64],
            Self::QuadletNativeSyntax => &RULES[65],
            Self::QuadletNativeModel => &RULES[66],
            Self::QuadletNativeDocumentSet => &RULES[67],
            Self::QuadletExternalEnvironmentUnverified => &RULES[60],
            Self::QuadletNativeFailure => &RULES[68],
        }
    }
}

/// Finds a rule by exact code or human-readable name.
#[must_use]
pub fn find_rule(value: &str) -> Option<&'static DiagnosticRule> {
    RULES
        .iter()
        .find(|rule| rule.code.eq_ignore_ascii_case(value) || rule.name.eq_ignore_ascii_case(value))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{RULES, RuleId, find_rule};

    #[test]
    fn historical_rule_discriminants_remain_stable() {
        assert_eq!(RuleId::ComposeModelInvalid as u32, 1_000_001);
        assert_eq!(RuleId::ComposeProfileRequired as u32, 1_000_002);
        assert_eq!(RuleId::ComposeProfileMismatch as u32, 1_000_003);
        assert_eq!(RuleId::ComposeIntentUnsupported as u32, 1_000_004);
        assert_eq!(RuleId::ComposeValueInvalid as u32, 1_000_005);
        assert_eq!(RuleId::ComposeTargetInvalid as u32, 1_000_006);
        assert_eq!(RuleId::ComposeOutputUnsupported as u32, 1_000_007);
        assert_eq!(RuleId::ComposeGenerationFailed as u32, 1_000_008);
        assert_eq!(RuleId::ComposeCompatibilityConstraint as u32, 1_000_009);
        assert_eq!(RuleId::ComposeUnsetVariable as u32, 1_000_101);
        assert_eq!(RuleId::ComposeRequiredVariable as u32, 1_000_102);
        assert_eq!(RuleId::ComposeInterpolationInvalid as u32, 1_000_103);
        assert_eq!(RuleId::ComposeInterpolationNestingLimit as u32, 1_000_104);
        assert_eq!(RuleId::ComposeUnresolvedVariable as u32, 1_000_105);
        assert_eq!(RuleId::ComposeNativeError as u32, 1_000_197);
        assert_eq!(RuleId::ComposeNativeWarning as u32, 1_000_198);
        assert_eq!(RuleId::ComposeNativeNote as u32, 1_000_199);
        assert_eq!(RuleId::OrchestrationFailed as u32, 3_001_000);
        assert_eq!(RuleId::InputReadFailed as u32, 3_001_001);
        assert_eq!(RuleId::ComposeProjectRootInvalid as u32, 3_001_002);
        assert_eq!(RuleId::PodmanTargetSelectionInvalid as u32, 3_001_003);
        assert_eq!(RuleId::InterpolationInputInvalid as u32, 3_001_004);
        assert_eq!(RuleId::ComposeLoadFailed as u32, 3_001_005);
        assert_eq!(RuleId::ConversionFailed as u32, 3_001_006);
        assert_eq!(RuleId::QuadletParseFailed as u32, 3_001_007);
        assert_eq!(RuleId::OutputDirectoryNotEmpty as u32, 3_002_001);
        assert_eq!(RuleId::OutputPathInvalid as u32, 3_002_002);
        assert_eq!(RuleId::OutputWriteFailed as u32, 3_002_003);
        assert_eq!(RuleId::ReportWriteFailed as u32, 3_003_001);
        assert_eq!(RuleId::SupportBundleWriteFailed as u32, 3_003_002);
        assert_eq!(RuleId::PodmanSourceInvalid as u32, 4_000_001);
        assert_eq!(RuleId::PodmanInputUnmodelled as u32, 4_000_002);
        assert_eq!(RuleId::PodmanPromotionRequired as u32, 4_000_003);
        assert_eq!(RuleId::PodmanIdentityConflict as u32, 4_000_004);
        assert_eq!(RuleId::PodmanSecretGrantIncomplete as u32, 4_000_005);
        assert_eq!(RuleId::PodmanTargetInvalid as u32, 4_000_006);
        assert_eq!(RuleId::PodmanOutputUnsupported as u32, 4_000_007);
        assert_eq!(RuleId::PodmanPlanningFailed as u32, 4_000_008);
        assert_eq!(RuleId::QuadletTargetInvalid as u32, 5_000_001);
        assert_eq!(RuleId::QuadletOpenEndedTarget as u32, 5_000_002);
        assert_eq!(RuleId::QuadletOutputUnsupported as u32, 5_000_003);
        assert_eq!(RuleId::QuadletValueInvalid as u32, 5_000_004);
        assert_eq!(RuleId::QuadletGenerationFailed as u32, 5_000_005);
        assert_eq!(RuleId::QuadletCapabilityUnavailable as u32, 5_000_006);
        assert_eq!(RuleId::QuadletGroupingApproximation as u32, 5_000_007);
        assert_eq!(RuleId::QuadletDependencyUnsupported as u32, 5_000_008);
        assert_eq!(RuleId::QuadletRestartApproximation as u32, 5_000_009);
        assert_eq!(RuleId::QuadletEnvironmentFileApproximation as u32, 5_000_010);
        assert_eq!(RuleId::QuadletGroupingInvalid as u32, 5_000_011);
        assert_eq!(RuleId::QuadletDependencyInvalid as u32, 5_000_012);
        assert_eq!(RuleId::QuadletCapabilityDeprecated as u32, 5_000_013);
        assert_eq!(RuleId::QuadletUnresolvedVariable as u32, 5_000_014);
        assert_eq!(RuleId::QuadletSourceInvalid as u32, 5_001_001);
        assert_eq!(RuleId::QuadletModelInvalid as u32, 5_001_002);
        assert_eq!(RuleId::QuadletInputUnsupported as u32, 5_001_003);
        assert_eq!(RuleId::QuadletInputApproximation as u32, 5_001_004);
        assert_eq!(RuleId::QuadletNativeSyntax as u32, 5_001_101);
        assert_eq!(RuleId::QuadletNativeModel as u32, 5_001_102);
        assert_eq!(RuleId::QuadletNativeDocumentSet as u32, 5_001_103);
        assert_eq!(RuleId::QuadletNativeFailure as u32, 5_001_104);
        assert_eq!(RuleId::PodmanStateReconstructed as u32, 4_000_009);
    }

    #[test]
    fn historical_catalogue_relative_order_remains_stable() {
        // New rules may be inserted into the code-sorted catalogue. Preserve every
        // historical rule and its relative order while allowing those additions.
        let expected = [
            RuleId::ComposeModelInvalid,
            RuleId::ComposeProfileRequired,
            RuleId::ComposeProfileMismatch,
            RuleId::ComposeIntentUnsupported,
            RuleId::ComposeValueInvalid,
            RuleId::ComposeTargetInvalid,
            RuleId::ComposeOutputUnsupported,
            RuleId::ComposeGenerationFailed,
            RuleId::ComposeCompatibilityConstraint,
            RuleId::ComposeUnsetVariable,
            RuleId::ComposeRequiredVariable,
            RuleId::ComposeInterpolationInvalid,
            RuleId::ComposeInterpolationNestingLimit,
            RuleId::ComposeUnresolvedVariable,
            RuleId::ComposeNativeError,
            RuleId::ComposeNativeWarning,
            RuleId::ComposeNativeNote,
            RuleId::OrchestrationFailed,
            RuleId::InputReadFailed,
            RuleId::ComposeProjectRootInvalid,
            RuleId::PodmanTargetSelectionInvalid,
            RuleId::InterpolationInputInvalid,
            RuleId::ComposeLoadFailed,
            RuleId::ConversionFailed,
            RuleId::QuadletParseFailed,
            RuleId::OutputDirectoryNotEmpty,
            RuleId::OutputPathInvalid,
            RuleId::OutputWriteFailed,
            RuleId::ReportWriteFailed,
            RuleId::SupportBundleWriteFailed,
            RuleId::PodmanSourceInvalid,
            RuleId::PodmanInputUnmodelled,
            RuleId::PodmanPromotionRequired,
            RuleId::PodmanIdentityConflict,
            RuleId::PodmanSecretGrantIncomplete,
            RuleId::PodmanTargetInvalid,
            RuleId::PodmanOutputUnsupported,
            RuleId::PodmanPlanningFailed,
            RuleId::PodmanStateReconstructed,
            RuleId::QuadletTargetInvalid,
            RuleId::QuadletOpenEndedTarget,
            RuleId::QuadletOutputUnsupported,
            RuleId::QuadletValueInvalid,
            RuleId::QuadletGenerationFailed,
            RuleId::QuadletCapabilityUnavailable,
            RuleId::QuadletGroupingApproximation,
            RuleId::QuadletDependencyUnsupported,
            RuleId::QuadletRestartApproximation,
            RuleId::QuadletEnvironmentFileApproximation,
            RuleId::QuadletGroupingInvalid,
            RuleId::QuadletDependencyInvalid,
            RuleId::QuadletCapabilityDeprecated,
            RuleId::QuadletUnresolvedVariable,
            RuleId::QuadletSourceInvalid,
            RuleId::QuadletModelInvalid,
            RuleId::QuadletInputUnsupported,
            RuleId::QuadletInputApproximation,
            RuleId::QuadletNativeSyntax,
            RuleId::QuadletNativeModel,
            RuleId::QuadletNativeDocumentSet,
            RuleId::QuadletNativeFailure,
        ];
        let historical: Vec<_> = RULES
            .iter()
            .map(|rule| rule.id())
            .filter(|id| expected.contains(id))
            .collect();
        assert_eq!(historical, expected);
    }

    #[test]
    fn compose_output_help_distinguishes_privacy_refusal_from_authorized_omission() {
        let help = RuleId::ComposeOutputUnsupported.definition().help();
        assert!(help.contains("Protected service network aliases block every loss policy"));
        assert!(help.contains("keep private alias configuration outside the generated document"));
        assert!(help.contains("Otherwise, use --loss-policy partial only when a candidate exists"));
        assert!(help.contains("omitted intent is acceptable"));
    }

    #[test]
    fn docker_catalogue_separates_source_and_target_rules_and_preserves_existing_ids() {
        let docker: Vec<_> = RULES.iter().filter(|rule| rule.code().starts_with("BFD")).collect();
        assert_eq!(docker.len(), 7);
        for (id, code, name, severity, discriminant) in [
            (
                RuleId::DockerSourceInvalid,
                "BFD0001",
                "docker-source-invalid",
                crate::Severity::Error,
                2_000_001,
            ),
            (
                RuleId::DockerInputUnsupported,
                "BFD0002",
                "docker-input-unsupported",
                crate::Severity::Warning,
                2_000_002,
            ),
            (
                RuleId::DockerStateReconstructed,
                "BFD0003",
                "docker-state-reconstructed",
                crate::Severity::Note,
                2_000_003,
            ),
            (
                RuleId::DockerInputApproximation,
                "BFD0004",
                "docker-input-approximation",
                crate::Severity::Warning,
                2_000_004,
            ),
        ] {
            let rule = id.definition();
            assert_eq!(id as u32, discriminant);
            assert_eq!(
                (rule.code(), rule.name(), rule.default_severity()),
                (code, name, severity)
            );
            assert_eq!(rule.owner(), "Docker source adapter");
            assert_eq!(find_rule(code), Some(rule));
            assert_eq!(find_rule(name), Some(rule));
            assert_eq!(
                rule.diagnostic_code().map(|value| value.as_str().to_owned()),
                Ok(code.to_owned())
            );
        }
        for (id, code, discriminant) in [
            (RuleId::DockerTargetInvalid, "BFD0005", 2_000_005),
            (RuleId::DockerOutputUnsupported, "BFD0006", 2_000_006),
            (RuleId::DockerPlanningFailed, "BFD0007", 2_000_007),
        ] {
            assert_eq!(id.definition().code(), code);
            assert_eq!(id.definition().owner(), "Docker target adapter");
            assert_eq!(id as u32, discriminant);
            assert_eq!(find_rule(code), Some(id.definition()));
        }
        assert_eq!(RuleId::OrchestrationFailed as u32, 3_001_000);
        assert_eq!(RuleId::PodmanStateReconstructed as u32, 4_000_009);
    }

    #[test]
    fn catalogue_codes_names_and_typed_indices_are_unique_and_valid() -> Result<(), String> {
        let mut codes = BTreeSet::new();
        let mut names = BTreeSet::new();
        let mut ids = BTreeSet::new();
        for rule in RULES {
            rule.diagnostic_code().map_err(|error| error.to_string())?;
            assert!(ids.insert(rule.id()), "duplicate typed id {:?}", rule.id());
            assert_eq!(rule.id().definition(), rule);
            assert!(codes.insert(rule.code()), "duplicate code {}", rule.code());
            assert!(names.insert(rule.name()), "duplicate name {}", rule.name());
            assert!(!rule.description().is_empty());
            assert!(!rule.help().is_empty());
            assert_ne!(
                rule.owner(),
                "BoxFerry engine",
                "uncatalogued namespace for {}",
                rule.code()
            );
        }
        assert!(RULES.windows(2).all(|pair| pair[0].code() < pair[1].code()));
        assert_eq!(RuleId::OutputWriteFailed.definition().code(), "BFO2003");
        assert_eq!(RuleId::PodmanPromotionRequired.definition().code(), "BFP0003");
        assert_eq!(
            find_rule("podman-output-unsupported").map(|rule| rule.owner()),
            Some("Podman adapter")
        );
        assert_eq!(find_rule("quadlet-restart-policy-approximation"), find_rule("BFQ0009"));
        Ok(())
    }
}
