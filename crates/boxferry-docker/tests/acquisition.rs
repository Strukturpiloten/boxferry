//! Replaceable Docker acquisition boundary tests without a daemon.

use std::{cell::RefCell, rc::Rc};

#[cfg(unix)]
use std::{
    fs,
    io::Write,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

#[cfg(unix)]
use boxferry_docker::ProcessDockerCommandExecutor;
use boxferry_docker::{
    DockerAcquisitionError, DockerApiVersion, DockerCommandExecutor, DockerCommandOutput, DockerExpansionPolicy,
    DockerInspectCommand, DockerInspector, DockerResourceKind, DockerResourceSelection,
};
use boxferry_model::Identifier;

#[derive(Clone, Debug, Eq, PartialEq)]
struct RecordedCall {
    kind: DockerResourceKind,
    endpoint: String,
    api_version: DockerApiVersion,
    selectors: Vec<String>,
}

type RecordedCalls = Rc<RefCell<Vec<RecordedCall>>>;

#[derive(Clone, Default)]
struct RecordingExecutor {
    calls: RecordedCalls,
}

impl DockerCommandExecutor for RecordingExecutor {
    fn execute(&self, command: &DockerInspectCommand) -> Result<DockerCommandOutput, DockerAcquisitionError> {
        self.calls.borrow_mut().push(RecordedCall {
            kind: command.kind(),
            endpoint: command.endpoint().expose().to_owned(),
            api_version: command.api_version(),
            selectors: command
                .selectors()
                .iter()
                .map(|selector| selector.expose().to_owned())
                .collect(),
        });
        Ok(DockerCommandOutput::new("[]"))
    }
}

#[derive(Clone, Default)]
struct RelationshipExecutor {
    calls: RecordedCalls,
}

impl DockerCommandExecutor for RelationshipExecutor {
    fn execute(&self, command: &DockerInspectCommand) -> Result<DockerCommandOutput, DockerAcquisitionError> {
        self.calls.borrow_mut().push(RecordedCall {
            kind: command.kind(),
            endpoint: command.endpoint().expose().to_owned(),
            api_version: command.api_version(),
            selectors: command
                .selectors()
                .iter()
                .map(|selector| selector.expose().to_owned())
                .collect(),
        });
        Ok(DockerCommandOutput::new(match command.kind() {
            DockerResourceKind::Container => CONTAINER_RESPONSE,
            DockerResourceKind::Image => IMAGE_RESPONSE,
            DockerResourceKind::Network => NETWORK_RESPONSE,
            DockerResourceKind::Volume => VOLUME_RESPONSE,
            _ => "[]",
        }))
    }
}

const CONTAINER_RESPONSE: &str = r#"[
  {
    "Id": "container-web-id",
    "Image": "image-web-id",
    "Name": "/web",
    "Mounts": [
      {"Type": "volume", "Name": "data", "Destination": "/data"},
      {"Type": "bind", "Source": "/srv/web", "Destination": "/srv/web"}
    ],
    "NetworkSettings": {"Networks": {"frontend": {"Aliases": ["web"]}}}
  },
  {
    "Id": "container-web-id",
    "Image": "image-web-id",
    "Name": "/web",
    "Mounts": [],
    "NetworkSettings": {"Networks": {"frontend": {}}}
  }
]"#;

const IMAGE_RESPONSE: &str = r#"[
  {"Id": "image-web-id"},
  {"Id": "image-web-id"}
]"#;

const NETWORK_RESPONSE: &str = r#"[
  {"Name": "frontend"},
  {"Name": "frontend"}
]"#;

const VOLUME_RESPONSE: &str = r#"[
  {"Name": "data"},
  {"Name": "data"}
]"#;

#[test]
fn explicit_selection_uses_fixed_families_endpoint_and_api_version() -> Result<(), String> {
    let mut selection = DockerResourceSelection::new();
    selection.add_container("web").map_err(|error| error.to_string())?;
    selection
        .add_image("example.invalid/web:1")
        .map_err(|error| error.to_string())?;
    selection.add_network("frontend").map_err(|error| error.to_string())?;
    selection.add_volume("data").map_err(|error| error.to_string())?;
    let executor = RecordingExecutor::default();
    let inspector = inspector(executor.clone())?;

    let source = inspector
        .inspect(id("example")?, &selection)
        .map_err(|error| error.to_string())?;
    let calls = executor.calls.borrow();

    assert_eq!(
        calls.iter().map(|call| call.kind).collect::<Vec<_>>(),
        [
            DockerResourceKind::Container,
            DockerResourceKind::Image,
            DockerResourceKind::Network,
            DockerResourceKind::Volume,
        ]
    );
    assert!(
        calls
            .iter()
            .all(|call| { call.endpoint == "unix:///run/user/1000/docker.sock" && call.api_version == api_version() })
    );
    assert_eq!(calls[0].selectors, ["web"]);
    assert_eq!(source.api_version(), api_version());

    let rendered = format!("{selection:?} {source:?} {inspector:?}");
    for protected in [
        "web",
        "example.invalid/web:1",
        "frontend",
        "data",
        "unix:///run/user/1000/docker.sock",
    ] {
        assert!(!rendered.contains(protected));
    }
    assert!(rendered.contains("[REDACTED]"));
    Ok(())
}

