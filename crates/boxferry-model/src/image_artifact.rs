//! Provenance-aware image acquisition and build resources.
//!
//! These types describe application artifacts, not any one source or target format. A service's
//! runtime image remains distinct from the acquisition/build resources that produce or obtain it.

use crate::{Identifier, ProtectedString, Provenance, ResourceLimit, Sourced};

/// The generic syntax family that supplied a build declaration or per-key value collection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BuildSyntax {
    /// One short scalar declaration.
    Scalar,
    /// A structured declaration object.
    Structured,
    /// Mapping syntax for named values.
    Mapping,
    /// Sequence syntax for ordered values.
    Sequence,
    /// Repeated native assignment syntax.
    Repeated,
}

/// One typed collection belonging to one explicitly present image-artifact key.
///
/// The enclosing setting establishes that its key was present. Consequently, `values: []` is an
/// explicit empty/reset value, while omission is represented by the absence of that setting.
/// Repeated settings and values retain their declaration order and duplicate occurrences.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildSettingValues<T> {
    syntax: BuildSyntax,
    values: Vec<Sourced<T>>,
}

impl<T> BuildSettingValues<T> {
    /// Creates values for one explicitly present key.
    #[must_use]
    pub const fn new(syntax: BuildSyntax, values: Vec<Sourced<T>>) -> Self {
        Self { syntax, values }
    }

    /// Returns the source syntax family.
    #[must_use]
    pub const fn syntax(&self) -> BuildSyntax {
        self.syntax
    }

    /// Returns values in source order, including duplicates.
    #[must_use]
    pub fn values(&self) -> &[Sourced<T>] {
        &self.values
    }
}

/// One source-preserving assignment used by image artifact settings.
///
/// Empty and key-only assignments are retained for adapters to diagnose rather than discarded by
/// the neutral model. `ProtectedString` prevents interpolated names and values from leaking into
/// debug output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageArtifactAssignment {
    name: ProtectedString,
    value: Option<ProtectedString>,
}

impl ImageArtifactAssignment {
    /// Creates a key-only or key/value assignment without applying native naming rules.
    #[must_use]
    pub const fn new(name: ProtectedString, value: Option<ProtectedString>) -> Self {
        Self { name, value }
    }

    /// Returns the preserved assignment name.
    #[must_use]
    pub const fn name(&self) -> &ProtectedString {
        &self.name
    }

    /// Returns the explicit value, if the source supplied one.
    #[must_use]
    pub const fn value(&self) -> Option<&ProtectedString> {
        self.value.as_ref()
    }
}

/// One named additional context in a structured source build declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuildContext {
    name: ProtectedString,
    value: ProtectedString,
}

impl BuildContext {
    /// Creates a named additional build context without applying native rules.
    #[must_use]
    pub const fn new(name: ProtectedString, value: ProtectedString) -> Self {
        Self { name, value }
    }

    /// Returns the declared context name.
    #[must_use]
    pub const fn name(&self) -> &ProtectedString {
        &self.name
    }

    /// Returns the raw-preserving context value.
    #[must_use]
    pub const fn value(&self) -> &ProtectedString {
        &self.value
    }
}

/// A source build-secret declaration whose options remain distinct from target build-secret text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceBuildSecret {
    source: ProtectedString,
    target: Option<ProtectedString>,
    uid: Option<ProtectedString>,
    gid: Option<ProtectedString>,
    mode: Option<ProtectedString>,
}

impl SourceBuildSecret {
    /// Creates a source build-secret declaration without applying native validation.
    #[must_use]
    pub const fn new(source: ProtectedString) -> Self {
        Self {
            source,
            target: None,
            uid: None,
            gid: None,
            mode: None,
        }
    }

    /// Returns the preserved source name.
    #[must_use]
    pub const fn source(&self) -> &ProtectedString {
        &self.source
    }

    /// Sets the optional target name.
    pub fn set_target(&mut self, target: ProtectedString) {
        self.target = Some(target);
    }

    /// Returns the optional target name.
    #[must_use]
    pub const fn target(&self) -> Option<&ProtectedString> {
        self.target.as_ref()
    }

    /// Sets the optional UID spelling.
    pub fn set_uid(&mut self, uid: ProtectedString) {
        self.uid = Some(uid);
    }

    /// Returns the optional UID spelling.
    #[must_use]
    pub const fn uid(&self) -> Option<&ProtectedString> {
        self.uid.as_ref()
    }

