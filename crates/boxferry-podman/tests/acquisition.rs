//! Replaceable Podman acquisition boundary tests without a runtime.

use std::{cell::RefCell, rc::Rc};

use boxferry_engine::PlatformVersion;
use boxferry_model::Identifier;
use boxferry_podman::{
    PodmanAcquisitionError, PodmanCommandExecutor, PodmanCommandOutput, PodmanExpansionPolicy, PodmanInspectCommand,
    PodmanInspector, PodmanResourceKind, PodmanResourceSelection,
};

type RecordedCalls = Rc<RefCell<Vec<(PodmanResourceKind, Vec<String>)>>>;

#[derive(Clone, Default)]
struct RecordingExecutor {
    calls: RecordedCalls,
}

impl PodmanCommandExecutor for RecordingExecutor {
    fn execute(&self, command: &PodmanInspectCommand) -> Result<PodmanCommandOutput, PodmanAcquisitionError> {
        self.calls.borrow_mut().push((
            command.kind(),
            command
                .selectors()
                .iter()
                .map(|selector| selector.expose().to_owned())
                .collect(),
        ));
        Ok(PodmanCommandOutput::new("[]"))
    }
}

#[derive(Clone, Default)]
struct RelationshipExecutor {
    calls: RecordedCalls,
}

impl PodmanCommandExecutor for RelationshipExecutor {
    fn execute(&self, command: &PodmanInspectCommand) -> Result<PodmanCommandOutput, PodmanAcquisitionError> {
        self.calls.borrow_mut().push((
            command.kind(),
            command
                .selectors()
                .iter()
                .map(|selector| selector.expose().to_owned())
                .collect(),
        ));
        Ok(PodmanCommandOutput::new(match command.kind() {
            PodmanResourceKind::Pod => POD_RESPONSE,
            PodmanResourceKind::Container => CONTAINER_RESPONSE,
            PodmanResourceKind::Image => IMAGE_RESPONSE,
            PodmanResourceKind::Network => NETWORK_RESPONSE,
            PodmanResourceKind::Volume => VOLUME_RESPONSE,
            _ => "[]",
        }))
    }
}

const POD_RESPONSE: &str = r#"[
  {
    "Id": "pod-id",
    "Name": "app-pod",
    "Containers": [
      {"Id": "container-web-id"},
      {"Id": "container-worker-id"},
      {"Id": "container-worker-id"}
    ]
  }
]"#;

const CONTAINER_RESPONSE: &str = r#"[
  {
    "Id": "container-web-id",
    "Image": "image-web-id",
    "Name": "web",
    "Dependencies": ["namespace-or-generic-dependency-id"],
    "Mounts": [
      {"Type": "volume", "Name": "data", "Destination": "/data"},
      {"Type": "bind", "Source": "/srv/web", "Destination": "/srv/web"}
    ],
    "NetworkSettings": {"Networks": {"frontend": {"Aliases": ["web"]}}}
  },
  {
    "Id": "container-web-id",
    "Image": "image-web-id",
    "Name": "web",
    "Mounts": [],
    "NetworkSettings": {"Networks": {"frontend": {}}}
  },
  {
    "Id": "container-worker-id",
    "Image": "image-worker-id",
    "Name": "worker",
    "Mounts": [{"Type": "volume", "Name": "cache", "Destination": "/cache"}],
    "NetworkSettings": {"Networks": {"backend": {}}}
  }
]"#;

const IMAGE_RESPONSE: &str = r#"[
  {"Id": "image-web-id"},
  {"Id": "image-web-id"},
  {"Id": "image-worker-id"}
]"#;

const NETWORK_RESPONSE: &str = r#"[
  {"name": "frontend"},
  {"name": "frontend"},
  {"name": "backend"}
]"#;

const VOLUME_RESPONSE: &str = r#"[
  {"Name": "data"},
  {"Name": "data"},
  {"Name": "cache"}
]"#;

