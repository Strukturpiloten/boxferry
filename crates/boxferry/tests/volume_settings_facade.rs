//! Public facade coverage for typed Quadlet volume settings and image artifacts.

#![cfg(feature = "quadlet")]

use std::error::Error;

use boxferry::quadlet::quadlet_lens::source::SourceId as QuadletSourceId;
use boxferry::{
    Application, ArtifactDependency, ArtifactDependencyNode, BuildSettingValues, BuildSyntax, ConversionKind,
    ExportAdapter, Identifier, ImageBuild, ImageBuildSetting, ImportAdapter, LossPolicy, ModelError, PlatformVersion,
    ProtectedString, QuadletDocumentInput, QuadletExporter, QuadletImporter, QuadletSource, ResourceOwnership, Sourced,
    TargetProfile, Volume, VolumeImageSource, convert,
};

fn parse_source(
    application_name: Identifier,
    inputs: impl IntoIterator<Item = QuadletDocumentInput>,
) -> Result<QuadletSource, Box<dyn Error>> {
    Ok(QuadletSource::parse(application_name, inputs)?.into_source())
}

#[test]
fn facade_preserves_all_typed_volume_settings_at_their_podman_floors() -> Result<(), Box<dyn Error>> {
    let source = all_volume_settings_source()?;
    let adapter = QuadletImporter::new()?;
    let import_result = adapter.import(&source);
    let application = import_result.application().ok_or("application expected")?;
    let volume = application.volumes()[0].value();
    assert_eq!(volume.name().as_str(), "data");
    assert_eq!(
        volume.runtime_name().map(|value| value.value().expose()),
        Some("runtime-data")
    );
    assert_eq!(
        volume.service_name().map(|value| value.value().expose()),
        Some("volume-service")
    );
    assert_eq!(volume.driver().map(|value| value.value().expose()), Some("local"));
    assert_eq!(volume.device().map(|value| value.value().expose()), Some("/srv/data"));
    assert_eq!(volume.volume_type().map(|value| value.value().expose()), Some("none"));
    assert_eq!(volume.options().map(|value| value.value().expose()), Some("bind"));
    assert_eq!(volume.copy().map(Sourced::value), Some(&true));
    assert_eq!(volume.user().map(|value| value.value().expose()), Some("alice"));
    assert_eq!(volume.group().map(|value| value.value().expose()), Some("staff"));
    assert_eq!(volume.uid().map(|value| value.value().expose()), Some("1000"));
    assert_eq!(volume.gid().map(|value| value.value().expose()), Some("1001"));
    assert!(matches!(
        image_source(application, "image-data").map(Sourced::value),
        Some(VolumeImageSource::Literal(_))
    ));
    let application_debug = format!("{application:?}");
    assert!(!application_debug.contains("private-base"));
    assert!(!application_debug.contains("private-argument"));

    let at_floor = convert(
        &adapter,
        &source,
        &QuadletExporter::new()?,
        &podman_target(5, 4, 0, 5, 4, 2)?,
        LossPolicy::AllowPartial,
    )?;
    let floor_output = volume_text(&at_floor)?;
    for key in [
        "VolumeName=runtime-data",
        "ServiceName=volume-service",
        "User=alice",
        "Group=staff",
    ] {
        assert!(floor_output.contains(key), "missing {key}: {floor_output}");
    }
    assert!(!floor_output.contains("UID="));
    assert!(!floor_output.contains("GID="));
    for subject in ["volumes.data.uid", "volumes.data.gid"] {
        assert!(
            at_floor
                .outcomes()
                .iter()
                .any(|outcome| { outcome.subject() == subject && outcome.kind() == ConversionKind::Unsupported })
        );
    }

    let at_six = convert(
        &adapter,
        &source,
        &QuadletExporter::new()?,
        &podman_target(6, 0, 0, 6, 0, 2)?,
        LossPolicy::AllowPartial,
    )?;
    let six_output = volume_text(&at_six)?;
    for key in [
        "VolumeName=runtime-data",
        "Driver=local",
        "Device=/srv/data",
        "Type=none",
        "Options=bind",
        "Copy=true",
        "Label=org.example.owner=private-label",
        "ContainersConfModule=private-module.conf",
        "GlobalArgs=--log-level=debug",
        "PodmanArgs=--private-argument",
        "User=alice",
        "Group=staff",
        "UID=1000",
        "GID=1001",
        "ServiceName=volume-service",
    ] {
        assert!(six_output.contains(key), "missing {key}: {six_output}");
    }
    assert!(
        at_six
            .output()
            .and_then(|files| files.file("image-data.volume"))
            .is_some_and(|file| file.text().contains("Image=example.invalid/private-base:1"))
    );
    assert!(!format!("{at_six:?}").contains("private-argument"));

    let beyond_ceiling = convert(
        &adapter,
        &source,
        &QuadletExporter::new()?,
        &podman_target(6, 1, 1, 6, 1, 1)?,
        LossPolicy::AllowPartial,
    )?;
    assert!(beyond_ceiling.is_blocked());
    assert!(beyond_ceiling.output().is_none());
    Ok(())
}