    /// Sets the optional GID spelling.
    pub fn set_gid(&mut self, gid: ProtectedString) {
        self.gid = Some(gid);
    }

    /// Returns the optional GID spelling.
    #[must_use]
    pub const fn gid(&self) -> Option<&ProtectedString> {
        self.gid.as_ref()
    }

    /// Sets the optional mode spelling.
    pub fn set_mode(&mut self, mode: ProtectedString) {
        self.mode = Some(mode);
    }

    /// Returns the optional mode spelling.
    #[must_use]
    pub const fn mode(&self) -> Option<&ProtectedString> {
        self.mode.as_ref()
    }
}

/// A source build attestation value.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BuildAttestation {
    /// Boolean form.
    Boolean(bool),
    /// Raw parameterized form.
    Value(ProtectedString),
}

/// One field from a structured source build declaration.
///
/// These concepts deliberately describe source declaration intent. They do not claim equivalence
/// to similarly named acquisition/build settings.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SourceBuildSetting {
    /// Additional named contexts.
    AdditionalContexts(BuildSettingValues<BuildContext>),
    /// Build arguments.
    Arguments(BuildSettingValues<ImageArtifactAssignment>),
    /// Cache import locations.
    CacheFrom(BuildSettingValues<ProtectedString>),
    /// Cache export locations.
    CacheTo(BuildSettingValues<ProtectedString>),
    /// Build context.
    Context(ProtectedString),
    /// Build recipe path.
    RecipeFile(ProtectedString),
    /// Inline build recipe.
    InlineRecipe(ProtectedString),
    /// Build entitlements.
    Entitlements(BuildSettingValues<ProtectedString>),
    /// Extra host mappings.
    ExtraHosts(BuildSettingValues<ImageArtifactAssignment>),
    /// Isolation selection.
    Isolation(ProtectedString),
    /// Build metadata labels.
    Labels(BuildSettingValues<ImageArtifactAssignment>),
    /// Build network selection.
    Network(ProtectedString),
    /// Explicit no-cache choice.
    NoCache(bool),
    /// No-cache filters.
    NoCacheFilters(BuildSettingValues<ProtectedString>),
    /// Build platforms.
    Platforms(BuildSettingValues<ProtectedString>),
    /// Explicit privileged choice.
    Privileged(bool),
    /// Provenance attestation.
    Provenance(BuildAttestation),
    /// Explicit source-side pull choice.
    Pull(bool),
    /// SBOM attestation.
    Sbom(BuildAttestation),
    /// Source build secrets.
    Secrets(BuildSettingValues<SourceBuildSecret>),
    /// Shared-memory size spelling.
    ShmSize(ProtectedString),
    /// SSH declarations.
    Ssh(BuildSettingValues<ProtectedString>),
    /// Image tags.
    Tags(BuildSettingValues<ProtectedString>),
    /// Recipe target.
    Target(ProtectedString),
    /// Resource limits.
    Ulimits(BuildSettingValues<ResourceLimit>),
}

/// One source declaration for an image build.
///
/// A scalar context is not silently expanded into a structured declaration. Structured declarations
/// retain field order, duplicate fields, and explicitly present empty per-key collections.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum BuildSourceDeclaration {
    /// Short scalar context syntax.
    Scalar(ProtectedString),
    /// Structured declaration syntax.
    Structured(Vec<Sourced<SourceBuildSetting>>),
}

impl BuildSourceDeclaration {
    /// Returns the preserved source syntax family.
    #[must_use]
    pub const fn syntax(&self) -> BuildSyntax {
        match self {
            Self::Scalar(_) => BuildSyntax::Scalar,
            Self::Structured(_) => BuildSyntax::Structured,
        }
    }

    /// Returns structured settings in source order when this is a structured declaration.
    #[must_use]
    pub fn structured_settings(&self) -> Option<&[Sourced<SourceBuildSetting>]> {
        match self {
            Self::Scalar(_) => None,
            Self::Structured(settings) => Some(settings),
        }
    }
}