#[test]
fn empty_families_become_empty_arrays_without_executor_calls() -> Result<(), String> {
    let executor = RecordingExecutor::default();
    let inspector = inspector(executor.clone())?;
    let source = inspector
        .inspect(id("example")?, &DockerResourceSelection::new())
        .map_err(|error| error.to_string())?;

    assert!(executor.calls.borrow().is_empty());
    for document in [
        source.documents().containers(),
        source.documents().images(),
        source.documents().networks(),
        source.documents().volumes(),
    ] {
        assert_eq!(document, "[]");
    }
    Ok(())
}

#[test]
fn container_resource_policy_expands_finitely_and_deduplicates_results() -> Result<(), String> {
    let mut selection = DockerResourceSelection::new();
    selection.add_container("web").map_err(|error| error.to_string())?;
    selection.add_image("web-tag").map_err(|error| error.to_string())?;
    selection
        .add_network("frontend-alias")
        .map_err(|error| error.to_string())?;
    selection.add_volume("data-alias").map_err(|error| error.to_string())?;
    let executor = RelationshipExecutor::default();
    let inspector = inspector(executor.clone())?;

    let source = inspector
        .inspect_with_policy(id("example")?, &selection, DockerExpansionPolicy::ContainerResources)
        .map_err(|error| error.to_string())?;
    let calls = executor.calls.borrow();

    assert_eq!(calls[0].selectors, ["web"]);
    assert_eq!(calls[1].selectors, ["web-tag", "image-web-id"]);
    assert_eq!(calls[2].selectors, ["frontend-alias", "frontend"]);
    assert_eq!(calls[3].selectors, ["data-alias", "data"]);
    assert!(
        calls
            .iter()
            .all(|call| !call.selectors.iter().any(|value| value == "/srv/web"))
    );
    assert_eq!(json_array_len(source.documents().containers())?, 1);
    assert_eq!(json_array_len(source.documents().images())?, 1);
    assert_eq!(json_array_len(source.documents().networks())?, 1);
    assert_eq!(json_array_len(source.documents().volumes())?, 1);
    Ok(())
}

#[test]
fn explicit_policy_does_not_expand_relationships() -> Result<(), String> {
    let mut selection = DockerResourceSelection::new();
    selection.add_container("web").map_err(|error| error.to_string())?;
    let executor = RelationshipExecutor::default();
    let inspector = inspector(executor.clone())?;

    inspector
        .inspect_with_policy(id("example")?, &selection, DockerExpansionPolicy::ExplicitOnly)
        .map_err(|error| error.to_string())?;

    assert_eq!(executor.calls.borrow().len(), 1);
    Ok(())
}

#[test]
fn malformed_relationship_data_fails_without_disclosing_payload() -> Result<(), String> {
    #[derive(Clone)]
    struct InvalidExecutor;

    impl DockerCommandExecutor for InvalidExecutor {
        fn execute(&self, command: &DockerInspectCommand) -> Result<DockerCommandOutput, DockerAcquisitionError> {
            assert_eq!(command.kind(), DockerResourceKind::Container);
            Ok(DockerCommandOutput::new("private invalid json"))
        }
    }

    let mut selection = DockerResourceSelection::new();
    selection.add_container("web").map_err(|error| error.to_string())?;
    let inspector = inspector(InvalidExecutor)?;
    let result = inspector.inspect_with_policy(id("example")?, &selection, DockerExpansionPolicy::ContainerResources);
    let Err(error) = result else {
        return Err("invalid relationship data must fail".to_owned());
    };

    assert!(matches!(
        error,
        DockerAcquisitionError::InvalidInspectOutput {
            kind: DockerResourceKind::Container,
            ..
        }
    ));
    assert!(!format!("{error:?} {error}").contains("private invalid json"));
    Ok(())
}

#[test]
fn endpoint_selectors_output_and_process_errors_are_protected() -> Result<(), String> {
    let mut selection = DockerResourceSelection::new();
    selection
        .add_container("customer-secret-container")
        .map_err(|error| error.to_string())?;
    assert!(!format!("{selection:?}").contains("customer-secret-container"));

    let output = DockerCommandOutput::new("[{\"Config\":{\"Env\":[\"TOKEN=secret\"]}}]");
    assert!(!format!("{output:?}").contains("TOKEN=secret"));

    let error =
        DockerAcquisitionError::command_failed(DockerResourceKind::Container, Some(125), "machine-private-error");
    assert!(!format!("{error:?} {error}").contains("machine-private-error"));
    Ok(())
}