#[test]
fn facade_rejects_unrepresentable_volume_resets_duplicates_and_dependencies() -> Result<(), Box<dyn Error>> {
    let invalid = parse_source(
        Identifier::new("invalid")?,
        [QuadletDocumentInput::new(
            "data.volume",
            QuadletSourceId::new(2),
            concat!(
                "[Volume]\nDriver=local\nDriver=other\nType=none\nLabel=owner=one\nLabel=\n",
                "ContainersConfModule=base.conf\nContainersConfModule=\nGlobalArgs=--debug\nGlobalArgs=\n",
                "PodmanArgs=--private\nPodmanArgs=\n",
            ),
        )],
    )?;
    let invalid_import = QuadletImporter::new()?.import(&invalid);
    assert!(
        invalid_import
            .outcomes()
            .iter()
            .any(|outcome| { outcome.subject() == "volumes.data.Driver" && outcome.kind() == ConversionKind::Invalid })
    );
    for subject in [
        "volumes.data.type",
        "volumes.data.labels",
        "volumes.data.containers_conf_modules",
        "volumes.data.global_args",
        "volumes.data.podman_args",
    ] {
        assert!(
            invalid_import
                .outcomes()
                .iter()
                .any(|outcome| outcome.subject() == subject)
        );
    }

    let options_only = parse_source(
        Identifier::new("options")?,
        [QuadletDocumentInput::new(
            "data.volume",
            QuadletSourceId::new(3),
            "[Volume]\nOptions=bind\n",
        )],
    )?;
    let adapter = QuadletImporter::new()?;
    let before_six = convert(
        &adapter,
        &options_only,
        &QuadletExporter::new()?,
        &podman_target(5, 4, 0, 5, 4, 2)?,
        LossPolicy::AllowPartial,
    )?;
    assert!(before_six.outcomes().iter().any(|outcome| {
        outcome.subject() == "volumes.data.options" && outcome.kind() == ConversionKind::Unsupported
    }));
    let at_six = convert(
        &adapter,
        &options_only,
        &QuadletExporter::new()?,
        &podman_target(6, 0, 0, 6, 0, 2)?,
        LossPolicy::ExactOnly,
    )?;
    assert_eq!(volume_text(&at_six)?, "[Volume]\nOptions=bind\n");
    Ok(())
}