/// One image-acquisition setting.
///
/// The variants cover the full currently supported acquisition surface while keeping every value
/// target-independent: adapters decide which native key can encode a given setting.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ImageAcquisitionSetting {
    /// Artifact image source.
    Image(ProtectedString),
    /// Produced/acquired image tags.
    ImageTags(BuildSettingValues<ProtectedString>),
    /// Service-manager unit name.
    ServiceName(ProtectedString),
    /// Fetch all tags choice.
    AllTags(bool),
    /// Architecture selection.
    Architecture(ProtectedString),
    /// Authentication file.
    AuthFile(ProtectedString),
    /// Certificate directory.
    CertificateDirectory(ProtectedString),
    /// Containers configuration modules.
    ContainersConfigModules(BuildSettingValues<ProtectedString>),
    /// Credential text.
    Credentials(ProtectedString),
    /// Image decryption key.
    DecryptionKey(ProtectedString),
    /// Global runtime arguments.
    GlobalArguments(BuildSettingValues<ProtectedString>),
    /// Operating-system selection.
    OperatingSystem(ProtectedString),
}

/// An image-acquisition resource.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageAcquisition {
    name: Identifier,
    settings: Option<Vec<Sourced<ImageAcquisitionSetting>>>,
    settings_origins: Vec<Provenance>,
}

impl ImageAcquisition {
    /// Creates an empty acquisition resource.
    #[must_use]
    pub const fn new(name: Identifier) -> Self {
        Self {
            name,
            settings: None,
            settings_origins: Vec::new(),
        }
    }

    /// Returns the neutral resource name.
    #[must_use]
    pub const fn name(&self) -> &Identifier {
        &self.name
    }

    /// Sets acquisition settings, retaining explicit emptiness separately from omission.
    pub fn set_settings(&mut self, settings: Vec<Sourced<ImageAcquisitionSetting>>) {
        self.settings = Some(settings);
        self.settings_origins.clear();
    }

    /// Sets acquisition settings and their collection-level provenance.
    pub fn set_settings_with_origins(
        &mut self,
        settings: Vec<Sourced<ImageAcquisitionSetting>>,
        origins: Vec<Provenance>,
    ) {
        self.settings = Some(settings);
        self.settings_origins = origins;
    }

    /// Returns settings in source order, if explicitly present.
    #[must_use]
    pub fn settings(&self) -> Option<&[Sourced<ImageAcquisitionSetting>]> {
        self.settings.as_deref()
    }

    /// Returns acquisition collection provenance.
    #[must_use]
    pub fn settings_origins(&self) -> &[Provenance] {
        &self.settings_origins
    }
}

/// One image-build setting.
///
/// Fields that look similar to [`SourceBuildSetting`] remain separate: only an adapter with
/// target evidence may establish an exact or non-exact mapping between them.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ImageBuildSetting {
    /// Produced image tags.
    ImageTags(BuildSettingValues<ProtectedString>),
    /// Build network selection.
    Network(ProtectedString),
    /// Build labels.
    Labels(BuildSettingValues<ImageArtifactAssignment>),
    /// Build recipe file.
    RecipeFile(ProtectedString),
    /// Working-directory behavior spelling.
    SetWorkingDirectory(ProtectedString),
    /// Build recipe target.
    Target(ProtectedString),
    /// Native build arguments.
    BuildArguments(BuildSettingValues<ImageArtifactAssignment>),
    /// Native build-secret declarations.
    Secrets(BuildSettingValues<ProtectedString>),
    /// Architecture selection.
    Architecture(ProtectedString),
    /// Architecture variant selection.
    Variant(ProtectedString),
    /// Native pull-policy spelling.
    PullPolicy(ProtectedString),
    /// Retry-count spelling.
    Retry(ProtectedString),
    /// Retry-delay spelling.
    RetryDelay(ProtectedString),
    /// TLS verification choice.
    TlsVerify(bool),
    /// Remove intermediate artifacts choice.
    ForceRemove(bool),
    /// Authentication file.
    AuthFile(ProtectedString),
    /// Ignore-file spelling.
    IgnoreFile(ProtectedString),
    /// Service-manager unit name.
    ServiceName(ProtectedString),
    /// Supplemental group additions.
    GroupAdd(BuildSettingValues<ProtectedString>),
    /// DNS servers.
    DnsServers(BuildSettingValues<ProtectedString>),
    /// DNS resolver options.
    DnsOptions(BuildSettingValues<ProtectedString>),
    /// DNS search domains.
    DnsSearchDomains(BuildSettingValues<ProtectedString>),
    /// Build annotations.
    Annotations(BuildSettingValues<ImageArtifactAssignment>),
    /// Build environment assignments.
    Environment(BuildSettingValues<ImageArtifactAssignment>),
    /// Containers configuration modules.
    ContainersConfigModules(BuildSettingValues<ProtectedString>),
    /// Global runtime arguments.
    GlobalArguments(BuildSettingValues<ProtectedString>),
    /// Build volume declarations.
    Volumes(BuildSettingValues<ProtectedString>),
    /// Runtime-specific build arguments.
    RuntimeArguments(BuildSettingValues<ProtectedString>),
}