#[test]
fn explicit_selection_runs_only_fixed_read_only_families_in_stable_order() -> Result<(), String> {
    let mut selection = PodmanResourceSelection::new();
    selection.add_container("web").map_err(|error| error.to_string())?;
    selection
        .add_image("example.invalid/web:1")
        .map_err(|error| error.to_string())?;
    selection.add_network("frontend").map_err(|error| error.to_string())?;
    selection.add_volume("data").map_err(|error| error.to_string())?;
    selection.add_pod("app-pod").map_err(|error| error.to_string())?;
    let executor = RecordingExecutor::default();
    let inspector =
        PodmanInspector::new(executor.clone(), "/usr/bin/podman", version()).map_err(|error| error.to_string())?;

    let source = inspector
        .inspect(id("example")?, &selection)
        .map_err(|error| error.to_string())?;
    let calls = executor.calls.borrow();

    assert_eq!(
        calls.iter().map(|(kind, _)| *kind).collect::<Vec<_>>(),
        [
            PodmanResourceKind::Container,
            PodmanResourceKind::Image,
            PodmanResourceKind::Network,
            PodmanResourceKind::Volume,
            PodmanResourceKind::Pod,
        ]
    );
    assert_eq!(calls[0].1, ["web"]);
    assert_eq!(source.version(), version());
    assert_eq!(source.documents().containers(), "[]");

    let rendered = format!("{selection:?} {source:?}");
    for selector in ["web", "example.invalid/web:1", "frontend", "data", "app-pod"] {
        assert!(!rendered.contains(selector));
    }
    assert!(rendered.contains("[REDACTED]"));
    Ok(())
}

#[test]
fn empty_families_become_empty_arrays_without_spawning_commands() -> Result<(), String> {
    let executor = RecordingExecutor::default();
    let inspector = PodmanInspector::new(executor.clone(), "podman", version()).map_err(|error| error.to_string())?;
    let source = inspector
        .inspect(id("example")?, &PodmanResourceSelection::new())
        .map_err(|error| error.to_string())?;

    assert!(executor.calls.borrow().is_empty());
    for document in [
        source.documents().containers(),
        source.documents().images(),
        source.documents().networks(),
        source.documents().volumes(),
        source.documents().pods(),
    ] {
        assert_eq!(document, "[]");
    }
    Ok(())
}

#[test]
fn explicit_policy_expands_pod_members_and_finite_container_resources() -> Result<(), String> {
    let mut selection = PodmanResourceSelection::new();
    selection.add_container("web").map_err(|error| error.to_string())?;
    selection
        .add_image("web-image-tag")
        .map_err(|error| error.to_string())?;
    selection
        .add_network("frontend-alias")
        .map_err(|error| error.to_string())?;
    selection.add_volume("data-alias").map_err(|error| error.to_string())?;
    selection.add_pod("app-pod").map_err(|error| error.to_string())?;
    let executor = RelationshipExecutor::default();
    let inspector = PodmanInspector::new(executor.clone(), "podman", version()).map_err(|error| error.to_string())?;

    let source = inspector
        .inspect_with_policy(
            id("example")?,
            &selection,
            PodmanExpansionPolicy::PodMembersAndContainerResources,
        )
        .map_err(|error| error.to_string())?;
    let calls = executor.calls.borrow();

    assert_eq!(
        calls.as_slice(),
        [
            (PodmanResourceKind::Pod, vec!["app-pod".to_owned()]),
            (
                PodmanResourceKind::Container,
                vec![
                    "web".to_owned(),
                    "container-web-id".to_owned(),
                    "container-worker-id".to_owned(),
                ],
            ),
            (
                PodmanResourceKind::Image,
                vec![
                    "web-image-tag".to_owned(),
                    "image-web-id".to_owned(),
                    "image-worker-id".to_owned(),
                ],
            ),
            (
                PodmanResourceKind::Network,
                vec!["frontend-alias".to_owned(), "frontend".to_owned(), "backend".to_owned(),],
            ),
            (
                PodmanResourceKind::Volume,
                vec!["data-alias".to_owned(), "data".to_owned(), "cache".to_owned()],
            ),
        ]
    );
    assert_eq!(json_array_len(source.documents().pods())?, 1);
    assert_eq!(json_array_len(source.documents().containers())?, 2);
    assert_eq!(json_array_len(source.documents().images())?, 2);
    assert_eq!(json_array_len(source.documents().networks())?, 2);
    assert_eq!(json_array_len(source.documents().volumes())?, 2);
    Ok(())
}