#[test]
fn facade_distinguishes_literal_and_typed_volume_images_and_fails_closed() -> Result<(), Box<dyn Error>> {
    let source = volume_artifact_source()?;
    let adapter = QuadletImporter::new()?;
    let import_result = adapter.import(&source);
    let application = import_result
        .application()
        .ok_or_else(|| format!("application expected: {:#?}", import_result.diagnostics()))?;
    assert!(matches!(
        image_source(application, "acquired").map(Sourced::value),
        Some(VolumeImageSource::ImageAcquisition(_))
    ));
    assert!(matches!(
        image_source(application, "literal").map(Sourced::value),
        Some(VolumeImageSource::Literal(_))
    ));

    let output = convert(
        &adapter,
        &source,
        &QuadletExporter::new()?,
        &podman_target(6, 0, 0, 6, 0, 2)?,
        LossPolicy::ExactOnly,
    )?;
    assert!(!output.is_blocked(), "{:#?}", output.diagnostics());
    assert!(
        output
            .output()
            .and_then(|files| files.file("acquired.volume"))
            .is_some_and(|file| file.text().contains("Image=base.image"))
    );

    let typed = typed_build_volume_application()?;
    let typed_output = QuadletExporter::new()?
        .plan(&typed, &podman_target(6, 0, 0, 6, 0, 2)?)?
        .authorize(LossPolicy::ExactOnly);
    assert!(
        typed_output
            .output()
            .and_then(|files| files.file("builder.build"))
            .is_some()
    );
    assert!(
        typed_output
            .output()
            .and_then(|files| files.file("built.volume"))
            .is_some_and(|file| file.text().contains("Image=builder.build"))
    );
    assert!(
        typed_output
            .output()
            .and_then(|files| files.file("builder.build"))
            .is_some_and(|file| file.text().contains("Volume=cache.volume:/cache"))
    );

    let conflict = copy_image_conflict_application()?;
    let conflict_plan = QuadletExporter::new()?.plan(&conflict, &podman_target(6, 0, 0, 6, 0, 2)?)?;
    assert!(conflict_plan.outcomes().iter().any(|outcome| {
        outcome.subject() == "volumes.copy-image.image" && outcome.kind() == ConversionKind::Unsupported
    }));

    let missing = parse_source(
        Identifier::new("missing")?,
        [QuadletDocumentInput::new(
            "data.volume",
            QuadletSourceId::new(12),
            "[Volume]\nImage=absent.image\n",
        )],
    )?;
    let missing_result = convert(
        &QuadletImporter::new()?,
        &missing,
        &QuadletExporter::new()?,
        &podman_target(6, 0, 0, 6, 0, 2)?,
        LossPolicy::AllowPartial,
    );
    assert!(matches!(missing_result, Err(boxferry::ConversionError::Import(_))));

    let mut application = Application::new(Identifier::new("cycle")?);
    let mut volume = Volume::new(Identifier::new("cache")?, ResourceOwnership::Application);
    volume.set_image_source(Sourced::generated(VolumeImageSource::ImageBuild(Identifier::new(
        "cache-build",
    )?)))?;
    application.add_volume(Sourced::generated(volume))?;
    application.add_image_build(Sourced::generated(ImageBuild::new(Identifier::new("cache-build")?)))?;
    let volume_node = ArtifactDependencyNode::Volume(Identifier::new("cache")?);
    let build_node = ArtifactDependencyNode::ImageBuild(Identifier::new("cache-build")?);
    let cycle = [
        Sourced::generated(ArtifactDependency::new(
            Sourced::generated(volume_node.clone()),
            Sourced::generated(build_node.clone()),
        )),
        Sourced::generated(ArtifactDependency::new(
            Sourced::generated(build_node),
            Sourced::generated(volume_node),
        )),
    ];
    assert!(matches!(
        application.validate_image_artifact_dependencies(&cycle),
        Err(ModelError::ImageArtifactDependencyCycle { .. })
    ));
    Ok(())
}

fn all_volume_settings_source() -> Result<QuadletSource, Box<dyn Error>> {
    parse_source(
        Identifier::new("volumes")?,
        [
            QuadletDocumentInput::new(
                "data.volume",
                QuadletSourceId::new(1),
                concat!(
                    "[Volume]\nVolumeName=runtime-data\nDriver=local\nDevice=/srv/data\nType=none\n",
                    "Options=bind\nCopy=true\nLabel=org.example.owner=private-label\n",
                    "ContainersConfModule=private-module.conf\nGlobalArgs=--log-level=debug\n",
                    "PodmanArgs=--private-argument\nUser=alice\nGroup=staff\nUID=1000\nGID=1001\n",
                    "ServiceName=volume-service\n",
                ),
            ),
            QuadletDocumentInput::new(
                "image-data.volume",
                QuadletSourceId::new(2),
                "[Volume]\nImage=example.invalid/private-base:1\n",
            ),
        ],
    )
}