/// An image-build resource with independently retained source declaration and artifact settings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImageBuild {
    name: Identifier,
    source_declaration: Option<Sourced<BuildSourceDeclaration>>,
    settings: Option<Vec<Sourced<ImageBuildSetting>>>,
    settings_origins: Vec<Provenance>,
}

impl ImageBuild {
    /// Creates an empty image-build resource.
    #[must_use]
    pub const fn new(name: Identifier) -> Self {
        Self {
            name,
            source_declaration: None,
            settings: None,
            settings_origins: Vec::new(),
        }
    }

    /// Returns the neutral build-resource name.
    #[must_use]
    pub const fn name(&self) -> &Identifier {
        &self.name
    }

    /// Sets the source declaration without conflating scalar and structured syntax.
    pub fn set_source_declaration(&mut self, declaration: Sourced<BuildSourceDeclaration>) {
        self.source_declaration = Some(declaration);
    }

    /// Returns the optional source declaration.
    #[must_use]
    pub const fn source_declaration(&self) -> Option<&Sourced<BuildSourceDeclaration>> {
        self.source_declaration.as_ref()
    }

    /// Sets artifact settings, retaining explicit emptiness separately from omission.
    pub fn set_settings(&mut self, settings: Vec<Sourced<ImageBuildSetting>>) {
        self.settings = Some(settings);
        self.settings_origins.clear();
    }

    /// Sets artifact settings and their collection-level provenance.
    pub fn set_settings_with_origins(&mut self, settings: Vec<Sourced<ImageBuildSetting>>, origins: Vec<Provenance>) {
        self.settings = Some(settings);
        self.settings_origins = origins;
    }

    /// Returns artifact settings in source order, if explicitly present.
    #[must_use]
    pub fn settings(&self) -> Option<&[Sourced<ImageBuildSetting>]> {
        self.settings.as_deref()
    }

    /// Returns artifact-setting collection provenance.
    #[must_use]
    pub fn settings_origins(&self) -> &[Provenance] {
        &self.settings_origins
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BuildSettingValues, BuildSourceDeclaration, BuildSyntax, ImageAcquisition, ImageAcquisitionSetting,
        ImageArtifactAssignment, ImageBuild, ImageBuildSetting, SourceBuildSetting,
    };
    use crate::{Identifier, ProtectedString, Provenance, SourceId, Sourced};

    fn origin() -> Result<Provenance, String> {
        SourceId::new("build.yaml")
            .map(Provenance::source)
            .map_err(|error| error.to_string())
    }

    #[test]
    fn native_text_settings_preserve_working_directory_and_pull_policy() -> Result<(), String> {
        let origin = origin()?;
        let mut build = ImageBuild::new(Identifier::new("web-build").map_err(|error| error.to_string())?);
        build.set_settings(vec![
            Sourced::from_source(
                ImageBuildSetting::SetWorkingDirectory(ProtectedString::plain("unit")),
                origin.clone(),
            ),
            Sourced::from_source(ImageBuildSetting::PullPolicy(ProtectedString::plain("newer")), origin),
        ]);
        let settings = build.settings().ok_or("explicit settings")?;
        assert!(matches!(
            settings[0].value(),
            ImageBuildSetting::SetWorkingDirectory(value) if value.expose() == "unit"
        ));
        assert!(matches!(
            settings[1].value(),
            ImageBuildSetting::PullPolicy(value) if value.expose() == "newer"
        ));
        Ok(())
    }

    #[test]
    fn per_key_empty_reset_is_distinct_from_omission_and_retains_order_and_provenance() -> Result<(), String> {
        let origin = origin()?;
        let mut build = ImageBuild::new(Identifier::new("web-build").map_err(|error| error.to_string())?);
        assert_eq!(build.source_declaration(), None);
        build.set_source_declaration(Sourced::from_source(
            BuildSourceDeclaration::Structured(vec![
                Sourced::from_source(
                    SourceBuildSetting::Tags(BuildSettingValues::new(BuildSyntax::Sequence, Vec::new())),
                    origin.clone(),
                ),
                Sourced::from_source(
                    SourceBuildSetting::Arguments(BuildSettingValues::new(BuildSyntax::Mapping, Vec::new())),
                    origin.clone(),
                ),
            ]),
            origin.clone(),
        ));
        let declaration = build.source_declaration().ok_or("structured declaration")?;
        let settings = declaration.value().structured_settings().ok_or("structured settings")?;
        assert_eq!(settings.len(), 2);
        assert!(matches!(
            settings[0].value(),
            SourceBuildSetting::Tags(values) if values.values().is_empty() && values.syntax() == BuildSyntax::Sequence
        ));
        assert!(matches!(
            settings[1].value(),
            SourceBuildSetting::Arguments(values) if values.values().is_empty() && values.syntax() == BuildSyntax::Mapping
        ));
        assert_eq!(settings[0].origins(), std::slice::from_ref(&origin));
        Ok(())
    }

