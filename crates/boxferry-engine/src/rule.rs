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
    DockerDataInvalid = 2_000_001,
    DockerConfigurationUnsupported = 2_000_002,
    DockerVersionUnsupported = 2_000_003,
    DockerRelationshipMissing = 2_000_004,
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
    PodmanDataInvalid = 4_000_001,
    PodmanConfigurationUnsupported = 4_000_002,
    PodmanVersionUnsupported = 4_000_003,
    PodmanRelationshipMissing = 4_000_004,
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
    RuntimeReconstructionUncertain = 6_000_001,
    RuntimeOverrideInferred = 6_000_002,
    RuntimeComparisonIncomplete = 6_000_003,
    RuntimeOwnershipUncertain = 6_000_004,
    RuntimePodRelationshipIncomplete = 6_000_005,
    RuntimeImageMissing = 6_000_006,
    RuntimeModelInvalid = 6_000_007,
    RuntimeGroupConflict = 6_000_008,
    RuntimeLifecycleResolved = 6_000_009,
    RuntimeManagedMetadata = 6_000_010,
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
            Some(b"BFD") => "Docker adapter",
            Some(b"BFO") => "BoxFerry orchestration",
            Some(b"BFP") => "Podman adapter",
            Some(b"BFQ") => "Quadlet adapter",
            Some(b"BFR") => "runtime reconstruction",
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
        "Review the named subject; use --loss-policy partial only when the omitted intent is acceptable."
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
        "Use --interpolate and provide missing values with --env-file FILE or --env NAME=VALUE; partial authorization applies only when the affected intent can be omitted."
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
        DockerDataInvalid,
        "BFD0001",
        "docker-inspection-invalid",
        Error,
        "Docker inspection data is malformed or contradictory.",
        "Inspect the selected Docker response and retry with a supported, valid payload."
    ),
    rule!(
        DockerConfigurationUnsupported,
        "BFD0002",
        "docker-configuration-unsupported",
        Warning,
        "Docker inspection contains meaningful configuration outside the current model.",
        "Review the named field; use partial output only when omission is acceptable."
    ),
    rule!(
        DockerVersionUnsupported,
        "BFD0003",
        "docker-api-version-unsupported",
        Error,
        "The Docker Engine API version is outside the reviewed range.",
        "Select a Docker Engine API version covered by BoxFerry."
    ),
    rule!(
        DockerRelationshipMissing,
        "BFD0004",
        "docker-relationship-missing",
        Warning,
        "Docker inspection references a resource absent from the supplied snapshot.",
        "Inspect and supply the referenced resource or accept partial reconstruction."
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
        PodmanDataInvalid,
        "BFP0001",
        "podman-inspection-invalid",
        Error,
        "Podman inspection data is malformed or contradictory.",
        "Inspect the selected Podman response and retry with a supported, valid payload."
    ),
    rule!(
        PodmanConfigurationUnsupported,
        "BFP0002",
        "podman-configuration-unsupported",
        Warning,
        "Podman inspection contains meaningful configuration outside the current model.",
        "Review the named field; use partial output only when omission is acceptable."
    ),
    rule!(
        PodmanVersionUnsupported,
        "BFP0003",
        "podman-version-unsupported",
        Error,
        "The Podman version is outside the reviewed range.",
        "Select a Podman version covered by BoxFerry."
    ),
    rule!(
        PodmanRelationshipMissing,
        "BFP0004",
        "podman-relationship-missing",
        Warning,
        "Podman inspection references a resource absent from the supplied snapshot.",
        "Inspect and supply the referenced resource or accept partial reconstruction."
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
        "Resolve source variables before conversion; for Compose input, use --interpolate and provide missing values with --env-file FILE or --env NAME=VALUE."
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
    rule!(
        RuntimeReconstructionUncertain,
        "BFR0001",
        "runtime-reconstruction-uncertain",
        Warning,
        "Runtime inspection cannot prove the original authored definition.",
        "Review the reconstructed definition and every field-level decision before deployment."
    ),
    rule!(
        RuntimeOverrideInferred,
        "BFR0002",
        "runtime-override-inferred",
        Warning,
        "A runtime override was inferred by comparison with image defaults.",
        "Review the inferred override before generating a reusable definition."
    ),
    rule!(
        RuntimeComparisonIncomplete,
        "BFR0003",
        "runtime-comparison-incomplete",
        Warning,
        "Runtime-to-image comparison evidence is incomplete.",
        "Supply the missing image evidence or review the affected field manually."
    ),
    rule!(
        RuntimeOwnershipUncertain,
        "BFR0004",
        "runtime-ownership-uncertain",
        Warning,
        "Runtime resource lifecycle ownership is uncertain.",
        "Choose application-owned or external lifecycle explicitly."
    ),
    rule!(
        RuntimePodRelationshipIncomplete,
        "BFR0005",
        "runtime-pod-relationship-incomplete",
        Warning,
        "Runtime pod relationship evidence is incomplete or non-portable.",
        "Review and explicitly resolve the service-group lifecycle."
    ),
    rule!(
        RuntimeImageMissing,
        "BFR0006",
        "runtime-image-missing",
        Warning,
        "A runtime container has no reconstructable image reference.",
        "Supply a reviewed image reference or return to an authored source definition."
    ),
    rule!(
        RuntimeModelInvalid,
        "BFR0007",
        "runtime-model-invalid",
        Error,
        "Runtime observations cannot form a valid neutral application model.",
        "Correct the reported observation or resource identity."
    ),
    rule!(
        RuntimeGroupConflict,
        "BFR0008",
        "runtime-group-conflict",
        Error,
        "Runtime group relationship evidence is contradictory.",
        "Correct or narrow the selected runtime snapshot."
    ),
    rule!(
        RuntimeLifecycleResolved,
        "BFR0009",
        "runtime-lifecycle-resolved",
        Warning,
        "Caller policy resolved lifecycle intent that inspection could not prove.",
        "Review the explicit lifecycle override before deployment."
    ),
    rule!(
        RuntimeManagedMetadata,
        "BFR0010",
        "runtime-managed-metadata",
        Warning,
        "Runtime-managed metadata is unsafe to re-author as application metadata.",
        "Keep the metadata as evidence and do not copy it into generated definitions."
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
            Self::DockerDataInvalid => &RULES[17],
            Self::DockerConfigurationUnsupported => &RULES[18],
            Self::DockerVersionUnsupported => &RULES[19],
            Self::DockerRelationshipMissing => &RULES[20],
            Self::OrchestrationFailed => &RULES[21],
            Self::InputReadFailed => &RULES[22],
            Self::ComposeProjectRootInvalid => &RULES[23],
            Self::PodmanTargetSelectionInvalid => &RULES[24],
            Self::InterpolationInputInvalid => &RULES[25],
            Self::ComposeLoadFailed => &RULES[26],
            Self::ConversionFailed => &RULES[27],
            Self::QuadletParseFailed => &RULES[28],
            Self::OutputDirectoryNotEmpty => &RULES[29],
            Self::OutputPathInvalid => &RULES[30],
            Self::OutputWriteFailed => &RULES[31],
            Self::ReportWriteFailed => &RULES[32],
            Self::SupportBundleWriteFailed => &RULES[33],
            Self::PodmanDataInvalid => &RULES[34],
            Self::PodmanConfigurationUnsupported => &RULES[35],
            Self::PodmanVersionUnsupported => &RULES[36],
            Self::PodmanRelationshipMissing => &RULES[37],
            Self::QuadletTargetInvalid => &RULES[38],
            Self::QuadletOpenEndedTarget => &RULES[39],
            Self::QuadletOutputUnsupported => &RULES[40],
            Self::QuadletValueInvalid => &RULES[41],
            Self::QuadletGenerationFailed => &RULES[42],
            Self::QuadletCapabilityUnavailable => &RULES[43],
            Self::QuadletGroupingApproximation => &RULES[44],
            Self::QuadletDependencyUnsupported => &RULES[45],
            Self::QuadletRestartApproximation => &RULES[46],
            Self::QuadletEnvironmentFileApproximation => &RULES[47],
            Self::QuadletGroupingInvalid => &RULES[48],
            Self::QuadletDependencyInvalid => &RULES[49],
            Self::QuadletCapabilityDeprecated => &RULES[50],
            Self::QuadletUnresolvedVariable => &RULES[51],
            Self::QuadletSourceInvalid => &RULES[52],
            Self::QuadletModelInvalid => &RULES[53],
            Self::QuadletInputUnsupported => &RULES[54],
            Self::QuadletInputApproximation => &RULES[55],
            Self::QuadletNativeSyntax => &RULES[56],
            Self::QuadletNativeModel => &RULES[57],
            Self::QuadletNativeDocumentSet => &RULES[58],
            Self::QuadletNativeFailure => &RULES[59],
            Self::RuntimeReconstructionUncertain => &RULES[60],
            Self::RuntimeOverrideInferred => &RULES[61],
            Self::RuntimeComparisonIncomplete => &RULES[62],
            Self::RuntimeOwnershipUncertain => &RULES[63],
            Self::RuntimePodRelationshipIncomplete => &RULES[64],
            Self::RuntimeImageMissing => &RULES[65],
            Self::RuntimeModelInvalid => &RULES[66],
            Self::RuntimeGroupConflict => &RULES[67],
            Self::RuntimeLifecycleResolved => &RULES[68],
            Self::RuntimeManagedMetadata => &RULES[69],
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
        assert_eq!(find_rule("quadlet-restart-policy-approximation"), find_rule("BFQ0009"));
        Ok(())
    }
}