#[test]
fn invalid_selection_executable_and_endpoint_fail_before_execution() {
    let mut selection = DockerResourceSelection::new();
    assert!(selection.add_container("").is_err());
    assert!(selection.add_image("bad\0image").is_err());
    assert!(DockerInspector::new(RecordingExecutor::default(), "", "unix:///docker.sock", api_version()).is_err());
    assert!(DockerInspector::new(RecordingExecutor::default(), "docker", "", api_version()).is_err());
    assert!(DockerInspector::new(RecordingExecutor::default(), "docker", "bad\0endpoint", api_version()).is_err());
    assert!(
        DockerInspector::new(
            RecordingExecutor::default(),
            "docker",
            "unix:///docker.sock",
            DockerApiVersion::new(1, 39),
        )
        .is_err()
    );
    assert!(
        DockerInspector::new(
            RecordingExecutor::default(),
            "docker",
            "unix:///docker.sock",
            DockerApiVersion::new(1, 56),
        )
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn process_executor_forces_the_closed_command_and_removes_ambient_selection() -> Result<(), String> {
    let executable = InspectScript::new()?;
    let inspector = DockerInspector::new(
        ProcessDockerCommandExecutor::new(),
        executable.path(),
        "unix:///explicit/docker.sock",
        api_version(),
    )
    .map_err(|error| error.to_string())?;
    let mut selection = DockerResourceSelection::new();
    selection.add_container("web").map_err(|error| error.to_string())?;

    let source = inspector
        .inspect(id("example")?, &selection)
        .map_err(|error| error.to_string())?;

    assert_eq!(source.documents().containers(), "[]");
    Ok(())
}

#[cfg(unix)]
struct InspectScript(PathBuf);

#[cfg(unix)]
impl InspectScript {
    fn new() -> Result<Self, String> {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        for _ in 0..100 {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("boxferry-docker-executor-{}-{id}.sh", std::process::id()));
            let file = fs::OpenOptions::new().write(true).create_new(true).open(&path);
            let mut file = match file {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.to_string()),
            };
            let script = Self(path);
            file.write_all(
                concat!(
                    "#!/bin/sh\n",
                    "[ \"$DOCKER_API_VERSION\" = '1.40' ] || exit 40\n",
                    "[ -z \"${DOCKER_HOST+x}\" ] || exit 41\n",
                    "[ -z \"${DOCKER_CONTEXT+x}\" ] || exit 42\n",
                    "[ -z \"${DOCKER_CUSTOM_HEADERS+x}\" ] || exit 43\n",
                    "[ -z \"${DOCKER_TLS+x}\" ] || exit 44\n",
                    "[ -z \"${DOCKER_TLS_VERIFY+x}\" ] || exit 45\n",
                    "[ -z \"${DOCKER_CERT_PATH+x}\" ] || exit 46\n",
                    "[ \"$1\" = '--config' ] || exit 47\n",
                    "[ -d \"$2\" ] || exit 48\n",
                    "[ -z \"$(find \"$2\" -mindepth 1 -print -quit)\" ] || exit 49\n",
                    "[ \"$3\" = '--host' ] || exit 50\n",
                    "[ \"$4\" = 'unix:///explicit/docker.sock' ] || exit 51\n",
                    "[ \"$5\" = 'container' ] || exit 52\n",
                    "[ \"$6\" = 'inspect' ] || exit 53\n",
                    "[ \"$7\" = '--' ] || exit 54\n",
                    "[ \"$8\" = 'web' ] || exit 55\n",
                    "printf '[]'\n",
                )
                .as_bytes(),
            )
            .map_err(|error| error.to_string())?;
            let mut permissions = file.metadata().map_err(|error| error.to_string())?.permissions();
            permissions.set_mode(0o700);
            file.set_permissions(permissions).map_err(|error| error.to_string())?;
            return Ok(script);
        }
        Err("could not allocate a unique Docker executor test script".to_owned())
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

#[cfg(unix)]
impl Drop for InspectScript {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn inspector<E>(executor: E) -> Result<DockerInspector<E>, String> {
    DockerInspector::new(
        executor,
        "/usr/bin/docker",
        "unix:///run/user/1000/docker.sock",
        api_version(),
    )
    .map_err(|error| error.to_string())
}

fn api_version() -> DockerApiVersion {
    DockerApiVersion::new(1, 40)
}

fn id(value: &str) -> Result<Identifier, String> {
    Identifier::new(value).map_err(|error| error.to_string())
}

fn json_array_len(document: &str) -> Result<usize, String> {
    serde_json::from_str::<Vec<serde_json::Value>>(document)
        .map(|values| values.len())
        .map_err(|error| error.to_string())
}