    #[test]
    fn scalar_and_structured_source_declarations_remain_distinct() -> Result<(), String> {
        let origin = origin()?;
        let mut build = ImageBuild::new(Identifier::new("web-build").map_err(|error| error.to_string())?);
        build.set_source_declaration(Sourced::from_source(
            BuildSourceDeclaration::Scalar(ProtectedString::plain("./web")),
            origin,
        ));
        assert_eq!(
            build
                .source_declaration()
                .map(|declaration| declaration.value().syntax()),
            Some(BuildSyntax::Scalar)
        );
        assert_eq!(
            build
                .source_declaration()
                .and_then(|declaration| declaration.value().structured_settings()),
            None
        );
        Ok(())
    }

    #[test]
    fn repeated_acquisition_values_and_sensitive_settings_retain_duplicates_and_redact() -> Result<(), String> {
        let origin = origin()?;
        let mut acquisition = ImageAcquisition::new(Identifier::new("base-image").map_err(|error| error.to_string())?);
        acquisition.set_settings_with_origins(
            vec![
                Sourced::from_source(
                    ImageAcquisitionSetting::ImageTags(BuildSettingValues::new(
                        BuildSyntax::Repeated,
                        vec![
                            Sourced::from_source(ProtectedString::plain("web:one"), origin.clone()),
                            Sourced::from_source(ProtectedString::plain("web:one"), origin.clone()),
                        ],
                    )),
                    origin.clone(),
                ),
                Sourced::from_source(
                    ImageAcquisitionSetting::Credentials(ProtectedString::sensitive("operator:secret")),
                    origin.clone(),
                ),
            ],
            vec![origin.clone()],
        );
        let settings = acquisition.settings().ok_or("explicit settings")?;
        assert!(matches!(
            settings[0].value(),
            ImageAcquisitionSetting::ImageTags(values) if values.values().len() == 2 && values.values()[0] == values.values()[1]
        ));
        assert_eq!(acquisition.settings_origins(), std::slice::from_ref(&origin));
        let assignment = ImageArtifactAssignment::new(
            ProtectedString::plain("TOKEN"),
            Some(ProtectedString::sensitive("build-secret")),
        );
        let target_secrets = ImageBuildSetting::Secrets(BuildSettingValues::new(
            BuildSyntax::Repeated,
            vec![Sourced::from_source(
                ProtectedString::sensitive("target-build-secret"),
                origin.clone(),
            )],
        ));
        let source_ssh = SourceBuildSetting::Ssh(BuildSettingValues::new(
            BuildSyntax::Sequence,
            vec![Sourced::from_source(
                ProtectedString::sensitive("ssh-agent-secret"),
                origin.clone(),
            )],
        ));
        let auth = ImageAcquisitionSetting::AuthFile(ProtectedString::sensitive("auth-file-secret"));
        let decryption = ImageAcquisitionSetting::DecryptionKey(ProtectedString::sensitive("decryption-secret"));
        let debug = format!("{acquisition:?} {assignment:?} {target_secrets:?} {source_ssh:?} {auth:?} {decryption:?}");
        for secret in [
            "operator:secret",
            "build-secret",
            "target-build-secret",
            "ssh-agent-secret",
            "auth-file-secret",
            "decryption-secret",
        ] {
            assert!(!debug.contains(secret));
        }
        assert!(debug.contains("[REDACTED]"));
        Ok(())
    }

    #[test]
    fn malformed_or_empty_assignments_are_retained_for_adapter_diagnostics() {
        let assignment = ImageArtifactAssignment::new(ProtectedString::plain(""), Some(ProtectedString::plain("")));
        assert_eq!(assignment.name().expose(), "");
        assert_eq!(assignment.value().map(ProtectedString::expose), Some(""));
    }
}