fn volume_artifact_source() -> Result<QuadletSource, Box<dyn Error>> {
    parse_source(
        Identifier::new("artifacts")?,
        [
            QuadletDocumentInput::new(
                "base.image",
                QuadletSourceId::new(4),
                "[Image]\nImage=example.invalid/base:1\n",
            ),
            QuadletDocumentInput::new(
                "literal.volume",
                QuadletSourceId::new(6),
                "[Volume]\nImage=example.invalid/literal:1\n",
            ),
            QuadletDocumentInput::new(
                "acquired.volume",
                QuadletSourceId::new(7),
                "[Volume]\nImage=base.image\n",
            ),
        ],
    )
}

fn typed_build_volume_application() -> Result<Application, Box<dyn Error>> {
    let mut application = Application::new(Identifier::new("typed")?);
    let mut build_resource = ImageBuild::new(Identifier::new("builder")?);
    build_resource.set_settings(vec![
        Sourced::generated(ImageBuildSetting::ImageTags(BuildSettingValues::new(
            BuildSyntax::Repeated,
            vec![Sourced::generated(ProtectedString::plain("example.invalid/builder:1"))],
        ))),
        Sourced::generated(ImageBuildSetting::SetWorkingDirectory(ProtectedString::plain("."))),
        Sourced::generated(ImageBuildSetting::Volumes(BuildSettingValues::new(
            BuildSyntax::Repeated,
            vec![Sourced::generated(ProtectedString::sensitive("cache.volume:/cache"))],
        ))),
    ]);
    application.add_image_build(Sourced::generated(build_resource))?;
    application.add_volume(Sourced::generated(Volume::new(
        Identifier::new("cache")?,
        ResourceOwnership::Application,
    )))?;
    let mut image_backed_volume = Volume::new(Identifier::new("built")?, ResourceOwnership::Application);
    image_backed_volume.set_image_source(Sourced::generated(VolumeImageSource::ImageBuild(Identifier::new(
        "builder",
    )?)))?;
    application.add_volume(Sourced::generated(image_backed_volume))?;
    Ok(application)
}

fn copy_image_conflict_application() -> Result<Application, Box<dyn Error>> {
    let mut application = Application::new(Identifier::new("conflict")?);
    let mut volume = Volume::new(Identifier::new("copy-image")?, ResourceOwnership::Application);
    volume.set_copy(Sourced::generated(true));
    volume.set_image_source(Sourced::generated(VolumeImageSource::Literal(
        ProtectedString::sensitive("example.invalid/private-copy:1"),
    )))?;
    application.add_volume(Sourced::generated(volume))?;
    Ok(application)
}

fn podman_target(
    minimum_major: u64,
    minimum_minor: u64,
    minimum_patch: u64,
    maximum_major: u64,
    maximum_minor: u64,
    maximum_patch: u64,
) -> Result<TargetProfile, Box<dyn Error>> {
    Ok(TargetProfile::new(
        "podman",
        PlatformVersion::new(minimum_major, minimum_minor, minimum_patch),
        Some(PlatformVersion::new(maximum_major, maximum_minor, maximum_patch)),
    )?)
}

fn volume_text(result: &boxferry::ConversionResult<boxferry::QuadletOutput>) -> Result<&str, Box<dyn Error>> {
    result
        .output()
        .and_then(|output| output.file("data.volume"))
        .map(boxferry::QuadletFile::text)
        .ok_or_else(|| format!("data volume output expected: {:#?}", result.diagnostics()).into())
}

fn image_source<'a>(application: &'a Application, name: &str) -> Option<&'a Sourced<VolumeImageSource>> {
    application
        .volumes()
        .iter()
        .find(|volume| volume.value().name().as_str() == name)
        .and_then(|volume| volume.value().image_source())
}