#[test]
fn container_resource_policy_does_not_follow_pod_members_or_bind_mounts() -> Result<(), String> {
    let mut selection = PodmanResourceSelection::new();
    selection.add_container("web").map_err(|error| error.to_string())?;
    selection.add_pod("app-pod").map_err(|error| error.to_string())?;
    let executor = RelationshipExecutor::default();
    let inspector = PodmanInspector::new(executor.clone(), "podman", version()).map_err(|error| error.to_string())?;

    inspector
        .inspect_with_policy(id("example")?, &selection, PodmanExpansionPolicy::ContainerResources)
        .map_err(|error| error.to_string())?;
    let calls = executor.calls.borrow();

    assert_eq!(calls[1].1, ["web"]);
    assert_eq!(calls[4].1, ["data", "cache"]);
    assert!(
        calls
            .iter()
            .all(|(_, selectors)| !selectors.iter().any(|value| value == "/srv/web"))
    );
    assert!(calls.iter().all(|(_, selectors)| {
        !selectors
            .iter()
            .any(|value| value == "namespace-or-generic-dependency-id")
    }));
    Ok(())
}

#[test]
fn malformed_relationship_data_fails_without_disclosing_payload() -> Result<(), String> {
    #[derive(Clone)]
    struct InvalidExecutor;

    impl PodmanCommandExecutor for InvalidExecutor {
        fn execute(&self, command: &PodmanInspectCommand) -> Result<PodmanCommandOutput, PodmanAcquisitionError> {
            assert_eq!(command.kind(), PodmanResourceKind::Container);
            Ok(PodmanCommandOutput::new("private invalid json"))
        }
    }

    let mut selection = PodmanResourceSelection::new();
    selection.add_container("web").map_err(|error| error.to_string())?;
    let inspector = PodmanInspector::new(InvalidExecutor, "podman", version()).map_err(|error| error.to_string())?;
    let result = inspector.inspect_with_policy(id("example")?, &selection, PodmanExpansionPolicy::ContainerResources);
    let Err(error) = result else {
        return Err("invalid relationship data must fail".to_owned());
    };

    assert!(matches!(
        error,
        PodmanAcquisitionError::InvalidInspectOutput {
            kind: PodmanResourceKind::Container,
            ..
        }
    ));
    assert!(!format!("{error:?} {error}").contains("private invalid json"));
    Ok(())
}

#[test]
fn selectors_and_process_errors_are_sensitive_by_default() -> Result<(), String> {
    let mut selection = PodmanResourceSelection::new();
    selection
        .add_container("customer-secret-container")
        .map_err(|error| error.to_string())?;
    assert!(!format!("{selection:?}").contains("customer-secret-container"));

    let output = PodmanCommandOutput::new("[{\"Config\":{\"Env\":[\"TOKEN=secret\"]}}]");
    assert!(!format!("{output:?}").contains("TOKEN=secret"));

    let error =
        PodmanAcquisitionError::command_failed(PodmanResourceKind::Container, Some(125), "machine-private-error");
    assert!(!format!("{error:?} {error}").contains("machine-private-error"));
    Ok(())
}

#[test]
fn invalid_selectors_and_executable_fail_before_execution() {
    let mut selection = PodmanResourceSelection::new();
    assert!(selection.add_container("").is_err());
    assert!(selection.add_image("bad\0image").is_err());
    assert!(PodmanInspector::new(RecordingExecutor::default(), "", version()).is_err());
}

fn version() -> PlatformVersion {
    PlatformVersion::new(5, 4, 0)
}

fn id(value: &str) -> Result<Identifier, String> {
    Identifier::new(value).map_err(|error| error.to_string())
}

fn json_array_len(document: &str) -> Result<usize, String> {
    serde_json::from_str::<Vec<serde_json::Value>>(document)
        .map(|values| values.len())
        .map_err(|error| error.to_string())
}
