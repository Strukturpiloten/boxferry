//! Bounded, read-only Docker CLI journeys against a synthetic local Unix daemon.

#![cfg(all(
    feature = "cli",
    feature = "compose",
    feature = "docker",
    feature = "podman",
    feature = "quadlet",
    unix
))]

use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    io::{self, Read, Write},
    os::unix::{
        fs::{DirBuilderExt, MetadataExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

static NEXT_CASE: AtomicU64 = AtomicU64::new(0);
const FIRST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const SECOND: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const PRIVATE_NATIVE_CANARY: &str = "native-response-canary-6722";

/// Only Unix socket fixtures need a short root; ordinary output fixtures keep TMPDIR.
struct SocketTestDirectory(PathBuf);

impl SocketTestDirectory {
    fn new() -> io::Result<Self> {
        for _ in 0..64 {
            let path = Path::new("/tmp").join(format!(
                "bf-docker-cli-{}-{}",
                std::process::id(),
                NEXT_CASE.fetch_add(1, Ordering::Relaxed),
            ));
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "private socket fixture directory cannot be allocated",
        ))
    }
}

impl Drop for SocketTestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn socket_fixture_root_is_short_private_and_removed_with_its_contents() -> io::Result<()> {
    let path;
    {
        let directory = SocketTestDirectory::new()?;
        path = directory.0.clone();
        assert_eq!(path.parent(), Some(Path::new("/tmp")));
        assert_eq!(fs::symlink_metadata(&path)?.mode() & 0o777, 0o700);
        fs::write(path.join("private-fixture-file"), b"synthetic fixture")?;
        let socket = path.join("private-backlogged.sock");
        assert!(socket.as_os_str().len() < 100);
        let listener = UnixListener::bind(socket)?;
        drop(listener);
    }
    assert!(!path.exists());
    Ok(())
}

/// Independently authored failure replies, never a native capability fixture.
#[derive(Clone)]
enum FakeDockerReply {
    Http { status: u16, body: Vec<u8> },
    Raw(Vec<u8>),
}

impl FakeDockerReply {
    fn bounded(&self) -> bool {
        match self {
            Self::Http { status, body } => (100..=599).contains(status) && body.len() <= 16_384,
            Self::Raw(bytes) => bytes.len() <= 16_384,
        }
    }

    fn write_to(&self, stream: &mut UnixStream) -> io::Result<()> {
        match self {
            Self::Http { status, body } => {
                let header = format!(
                    "HTTP/1.1 {status} Synthetic\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                    body.len(),
                );
                stream.write_all(header.as_bytes())?;
                stream.write_all(body)
            }
            Self::Raw(bytes) => stream.write_all(bytes),
        }
    }
}

struct FakeDocker {
    root: PathBuf,
    socket: PathBuf,
    seen: Arc<Mutex<Vec<(String, String)>>>,
    empty_connections: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    _directory: SocketTestDirectory,
}

impl FakeDocker {
    fn new(list: serde_json::Value, inspected: BTreeMap<&'static str, serde_json::Value>) -> io::Result<Self> {
        Self::with_replies(list, inspected, BTreeMap::new())
    }

    fn with_replies(
        list: serde_json::Value,
        inspected: BTreeMap<&'static str, serde_json::Value>,
        replies: BTreeMap<&'static str, FakeDockerReply>,
    ) -> io::Result<Self> {
        if replies.values().any(|reply| !reply.bounded()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "fake Docker reply exceeds its bounds",
            ));
        }
        let directory = SocketTestDirectory::new()?;
        let root = directory.0.clone();
        let socket = root.join("engine.sock");
        let listener = UnixListener::bind(&socket)?;
        listener.set_nonblocking(true)?;
        let seen = Arc::new(Mutex::new(Vec::new()));
        let empty_connections = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_seen = Arc::clone(&seen);
        let thread_empty_connections = Arc::clone(&empty_connections);
        let thread_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                let stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(_) => break,
                };
                let mut stream = stream;
                let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                while request.len() < 4096 && !request.ends_with(b"\r\n\r\n") {
                    match stream.read(&mut byte) {
                        Ok(1) => request.push(byte[0]),
                        _ => break,
                    }
                }
                let Some((method, path)) = String::from_utf8_lossy(&request).lines().next().and_then(|line| {
                    let mut words = line.split_ascii_whitespace();
                    Some((words.next()?.to_owned(), words.next()?.to_owned()))
                }) else {
                    if request.is_empty() && !thread_stop.load(Ordering::Relaxed) {
                        thread_empty_connections.fetch_add(1, Ordering::Relaxed);
                    }
                    continue;
                };
                if let Ok(mut seen) = thread_seen.lock() {
                    seen.push((method, path.clone()));
                }
                if let Some(reply) = replies.get(path.as_str()) {
                    let _ = reply.write_to(&mut stream);
                    continue;
                }
                let body = match path.as_str() {
                    "/version" => serde_json::json!({
                        "Version": "28.0.0", "ApiVersion": "1.50", "MinAPIVersion": "1.41"
                    }),
                    "/v1.49/info" => serde_json::json!({"ServerVersion": "28.0.0", "Rootless": false}),
                    "/v1.49/containers/json?all=1" => list.clone(),
                    _ => {
                        let id = path
                            .strip_prefix("/v1.49/containers/")
                            .and_then(|path| path.strip_suffix("/json"))
                            .or_else(|| path.strip_prefix("/v1.49/networks/"))
                            .or_else(|| path.strip_prefix("/v1.49/volumes/"));
                        id.and_then(|id| inspected.get(id).cloned())
                            .unwrap_or_else(|| serde_json::json!({}))
                    }
                };
                if let Ok(body) = serde_json::to_vec(&body) {
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                        body.len(),
                    );
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(&body);
                }
            }
        });
        Ok(Self {
            root,
            socket,
            seen,
            empty_connections,
            stop,
            thread: Some(thread),
            _directory: directory,
        })
    }

    fn socket(&self) -> &Path {
        &self.socket
    }

    fn paths(&self) -> io::Result<Vec<String>> {
        Ok(self.requests()?.into_iter().map(|(_, path)| path).collect())
    }

    fn requests(&self) -> io::Result<Vec<(String, String)>> {
        self.seen
            .lock()
            .map(|seen| seen.clone())
            .map_err(|_| io::Error::other("fake Docker request log poisoned"))
    }

    fn empty_connection_count(&self) -> u64 {
        self.empty_connections.load(Ordering::Relaxed)
    }
}

impl Drop for FakeDocker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = UnixStream::connect(&self.socket);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = fs::remove_file(&self.socket);
        let _ = fs::remove_dir(&self.root);
    }
}

fn summary(id: &str, name: &str, labels: serde_json::Value) -> serde_json::Value {
    let mut summary = serde_json::json!({"Id": id, "Names": [format!("/{name}")], "Image": "busybox:1"});
    summary["Labels"] = labels;
    summary
}

fn inspect(id: &str, name: &str) -> serde_json::Value {
    serde_json::json!({"Id": id, "Name": format!("/{name}"), "Config": {"Image": "busybox:1"}})
}

fn run(server: &FakeDocker, route: &[&str], options: &[&str]) -> io::Result<Output> {
    Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(route)
        .arg("--docker-socket")
        .arg(server.socket())
        .args(options)
        .args(["--console-format", "json"])
        .output()
}

fn assert_docker_healthcheck(body: &serde_json::Value, interval: u64, timeout: u64, retries: u64) {
    assert_eq!(body["Healthcheck"]["Test"], serde_json::json!(["CMD", "/bin/true"]));
    assert_eq!(body["Healthcheck"]["Interval"], interval);
    assert_eq!(body["Healthcheck"]["Timeout"], timeout);
    assert_eq!(body["Healthcheck"]["Retries"], retries);
}

/// Acquisition failures must stop before native inspection or neutral-model export.
/// Both commands exercise the real facade and `DockerLens` client against authored wire replies;
/// no target catalogue admission, daemon, or generated request execution is involved.
fn assert_docker_acquisition_failure(
    reply_path: &'static str,
    reply: &FakeDockerReply,
    expected_summary: &str,
    expected_paths: &[&str],
) -> Result<(), Box<dyn Error>> {
    for verb in ["validate", "convert"] {
        let server = FakeDocker::with_replies(
            serde_json::json!([summary(FIRST, "uninspected-private-container", serde_json::json!({}))]),
            BTreeMap::from([(FIRST, inspect(FIRST, "uninspected-private-container"))]),
            BTreeMap::from([(reply_path, reply.clone())]),
        )?;
        let destination = server.root.join("unwritten-compose-output");
        let mut command = Command::new(env!("CARGO_BIN_EXE_boxferry"));
        command
            .current_dir(&server.root)
            .args([verb, "docker", "compose", "--docker-socket"])
            .arg(server.socket())
            .args([
                "--docker-container-id",
                FIRST,
                "--loss-policy",
                "partial",
                "--console-format",
                "json",
            ]);
        if verb == "convert" {
            command.arg("--output-directory").arg(&destination);
        }
        let result = command.output()?;
        assert_eq!(result.status.code(), Some(1), "{verb}: {expected_summary}");
        let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
        assert_eq!(report["schema_version"], 1);
        assert_eq!(report["source_type"], "docker");
        assert_eq!(report["target_type"], "compose");
        assert_eq!(report["status"], "failure");
        assert_eq!(report["exit_category"], "input-or-execution");
        assert_eq!(report["failed_stage"], "input-discovery");
        assert_eq!(report["primary_diagnostic_code"], "BFO1000");
        let diagnostics = report["diagnostics"]
            .as_array()
            .ok_or("missing acquisition diagnostics")?;
        assert_eq!(
            diagnostics.len(),
            1,
            "a rejected source must not reach importer/exporter diagnostics"
        );
        assert_eq!(diagnostics[0]["code"], "BFO1000");
        assert_eq!(diagnostics[0]["name"], "orchestration-failed");
        assert_eq!(diagnostics[0]["severity"], "error");
        assert_eq!(diagnostics[0]["summary"], expected_summary);
        assert_eq!(report["inputs"], serde_json::json!([]));
        assert_eq!(report["discovery"], serde_json::json!([]));
        assert_eq!(report["output_artifacts"], serde_json::json!([]));
        let expected_requests = expected_paths
            .iter()
            .map(|path| ("GET".to_owned(), (*path).to_owned()))
            .collect::<Vec<_>>();
        assert_eq!(server.requests()?, expected_requests, "{verb}: {expected_summary}");
        assert!(
            !destination.exists(),
            "failed acquisition must not create an output directory"
        );
        let entries = fs::read_dir(&server.root)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<io::Result<Vec<_>>>()?;
        assert_eq!(
            entries,
            vec![std::ffi::OsString::from("engine.sock")],
            "failed acquisition wrote an artifact"
        );
        let public_text = format!(
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr),
        );
        for private in [
            PRIVATE_NATIVE_CANARY,
            "uninspected-private-container",
            FIRST,
            server.socket().to_str().ok_or("fake socket path is not UTF-8")?,
            destination.to_str().ok_or("fake output path is not UTF-8")?,
        ] {
            assert!(
                !public_text.contains(private),
                "{verb}: acquisition failure leaked private evidence"
            );
        }
    }
    Ok(())
}

#[test]
fn docker_cli_acquisition_opens_only_connections_with_read_only_requests() -> Result<(), Box<dyn Error>> {
    for verb in ["validate", "convert"] {
        let server = FakeDocker::new(
            serde_json::json!([summary(FIRST, "web", serde_json::json!({}))]),
            BTreeMap::from([(FIRST, inspect(FIRST, "web"))]),
        )?;
        let destination = server.root.join("no-preflight-output");
        let mut command = Command::new(env!("CARGO_BIN_EXE_boxferry"));
        command
            .args([verb, "docker", "compose", "--docker-socket"])
            .arg(server.socket())
            .args([
                "--docker-container-id",
                FIRST,
                "--loss-policy",
                "partial",
                "--console-format",
                "json",
            ]);
        if verb == "convert" {
            command.arg("--output-directory").arg(&destination);
        }
        let result = command.output()?;
        assert_eq!(
            result.status.code(),
            Some(0),
            "{verb}: {}",
            String::from_utf8_lossy(&result.stdout)
        );
        assert_eq!(
            server.empty_connection_count(),
            0,
            "{verb}: unexpected connect-only preflight"
        );
        assert_eq!(
            server.requests()?,
            vec![
                ("GET".into(), "/version".into()),
                ("GET".into(), "/v1.49/info".into()),
                ("GET".into(), format!("/v1.49/containers/{FIRST}/json")),
            ],
        );
        if verb == "convert" {
            fs::remove_file(destination.join("compose.yaml"))?;
            fs::remove_dir(destination)?;
        }
    }
    Ok(())
}

#[test]
fn docker_unconnectable_selected_socket_fails_without_fallback_artifacts_or_private_values()
-> Result<(), Box<dyn Error>> {
    for verb in ["validate", "convert"] {
        let fallback = FakeDocker::new(serde_json::json!([]), BTreeMap::new())?;
        let selected = fallback.root.join("private-selected.sock");
        drop(UnixListener::bind(&selected)?);
        let destination = fallback.root.join("private-unwritten-output");
        let mut command = Command::new(env!("CARGO_BIN_EXE_boxferry"));
        command
            .current_dir(&fallback.root)
            .args([verb, "docker", "compose", "--docker-socket"])
            .arg(&selected)
            .args(["--docker-container-id", FIRST, "--console-format", "json"]);
        if verb == "convert" {
            command.arg("--output-directory").arg(&destination);
        }
        let result = command.output()?;
        assert_eq!(result.status.code(), Some(1));
        let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
        assert_eq!(report["failed_stage"], "input-discovery");
        assert_eq!(report["primary_diagnostic_code"], "BFO1000");
        assert_eq!(report["diagnostics"][0]["summary"], "Docker acquisition failed: Io");
        assert_eq!(report["output_artifacts"], serde_json::json!([]));
        assert_eq!(report["inputs"], serde_json::json!([]));
        assert!(!destination.exists());
        assert!(fallback.requests()?.is_empty());
        assert_eq!(fallback.empty_connection_count(), 0);
        let public_text = format!(
            "{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        for private in [
            "private-selected",
            "private-unwritten",
            FIRST,
            fallback.root.to_str().ok_or("non-UTF-8 fixture path")?,
        ] {
            assert!(
                !public_text.contains(private),
                "{verb}: failed socket leaked private evidence"
            );
        }
        let mut entries = fs::read_dir(&fallback.root)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<io::Result<Vec<_>>>()?;
        entries.sort();
        assert_eq!(
            entries,
            vec![
                std::ffi::OsString::from("engine.sock"),
                std::ffi::OsString::from("private-selected.sock")
            ]
        );
        fs::remove_file(selected)?;
    }
    Ok(())
}

#[test]
fn docker_source_http_rejections_stop_at_the_failed_read_without_export_or_private_values() -> Result<(), Box<dyn Error>>
{
    let body = serde_json::to_vec(&serde_json::json!({"message": PRIVATE_NATIVE_CANARY}))?;
    for (path, status, expected_paths) in [
        ("/version", 500, vec!["/version"]),
        ("/v1.49/info", 403, vec!["/version", "/v1.49/info"]),
    ] {
        assert_docker_acquisition_failure(
            path,
            &FakeDockerReply::Http {
                status,
                body: body.clone(),
            },
            "Docker acquisition failed: Status",
            &expected_paths,
        )?;
    }
    Ok(())
}

#[test]
fn docker_source_malformed_version_json_is_a_value_free_negotiation_failure() -> Result<(), Box<dyn Error>> {
    assert_docker_acquisition_failure(
        "/version",
        &FakeDockerReply::Http {
            status: 200,
            body: format!("{{\"ApiVersion\":\"1.49\",\"private\":\"{PRIVATE_NATIVE_CANARY}\"").into_bytes(),
        },
        "Docker acquisition failed: Version",
        &["/version"],
    )
}

#[test]
fn docker_source_conflicting_response_lengths_are_a_value_free_protocol_failure() -> Result<(), Box<dyn Error>> {
    assert_docker_acquisition_failure(
        "/version",
        &FakeDockerReply::Raw(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nContent-Length: {}\r\n\r\n{PRIVATE_NATIVE_CANARY}",
                PRIVATE_NATIVE_CANARY.len(),
            )
            .into_bytes(),
        ),
        "Docker acquisition failed: Protocol",
        &["/version"],
    )
}

#[test]
fn docker_source_unsupported_api_bounds_stop_before_versioned_reads() -> Result<(), Box<dyn Error>> {
    // 1.40 is below the acquisition floor; 1.50 as a minimum exceeds the client's 1.49 ceiling.
    // These independently authored boundaries are source negotiation, not target capabilities.
    for (maximum, minimum) in [("1.40", "1.24"), ("1.50", "1.50")] {
        assert_docker_acquisition_failure(
            "/version",
            &FakeDockerReply::Http {
                status: 200,
                body: serde_json::to_vec(&serde_json::json!({
                    "Version": "28.0.0",
                    "ApiVersion": maximum,
                    "MinAPIVersion": minimum,
                    "PrivateFixtureCanary": PRIVATE_NATIVE_CANARY,
                }))?,
            },
            "Docker acquisition failed: Version",
            &["/version"],
        )?;
    }
    Ok(())
}

#[test]
fn docker_public_label_errors_and_help_never_echo_declarations() -> Result<(), Box<dyn Error>> {
    let private = "volume:private-label-source=private-label-key-canary";
    for options in [
        vec![
            "--docker-public-label",
            "image:private-label-source=private-label-key-canary",
        ],
        vec![
            "--docker-public-label",
            "container:private-label-source=private-label-key-canary",
        ],
        vec!["--docker-public-label", "volume:private-label-source="],
        vec!["--docker-public-label", private, "--docker-public-label", private],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_boxferry"))
            .args(["validate", "docker", "compose"])
            .args(options)
            .output()?;
        assert!(!output.status.success());
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(text.contains("Docker public label declaration"));
        assert!(!text.contains("private-label-source") && !text.contains("private-label-key-canary"));
        assert!(
            !text.contains("conventional local Docker socket"),
            "input validation must precede acquisition"
        );
    }
    for route in [["docker", "compose"], ["compose", "compose"]] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_boxferry"));
        command.args(["validate", route[0], route[1], "--docker-public-label", private]);
        if route[0] == "docker" {
            command.arg("--help");
        }
        let output = command.output()?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!text.contains("private-label-source") && !text.contains("private-label-key-canary"));
        if route[0] == "docker" {
            assert!(output.status.success());
            assert!(text.contains("--docker-public-label"));
        } else {
            assert!(!output.status.success());
        }
    }
    Ok(())
}

#[test]
fn docker_resource_decision_errors_never_echo_keys_or_destinations() -> Result<(), Box<dyn Error>> {
    let private = "volume:private-source=create:private-destination";
    for options in [
        vec!["--docker-resource-decision", "volume:private-source=create:"],
        vec![
            "--docker-resource-decision",
            private,
            "--docker-resource-decision",
            private,
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_boxferry"))
            .args(["validate", "docker", "compose"])
            .args(options)
            .output()?;
        assert!(!output.status.success());
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            text.contains("Docker resource decision"),
            "missing scoped error: {text}"
        );
        assert!(!text.contains("private-source") && !text.contains("private-destination"));
    }
    Ok(())
}

#[test]
fn unselected_docker_resource_decision_fails_without_identity_in_report() -> Result<(), Box<dyn Error>> {
    let server = FakeDocker::new(
        serde_json::json!([summary(FIRST, "web", serde_json::json!({}))]),
        BTreeMap::from([(FIRST, inspect(FIRST, "web"))]),
    )?;
    let output = run(
        &server,
        &["validate", "docker", "compose"],
        &[
            "--docker-resource-decision",
            "volume:private-source=create:private-destination",
            "--loss-policy",
            "partial",
        ],
    )?;
    assert!(!output.status.success());
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!text.contains("private-source") && !text.contains("private-destination"));
    assert!(text.contains("resource_decisions") || text.contains("resource decision"));
    assert!(server.paths()?.iter().all(|path| !path.contains("/create")));
    Ok(())
}

#[test]
fn selected_docker_volume_decision_writes_only_explicit_private_destination() -> Result<(), Box<dyn Error>> {
    let server = FakeDocker::new(
        serde_json::json!([]),
        BTreeMap::from([(
            "private-source",
            serde_json::json!({
                "Name":"private-source", "Driver":"local", "Mountpoint":"/private/source-mount", "Options":{}
            }),
        )]),
    )?;
    let destination = server.root.join("plan");
    let output = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["convert", "docker", "docker", "--docker-socket"])
        .arg(server.socket())
        .args([
            "--docker-volume",
            "private-source",
            "--docker-resource-decision",
            "volume:private-source=create:private-destination",
            "--docker-target",
            "upstream-29.8.1-rootful",
            "--loss-policy",
            "partial",
            "--console-format",
            "json",
        ])
        .arg("--output-directory")
        .arg(&destination)
        .output()?;
    assert!(
        output.status.success(),
        "Docker volume plan failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = String::from_utf8(output.stdout)?;
    assert!(!report.contains("private-source") && !report.contains("private-destination"));
    assert!(report.contains("--docker-resource-decision"));
    let plan = fs::read_to_string(destination.join("docker-plan.json"))?;
    assert!(plan.contains("\"Name\":\"private-destination\""));
    assert!(!plan.contains("private-source"));
    assert_eq!(plan.matches("/volumes/create").count(), 1);
    assert!(server.paths()?.iter().all(|path| !path.contains("/create")));
    Ok(())
}

#[test]
fn default_docker_source_selects_only_the_sole_discovered_container() -> Result<(), Box<dyn Error>> {
    let server = FakeDocker::new(
        serde_json::json!([summary(FIRST, "web", serde_json::json!({}))]),
        BTreeMap::from([(FIRST, inspect(FIRST, "web"))]),
    )?;
    let result = run(
        &server,
        &["validate", "docker", "compose"],
        &["--loss-policy", "partial"],
    )?;
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(report["source_type"], "docker");
    assert_eq!(report["target_type"], "compose");
    assert_eq!(report["application"], "docker-import");
    let paths = server.paths()?;
    assert_eq!(
        paths
            .iter()
            .filter(|path| path.as_str() == "/v1.49/containers/json?all=1")
            .count(),
        1
    );
    assert_eq!(
        paths
            .iter()
            .filter(|path| path.contains(FIRST) && path.ends_with("/json"))
            .count(),
        1
    );
    assert!(paths.iter().all(|path| !path.contains(SECOND)));
    assert!(paths.iter().all(|path| !path.contains("/create")));
    Ok(())
}

#[test]
fn default_docker_source_selects_complete_project_replicas_by_distinct_ids() -> Result<(), Box<dyn Error>> {
    let labels =
        serde_json::json!({"com.docker.compose.project": "private-project", "com.docker.compose.service": "web"});
    let server = FakeDocker::new(
        serde_json::json!([
            summary(FIRST, "web-1", labels.clone()),
            summary(SECOND, "web-2", labels)
        ]),
        BTreeMap::from([(FIRST, inspect(FIRST, "web-1")), (SECOND, inspect(SECOND, "web-2"))]),
    )?;
    let result = run(
        &server,
        &["validate", "docker", "compose"],
        &["--loss-policy", "partial"],
    )?;
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    let paths = server.paths()?;
    assert!(paths.iter().any(|path| path.contains(FIRST) && path.ends_with("/json")));
    assert!(
        paths
            .iter()
            .any(|path| path.contains(SECOND) && path.ends_with("/json"))
    );
    let text = String::from_utf8(result.stdout)?;
    assert!(!text.contains("private-project"));
    Ok(())
}

#[test]
fn ambiguous_docker_discovery_never_inspects_a_peer_or_exposes_labels() -> Result<(), Box<dyn Error>> {
    let server = FakeDocker::new(
        serde_json::json!([
            summary(
                FIRST,
                "web",
                serde_json::json!({"io.boxferry.application": "private-one"})
            ),
            summary(
                SECOND,
                "peer",
                serde_json::json!({"io.boxferry.application": "private-two"})
            ),
        ]),
        BTreeMap::from([(FIRST, inspect(FIRST, "web")), (SECOND, inspect(SECOND, "peer"))]),
    )?;
    let result = run(
        &server,
        &["validate", "docker", "compose"],
        &["--loss-policy", "partial"],
    )?;
    assert_eq!(result.status.code(), Some(1));
    let paths = server.paths()?;
    assert!(paths.iter().all(|path| !path.contains(FIRST) && !path.contains(SECOND)));
    let text = String::from_utf8(result.stdout)?;
    assert!(text.contains("--docker-container"));
    assert!(!text.contains("private-one"));
    assert!(!text.contains("private-two"));
    assert!(!text.contains(server.socket().to_string_lossy().as_ref()));
    Ok(())
}

#[test]
fn explicit_docker_id_bypasses_ambiguous_list_without_inspecting_peer() -> Result<(), Box<dyn Error>> {
    let server = FakeDocker::new(
        serde_json::json!([
            summary(FIRST, "web", serde_json::json!({"io.boxferry.application": "one"})),
            summary(SECOND, "peer", serde_json::json!({"io.boxferry.application": "two"})),
        ]),
        BTreeMap::from([(FIRST, inspect(FIRST, "web")), (SECOND, inspect(SECOND, "peer"))]),
    )?;
    let result = run(
        &server,
        &["validate", "docker", "compose"],
        &["--docker-container-id", FIRST, "--loss-policy", "partial"],
    )?;
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    let paths = server.paths()?;
    assert!(paths.iter().all(|path| !path.contains("/containers/json?all=1")));
    assert!(paths.iter().any(|path| path.contains(FIRST) && path.ends_with("/json")));
    assert!(paths.iter().all(|path| !path.contains(SECOND)));
    Ok(())
}

#[test]
fn docker_same_format_route_writes_complete_private_inert_plan_with_independent_target() -> Result<(), Box<dyn Error>> {
    let server = FakeDocker::new(
        serde_json::json!([summary(FIRST, "web", serde_json::json!({}))]),
        BTreeMap::from([(FIRST, inspect(FIRST, "web"))]),
    )?;
    let destination = server.root.join("plan");
    let result = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["convert", "docker", "docker", "--docker-socket"])
        .arg(server.socket())
        .args([
            "--docker-target",
            "upstream-29.8.1-rootless",
            "--loss-policy",
            "partial",
        ])
        .arg("--output-directory")
        .arg(&destination)
        .args(["--console-format", "json"])
        .output()?;
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(report["source_type"], "docker");
    assert_eq!(report["target_type"], "docker");
    assert_eq!(report["requested_versions"]["minimum"], "upstream-29.8.1-rootless");
    assert_eq!(report["resolved_versions"]["minimum"], "29.8.1");
    let artifact = destination.join("docker-plan.json");
    let plan: serde_json::Value = serde_json::from_slice(&fs::read(&artifact)?)?;
    assert_eq!(plan["schema_version"], 1);
    assert_eq!(plan["context"]["kind"], "target");
    assert_eq!(plan["context"]["engine_release"], "29.8.1");
    assert_eq!(plan["context"]["daemon_mode"], "rootless");
    assert_eq!(plan["requests"].as_array().map(Vec::len), Some(1));
    assert_eq!(plan["prerequisites"].as_array().map(Vec::len), Some(0));
    assert_eq!(fs::metadata(&destination)?.mode() & 0o777, 0o700);
    assert_eq!(fs::metadata(&artifact)?.mode() & 0o777, 0o600);
    assert!(
        server
            .paths()?
            .iter()
            .all(|path| path.starts_with("/version") || path.contains("/info") || path.contains("/containers"))
    );
    fs::remove_file(artifact)?;
    fs::remove_dir(destination)?;
    Ok(())
}

#[test]
fn docker_command_clear_without_exec_entrypoint_reports_field_and_writes_no_plan() -> Result<(), Box<dyn Error>> {
    let mut observed = inspect(FIRST, "web");
    observed["Config"]["Image"] = serde_json::json!("private.example/image-canary:1");
    observed["Config"]["Cmd"] = serde_json::json!([]);
    observed["Config"]["Entrypoint"] = serde_json::json!([]);
    let server = FakeDocker::new(
        serde_json::json!([summary(FIRST, "web", serde_json::json!({}))]),
        BTreeMap::from([(FIRST, observed)]),
    )?;
    let destination = server.root.join("unsafe-command-plan");
    let result = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["convert", "docker", "docker", "--docker-socket"])
        .arg(server.socket())
        .args(["--docker-target", "upstream-29.8.1-rootful", "--loss-policy", "partial"])
        .arg("--output-directory")
        .arg(&destination)
        .args(["--console-format", "json"])
        .output()?;
    assert_eq!(result.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(report["status"], "failure");
    assert!(
        report["diagnostics"]
            .as_array()
            .is_some_and(|diagnostics| diagnostics.iter().any(|diagnostic| {
                diagnostic["code"] == "BFD0007"
                    && diagnostic["fields"].as_array().is_some_and(|fields| {
                        fields.iter().any(|field| {
                            field["name"] == "subject" && field["value"] == "services.docker-service-1.command"
                        })
                    })
            }))
    );
    let report_text = format!(
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(report_text.contains("explicit nonempty exec entrypoint"));
    assert!(!report_text.contains("private.example") && !report_text.contains("image-canary"));
    assert!(
        !destination.exists(),
        "invalid native intent must not emit a Docker plan"
    );
    assert!(server.paths()?.iter().all(|path| !path.contains("/create")));
    Ok(())
}

#[test]
fn docker_network_capture_keeps_alias_private_and_blocks_unowned_target_topology() -> Result<(), Box<dyn Error>> {
    const NETWORK_ID: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
    let inspected = serde_json::json!({
        "Id": FIRST,
        "Name": "/web",
        "Config": {"Image": "example.invalid/web:1"},
        "HostConfig": {"NetworkMode": "bridge"},
        "NetworkSettings": {"Networks": {"private-network": {
            "NetworkID": NETWORK_ID,
            "Aliases": ["private-alias"]
        }}}
    });
    let network = serde_json::json!({
        "Id": NETWORK_ID,
        "Name": "private-network",
        "Driver": "bridge",
        "Internal": false
    });
    let server = FakeDocker::new(
        serde_json::json!([summary(FIRST, "web", serde_json::json!({}))]),
        BTreeMap::from([(FIRST, inspected), (NETWORK_ID, network)]),
    )?;
    let destination = server.root.join("unowned-network-plan");
    let result = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["convert", "docker", "docker", "--docker-socket"])
        .arg(server.socket())
        .args([
            "--docker-container-id",
            FIRST,
            "--docker-target",
            "upstream-29.8.1-rootful",
            "--loss-policy",
            "partial",
        ])
        .arg("--output-directory")
        .arg(&destination)
        .args(["--console-format", "json"])
        .output()?;
    assert_eq!(result.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(report["status"], "failure");
    assert!(!String::from_utf8_lossy(&result.stdout).contains("private-network"));
    assert!(!String::from_utf8_lossy(&result.stdout).contains("private-alias"));
    assert!(
        report["diagnostics"]
            .as_array()
            .is_some_and(|diagnostics| diagnostics.iter().any(|diagnostic| {
                diagnostic["code"] == "BFD0006"
                    && diagnostic["fields"].as_array().is_some_and(|fields| {
                        fields.iter().any(|field| {
                            field["name"] == "subject" && field["value"] == "networks.docker-network-1.ownership"
                        })
                    })
            })),
        "uncertain network ownership needs an exact actionable loss"
    );
    assert!(
        report["diagnostics"]
            .as_array()
            .is_some_and(|diagnostics| diagnostics.iter().any(|diagnostic| {
                diagnostic["code"] == "BFD0007"
                    && diagnostic["fields"].as_array().is_some_and(|fields| {
                        fields.iter().any(|field| {
                            field["name"] == "subject" && field["value"] == "services.docker-service-1.networks.0"
                        })
                    })
            })),
        "the unresolved network cannot become a standalone service attachment"
    );
    assert!(
        !destination.exists(),
        "an unowned Docker network cannot become a standalone create request"
    );
    assert!(
        server
            .paths()?
            .iter()
            .any(|path| path == &format!("/v1.49/networks/{NETWORK_ID}"))
    );
    Ok(())
}

fn assert_docker_process_identity_policy_refusal(server: &FakeDocker) -> Result<(), Box<dyn Error>> {
    let strict = run(
        server,
        &["validate", "docker", "docker"],
        &[
            "--docker-container-id",
            FIRST,
            "--docker-target",
            "upstream-29.8.1-rootful",
            "--loss-policy",
            "exact",
        ],
    )?;
    assert_eq!(
        strict.status.code(),
        Some(2),
        "effective runtime identity must require an explicit approximation decision"
    );
    assert!(strict.stderr.is_empty());
    let strict_report: serde_json::Value = serde_json::from_slice(&strict.stdout)?;
    assert_eq!(strict_report["status"], "blocked");
    assert_eq!(strict_report["exit_category"], "policy-blocked");
    assert_eq!(strict_report["primary_diagnostic_code"], "BFD0004");
    assert_eq!(strict_report["fidelity"]["approximate"], 1);
    assert_eq!(strict_report["fidelity"]["invalid"], 0);
    assert_eq!(strict_report["fidelity"]["unsupported"], 0);
    assert_eq!(strict_report["output_artifacts"], serde_json::json!([]));
    let strict_diagnostics = strict_report["diagnostics"]
        .as_array()
        .ok_or("strict Docker diagnostics missing")?;
    assert!(
        !strict_diagnostics
            .iter()
            .any(|diagnostic| diagnostic["code"] == "BFD0007")
    );
    assert!(strict_diagnostics.iter().any(|diagnostic| {
        diagnostic["code"] == "BFD0004"
            && diagnostic["fields"].as_array().is_some_and(|fields| {
                fields
                    .iter()
                    .any(|field| field["name"] == "subject" && field["value"] == "services[0].runtime_name")
            })
    }));
    for private_value in ["web-runtime", "hello-operator-review", "/srv/app", FIRST] {
        assert!(!String::from_utf8_lossy(&strict.stdout).contains(private_value));
    }
    assert!(
        !server.root.join("reviewed-process-plan").exists(),
        "strict validation must not emit a native artifact"
    );
    Ok(())
}

#[test]
fn docker_same_format_preserves_reviewed_process_health_and_runtime_identity_as_private_intent()
-> Result<(), Box<dyn Error>> {
    let inspected = serde_json::json!({
        "Id": FIRST,
        "Name": "/web-runtime",
        "Config": {
            "Image": "example.invalid/web:1",
            "Cmd": ["hello-operator-review"],
            "Entrypoint": ["/bin/echo"],
            "User": "1000",
            "WorkingDir": "/srv/app",
            "Healthcheck": {
                "Test": ["CMD", "/bin/true"],
                "Interval": 1_000_000_000,
                "Timeout": 1_000_000_000,
                "Retries": 3
            }
        },
        "HostConfig": {"RestartPolicy": {"Name": "no", "MaximumRetryCount": 0}},
        "Mounts": []
    });
    let server = FakeDocker::new(
        serde_json::json!([summary(FIRST, "web-runtime", serde_json::json!({}))]),
        BTreeMap::from([(FIRST, inspected)]),
    )?;
    assert_docker_process_identity_policy_refusal(&server)?;
    let destination = server.root.join("reviewed-process-plan");
    let result = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["convert", "docker", "docker", "--docker-socket"])
        .arg(server.socket())
        .args([
            "--docker-container-id",
            FIRST,
            "--docker-target",
            "upstream-29.8.1-rootful",
            "--loss-policy",
            "approximate",
        ])
        .arg("--output-directory")
        .arg(&destination)
        .args(["--console-format", "json"])
        .output()?;
    assert_eq!(
        result.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    assert!(result.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(report["status"], "success");
    assert_eq!(report["requested_versions"]["minimum"], "upstream-29.8.1-rootful");
    assert!(!String::from_utf8_lossy(&result.stdout).contains("hello-operator-review"));
    assert!(!String::from_utf8_lossy(&result.stdout).contains("web-runtime"));
    let artifact = destination.join("docker-plan.json");
    let bytes = fs::read(&artifact)?;
    let plan: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert_eq!(plan["schema_version"], 1);
    assert_eq!(plan["context"]["kind"], "target");
    assert_eq!(plan["context"]["engine_release"], "29.8.1");
    assert_eq!(plan["context"]["daemon_mode"], "rootful");
    let requests = plan["requests"].as_array().ok_or("native Docker requests")?;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["method"], "POST");
    assert_eq!(requests[0]["path"], "/v1.56/containers/create?name=web-runtime");
    let body = &requests[0]["body"];
    assert_eq!(body["Image"], "example.invalid/web:1");
    assert_eq!(body["Cmd"], serde_json::json!(["hello-operator-review"]));
    assert_eq!(body["Entrypoint"], serde_json::json!(["/bin/echo"]));
    assert_eq!(body["User"], "1000");
    assert_eq!(body["WorkingDir"], "/srv/app");
    assert_docker_healthcheck(body, 1_000_000_000, 1_000_000_000, 3);
    assert_eq!(body["HostConfig"]["RestartPolicy"]["Name"], "no");
    assert_eq!(plan["prerequisites"], serde_json::json!([]));
    assert_eq!(fs::metadata(&destination)?.mode() & 0o777, 0o700);
    assert_eq!(fs::metadata(&artifact)?.mode() & 0o777, 0o600);
    assert!(
        server
            .paths()?
            .iter()
            .all(|path| path.starts_with("/version") || path.contains("/info") || path.contains("/containers/"))
    );
    fs::remove_file(artifact)?;
    fs::remove_dir(destination)?;
    Ok(())
}

// This distinct fixture uses the current reviewed command/health/restart admission.
// User/workdir output remains a separate positive contract in the preceding test.
#[test]
fn docker_same_format_admitted_process_health_requires_policy_consent_and_stays_inert() -> Result<(), Box<dyn Error>> {
    let inspected = serde_json::json!({
        "Id": FIRST,
        "Name": "/offline-worker",
        "Config": {
            "Image": "example.invalid/process:1",
            "Cmd": ["reviewed-offline-argument"],
            "Entrypoint": ["/bin/echo"],
            "Healthcheck": {
                "Test": ["CMD", "/bin/true"],
                "Interval": 1_000_000_000,
                "Timeout": 2_000_000_000,
                "Retries": 4
            }
        },
        "HostConfig": {"RestartPolicy": {"Name": "on-failure", "MaximumRetryCount": 3}},
        "Mounts": []
    });
    for policy in ["exact", "approximate"] {
        let server = FakeDocker::new(
            serde_json::json!([summary(FIRST, "offline-worker", serde_json::json!({}))]),
            BTreeMap::from([(FIRST, inspected.clone())]),
        )?;
        let destination = server.root.join("admitted-process-plan");
        let mut command = Command::new(env!("CARGO_BIN_EXE_boxferry"));
        command
            .args([
                if policy == "exact" { "validate" } else { "convert" },
                "docker",
                "docker",
            ])
            .arg("--docker-socket")
            .arg(server.socket())
            .args([
                "--docker-target",
                "upstream-29.8.1-rootful",
                "--loss-policy",
                policy,
                "--console-format",
                "json",
            ]);
        if policy == "approximate" {
            command.arg("--output-directory").arg(&destination);
        }
        let result = command.output()?;
        assert_eq!(
            result.status.code(),
            Some(if policy == "exact" { 2 } else { 0 }),
            "{policy}: stdout={} stderr={}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(result.stderr.is_empty(), "{policy}");
        for private_value in ["offline-worker", "reviewed-offline-argument", FIRST] {
            assert!(
                !String::from_utf8_lossy(&result.stdout).contains(private_value),
                "{policy}"
            );
        }
        let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
        assert_eq!(report["source_type"], "docker");
        assert_eq!(report["target_type"], "docker");
        assert_eq!(report["fidelity"]["invalid"], 0, "{policy}");
        assert_eq!(report["fidelity"]["unsupported"], 0, "{policy}");
        assert_eq!(report["fidelity"]["approximate"], 1, "{policy}");
        let diagnostics = report["diagnostics"].as_array().ok_or("Docker diagnostics missing")?;
        assert!(!diagnostics.iter().any(|diagnostic| diagnostic["code"] == "BFD0007"));
        assert!(diagnostics.iter().any(|diagnostic| {
            diagnostic["code"] == "BFD0004"
                && diagnostic["fields"].as_array().is_some_and(|fields| {
                    fields
                        .iter()
                        .any(|field| field["name"] == "subject" && field["value"] == "services[0].runtime_name")
                })
        }));
        assert_minimal_docker_read_only_acquisition(&server, policy)?;
        if policy == "exact" {
            assert_eq!(report["status"], "blocked");
            assert_eq!(report["exit_category"], "policy-blocked");
            assert_eq!(report["primary_diagnostic_code"], "BFD0004");
            assert_eq!(report["output_artifacts"], serde_json::json!([]));
            assert!(!destination.exists());
        } else {
            assert_eq!(report["status"], "success");
            assert_admitted_process_health_docker_artifact(&destination, &report)?;
            fs::remove_file(destination.join("docker-plan.json"))?;
            fs::remove_dir(destination)?;
        }
    }
    Ok(())
}

fn assert_admitted_process_health_docker_artifact(
    destination: &Path,
    report: &serde_json::Value,
) -> Result<(), Box<dyn Error>> {
    let path = destination.join("docker-plan.json");
    let bytes = fs::read(&path)?;
    assert_eq!(
        fs::read_dir(destination)?.count(),
        1,
        "only the inert native artifact is written"
    );
    assert_eq!(
        report["output_artifacts"],
        serde_json::json!([{"name": "docker-plan.json", "size": bytes.len()}])
    );
    assert_eq!(report["requested_versions"]["minimum"], "upstream-29.8.1-rootful");
    assert_eq!(report["resolved_versions"]["minimum"], "29.8.1");
    let plan: serde_json::Value = serde_json::from_slice(&bytes)?;
    assert_eq!(plan["schema_version"], 1);
    assert_eq!(plan["context"]["kind"], "target");
    assert_eq!(plan["context"]["engine_release"], "29.8.1");
    assert_eq!(plan["context"]["daemon_mode"], "rootful");
    assert_eq!(plan["context"]["rendering_api_version"], "1.56");
    assert_eq!(plan["prerequisites"], serde_json::json!([]));
    let requests = plan["requests"].as_array().ok_or("Docker create requests missing")?;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["method"], "POST");
    assert_eq!(requests[0]["path"], "/v1.56/containers/create?name=offline-worker");
    let body = &requests[0]["body"];
    assert_eq!(body["Image"], "example.invalid/process:1");
    assert_eq!(body["Cmd"], serde_json::json!(["reviewed-offline-argument"]));
    assert_eq!(body["Entrypoint"], serde_json::json!(["/bin/echo"]));
    assert_docker_healthcheck(body, 1_000_000_000, 2_000_000_000, 4);
    assert_eq!(body["HostConfig"]["RestartPolicy"]["Name"], "on-failure");
    assert_eq!(body["HostConfig"]["RestartPolicy"]["MaximumRetryCount"], 3);
    assert_eq!(fs::metadata(destination)?.mode() & 0o777, 0o700);
    assert_eq!(fs::metadata(path)?.mode() & 0o777, 0o600);
    Ok(())
}

#[test]
fn docker_source_environment_and_output_value_consent_are_independent() -> Result<(), Box<dyn Error>> {
    let inspected = serde_json::json!({
        "Id": FIRST,
        "Name": "/web",
        "Config": {"Image": "busybox:1", "Env": ["TOKEN=private-canary"]},
    });
    let server = FakeDocker::new(
        serde_json::json!([summary(FIRST, "web", serde_json::json!({}))]),
        BTreeMap::from([(FIRST, inspected)]),
    )?;
    for (name, source_consent, output_consent, emitted) in [
        ("withheld", false, false, false),
        ("source-only", true, false, false),
        ("output-only", false, true, false),
        ("both", true, true, true),
    ] {
        let destination = server.root.join(name);
        let mut command = Command::new(env!("CARGO_BIN_EXE_boxferry"));
        command
            .args(["convert", "docker", "docker", "--docker-socket"])
            .arg(server.socket())
            .args(["--docker-target", "upstream-29.8.1-rootful", "--loss-policy", "partial"]);
        if source_consent {
            command.arg("--promote-docker-protected-environment-values");
        }
        if output_consent {
            command.args(["--environment-values", "include"]);
        }
        let result = command
            .arg("--output-directory")
            .arg(&destination)
            .args(["--console-format", "json"])
            .output()?;
        assert_eq!(result.status.code(), Some(if emitted { 0 } else { 2 }), "{name}");
        assert!(!String::from_utf8_lossy(&result.stdout).contains("private-canary"));
        let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
        let artifact = destination.join("docker-plan.json");
        if !emitted {
            assert_eq!(report["status"], "blocked", "{name}");
            assert!(
                report["diagnostics"]
                    .as_array()
                    .is_some_and(|diagnostics| diagnostics.iter().any(|diagnostic| {
                        diagnostic["code"] == "BFD0006"
                            && diagnostic["fields"].as_array().is_some_and(|fields| {
                                fields.iter().any(|field| {
                                    field["name"] == "subject"
                                        && field["value"] == "services.docker-service-1.environment.TOKEN"
                                })
                            })
                    })),
                "missing named Docker environment loss: {name}"
            );
            assert!(
                !artifact.exists(),
                "incomplete standalone Docker request escaped: {name}"
            );
            assert!(
                !destination.exists(),
                "blocked Docker route created an output directory: {name}"
            );
            continue;
        }
        assert_eq!(report["status"], "success", "{name}");
        let bytes = fs::read(&artifact)?;
        assert_eq!(
            String::from_utf8_lossy(&bytes).contains("private-canary"),
            emitted,
            "{name}"
        );
        assert_eq!(fs::metadata(&destination)?.mode() & 0o777, 0o700);
        assert_eq!(fs::metadata(&artifact)?.mode() & 0o777, 0o600);
        fs::remove_file(artifact)?;
        fs::remove_dir(destination)?;
    }
    Ok(())
}

#[test]
fn docker_source_reaches_every_exporter_through_the_cli() -> Result<(), Box<dyn Error>> {
    let server = FakeDocker::new(
        serde_json::json!([summary(FIRST, "web", serde_json::json!({}))]),
        BTreeMap::from([(FIRST, inspect(FIRST, "web"))]),
    )?;
    for target in ["compose", "quadlet", "podman", "docker"] {
        let mut options = vec!["--loss-policy", "partial"];
        if target == "docker" {
            options.extend(["--docker-target", "debian11-20.10.5-rootful"]);
        }
        if target == "podman" {
            options.extend(["--podman-target-context", "unknown"]);
        }
        let result = run(&server, &["validate", "docker", target], &options)?;
        let expected_status = i32::from(target == "podman");
        assert_eq!(
            result.status.code(),
            Some(expected_status),
            "{target}: stdout={} stderr={}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
        assert_eq!(report["source_type"], "docker");
        assert_eq!(report["target_type"], target);
        if target == "podman" {
            assert_eq!(report["primary_diagnostic_code"], "BFP0008");
            assert!(report["output_artifacts"].as_array().is_some_and(Vec::is_empty));
        }
    }
    Ok(())
}

fn docker_row_options(target: &str) -> Vec<&'static str> {
    match target {
        "docker" => vec!["--docker-target", "upstream-29.8.1-rootful"],
        "podman" => vec!["--podman-target-context", "unknown", "--podman-max-version", "6.1.0"],
        _ => Vec::new(),
    }
}

#[test]
fn docker_row_validation_conversion_parity_and_exact_default_refusal() -> Result<(), Box<dyn Error>> {
    for target in ["compose", "docker", "podman", "quadlet"] {
        let mut inspected = inspect(FIRST, "web");
        inspected["Config"]["Image"] = serde_json::json!("docker.io/library/busybox:1");
        let server = FakeDocker::new(
            serde_json::json!([summary(FIRST, "web", serde_json::json!({}))]),
            BTreeMap::from([(FIRST, inspected)]),
        )?;
        for (policy, expected_status) in [(None, 2), (Some("approximate"), 0)] {
            let destination = server.root.join(if policy.is_some() { "approved" } else { "blocked" });
            let mut options = docker_row_options(target);
            if let Some(policy) = policy {
                options.extend(["--loss-policy", policy]);
            }
            let validated = run(&server, &["validate", "docker", target], &options)?;
            assert_eq!(validated.status.code(), Some(expected_status), "validate {target}");
            assert!(validated.stderr.is_empty());
            assert!(!destination.exists(), "validation must not create output");
            let converted = Command::new(env!("CARGO_BIN_EXE_boxferry"))
                .args(["convert", "docker", target, "--docker-socket"])
                .arg(server.socket())
                .args(&options)
                .args(["--console-format", "json", "--output-directory"])
                .arg(&destination)
                .output()?;
            assert_eq!(converted.status.code(), Some(expected_status), "convert {target}");
            assert!(converted.stderr.is_empty());
            let validated: serde_json::Value = serde_json::from_slice(&validated.stdout)?;
            let converted: serde_json::Value = serde_json::from_slice(&converted.stdout)?;
            for key in [
                "schema_version",
                "status",
                "source_type",
                "target_type",
                "application",
                "fidelity",
                "diagnostics",
                "choices",
                "requested_versions",
                "resolved_versions",
                "output_artifacts",
            ] {
                assert_eq!(
                    validated.get(key).ok_or("validation field missing")?,
                    converted.get(key).ok_or("conversion field missing")?,
                    "{target}: {key}"
                );
            }
            assert_eq!(converted["fidelity"]["approximate"], 1);
            assert_eq!(converted["fidelity"]["unsupported"], 0);
            assert_eq!(
                destination.exists(),
                policy.is_some(),
                "blocked conversion must not create output"
            );
            for canary in [
                FIRST,
                PRIVATE_NATIVE_CANARY,
                server.socket().to_str().ok_or("socket path")?,
            ] {
                assert!(!validated.to_string().contains(canary));
                assert!(!converted.to_string().contains(canary));
            }
        }
        let inspect_path = format!("/v1.49/containers/{FIRST}/json");
        let requests = server.requests()?;
        assert_eq!(requests.iter().filter(|(_, path)| path == &inspect_path).count(), 4);
        for (method, path) in requests {
            assert_eq!(method, "GET", "only read-only acquisition is allowed");
            assert!(
                matches!(
                    path.as_str(),
                    "/version" | "/v1.49/info" | "/v1.49/containers/json?all=1"
                ) || path == inspect_path
            );
        }
    }
    Ok(())
}

// Synthetic acquisition and file inspection prove offline conversion only. Generated
// Docker requests and Podman commands remain inert; no native runtime is consulted.
#[test]
fn docker_source_conversion_writes_every_exporter_artifact_without_applying_it() -> Result<(), Box<dyn Error>> {
    let podman_target_version = "6.1.0";
    for (target, expected_files) in [
        ("compose", vec!["compose.yaml"]),
        ("quadlet", vec!["docker-service-1.container"]),
        ("podman", vec!["podman-commands.sh", "podman.json"]),
        ("docker", vec!["docker-plan.json"]),
    ] {
        let mut inspected = inspect(FIRST, "web");
        inspected["Config"]["Image"] = serde_json::json!("docker.io/library/busybox:1");
        let server = FakeDocker::new(
            serde_json::json!([summary(FIRST, "web", serde_json::json!({}))]),
            BTreeMap::from([(FIRST, inspected)]),
        )?;
        let destination = server.root.join(target);
        let mut command = Command::new(env!("CARGO_BIN_EXE_boxferry"));
        command
            .args(["convert", "docker", target, "--docker-socket"])
            .arg(server.socket())
            .args(["--loss-policy", "approximate", "--console-format", "json"])
            .arg("--output-directory")
            .arg(&destination);
        if target == "docker" {
            command.args(["--docker-target", "upstream-29.8.1-rootful"]);
        }
        if target == "podman" {
            command.args([
                "--podman-target-context",
                "unknown",
                "--podman-max-version",
                podman_target_version,
            ]);
        }
        let result = command.output()?;
        assert_eq!(
            result.status.code(),
            Some(0),
            "{target}: stdout={} stderr={}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(result.stderr.is_empty(), "{target}");
        assert!(!String::from_utf8_lossy(&result.stdout).contains(FIRST), "{target}");
        let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
        assert_eq!(report["status"], "success", "{target}");
        assert_eq!(report["source_type"], "docker", "{target}");
        assert_eq!(report["target_type"], target);
        assert_eq!(report["application"], "docker-import", "{target}");
        assert_eq!(report["invocation"]["command_kind"], "convert", "{target}");
        assert_minimal_docker_source_approximation(&report, target)?;
        let artifacts = report["output_artifacts"]
            .as_array()
            .ok_or("output artifacts missing")?;
        assert_eq!(
            artifacts
                .iter()
                .map(|artifact| artifact["name"].as_str())
                .collect::<Vec<_>>(),
            expected_files.iter().map(|name| Some(*name)).collect::<Vec<_>>(),
            "{target} report artifact inventory"
        );
        let mut written = fs::read_dir(&destination)?
            .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
            .collect::<Result<Vec<_>, _>>()?;
        written.sort();
        assert_eq!(written, expected_files, "{target} written artifact inventory");
        for artifact in artifacts {
            let path = destination.join(artifact["name"].as_str().ok_or("artifact name missing")?);
            assert_eq!(
                artifact["size"],
                fs::metadata(path)?.len(),
                "{target} report byte count"
            );
        }
        assert_minimal_docker_source_artifacts(target, &destination, podman_target_version)?;
        assert_minimal_docker_read_only_acquisition(&server, target)?;
        for name in expected_files {
            fs::remove_file(destination.join(name))?;
        }
        fs::remove_dir(destination)?;
    }
    Ok(())
}

fn assert_minimal_docker_source_approximation(report: &serde_json::Value, target: &str) -> Result<(), Box<dyn Error>> {
    // The authored /web source name remains effective-state approximation;
    // preserving it in the target is no longer an unsupported omission.
    assert_eq!(report["fidelity"]["approximate"], 1, "{target}");
    assert_eq!(report["fidelity"]["unsupported"], 0, "{target}");
    assert_eq!(report["fidelity"]["invalid"], 0, "{target}");
    let diagnostics = report["diagnostics"].as_array().ok_or("source diagnostics missing")?;
    let approximations = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic["code"] == "BFD0004")
        .collect::<Vec<_>>();
    assert_eq!(approximations.len(), 1, "{target}");
    assert!(
        approximations[0]["fields"].as_array().is_some_and(|fields| {
            fields
                .iter()
                .any(|field| field["name"] == "subject" && field["value"] == "services[0].runtime_name")
        }),
        "{target}: missing source runtime-name approximation"
    );
    Ok(())
}

// Repository-authored semantic fixture: these are desired test values, not an
// inspect capture or renderer-generated expectation. The registry.invalid image
// is never pulled. No environment, label or host-bind consent is implied here.
fn rich_docker_source_inspect() -> serde_json::Value {
    serde_json::json!({
        "Id": FIRST,
        "Name": "/relay",
        "Config": {
            "Image": "registry.invalid/relay:7",
            "Cmd": ["serve", "--threads", "3"],
            "Entrypoint": ["/usr/local/bin/relay", "--foreground"],
            "User": "1001",
            "WorkingDir": "/srv/relay",
            "Healthcheck": {
                "Test": ["CMD", "/usr/local/bin/probe", "--ready"],
                "Interval": 3_000_000_000_u64,
                "Timeout": 2_000_000_000_u64,
                "StartPeriod": 5_000_000_000_u64,
                "Retries": 4
            }
        },
        "HostConfig": {
            "PortBindings": {
                "8080/tcp": [{"HostIp": "127.0.0.1", "HostPort": "18080"}],
                "9090/udp": [{"HostIp": "127.0.0.1", "HostPort": "19090"}]
            },
            "RestartPolicy": {"Name": "always", "MaximumRetryCount": 0},
            // Deliberately outside the bounded source mapping. A partial route
            // must report this exact loss, rather than claim a writable target exact.
            "ReadonlyRootfs": true
        }
    })
}

#[test]
fn rich_docker_to_compose_preserves_authored_semantics_and_reports_the_source_gap() -> Result<(), Box<dyn Error>> {
    rich_docker_document_journey("compose", "compose.yaml")
}

#[test]
fn rich_docker_to_quadlet_preserves_authored_semantics_and_reports_systemd_restart() -> Result<(), Box<dyn Error>> {
    rich_docker_document_journey("quadlet", "docker-service-1.container")
}

fn rich_docker_document_journey(target: &str, artifact: &str) -> Result<(), Box<dyn Error>> {
    let server = FakeDocker::new(
        serde_json::json!([]),
        BTreeMap::from([(FIRST, rich_docker_source_inspect())]),
    )?;
    let refused = run(
        &server,
        &["validate", "docker", target],
        &["--docker-container-id", FIRST, "--loss-policy", "approximate"],
    )?;
    assert_eq!(
        refused.status.code(),
        Some(2),
        "{target}: a source gap needs partial policy"
    );
    assert!(refused.stderr.is_empty());
    let refusal: serde_json::Value = serde_json::from_slice(&refused.stdout)?;
    assert_rich_docker_source_losses(&refusal, target)?;
    assert!(!server.root.join("first").exists());

    let mut previous = None;
    for directory in ["first", "second"] {
        let destination = server.root.join(directory);
        let result = Command::new(env!("CARGO_BIN_EXE_boxferry"))
            .args(["convert", "docker", target, "--docker-socket"])
            .arg(server.socket())
            .args([
                "--docker-container-id",
                FIRST,
                "--loss-policy",
                "partial",
                "--console-format",
                "json",
            ])
            .arg("--output-directory")
            .arg(&destination)
            .output()?;
        assert_eq!(
            result.status.code(),
            Some(0),
            "{target}: {}",
            String::from_utf8_lossy(&result.stdout)
        );
        assert!(result.stderr.is_empty());
        let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
        assert_eq!(report["status"], "success");
        assert_eq!(report["source_type"], "docker");
        assert_eq!(report["target_type"], target);
        assert_eq!(report["application"], "docker-import");
        assert_rich_docker_source_losses(&report, target)?;
        let files = fs::read_dir(&destination)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<io::Result<Vec<_>>>()?;
        assert_eq!(files, vec![std::ffi::OsString::from(artifact)]);
        let bytes = fs::read(destination.join(artifact))?;
        assert_eq!(
            report["output_artifacts"],
            serde_json::json!([{ "name": artifact, "size": bytes.len() }])
        );
        assert_rich_docker_document_semantics(target, artifact, std::str::from_utf8(&bytes)?)?;
        if let Some((old_bytes, old_report)) = &previous {
            assert_eq!(&bytes, old_bytes, "{target}: output depends on destination or run");
            assert_eq!(&report, old_report, "{target}: public report is not deterministic");
        } else {
            previous = Some((bytes, report));
        }
    }
    let inspect_path = format!("/v1.49/containers/{FIRST}/json");
    let expected = ["/version", "/v1.49/info", inspect_path.as_str()].repeat(3);
    let requests = server.requests()?;
    assert_eq!(
        requests.iter().map(|(_, path)| path.as_str()).collect::<Vec<_>>(),
        expected
    );
    assert!(requests.iter().all(|(method, _)| method == "GET"));
    assert_eq!(server.empty_connection_count(), 0);
    Ok(())
}

fn assert_rich_docker_source_losses(report: &serde_json::Value, target: &str) -> Result<(), Box<dyn Error>> {
    assert_eq!(report["fidelity"]["invalid"], 0);
    assert_eq!(
        report["fidelity"]["unsupported"],
        if target == "compose" { 2 } else { 1 }
    );
    assert_eq!(
        report["fidelity"]["approximate"],
        if target == "quadlet" { 2 } else { 1 }
    );
    assert_eq!(report["fidelity"]["other"], 0);
    let diagnostics = report["diagnostics"]
        .as_array()
        .ok_or("missing rich-source diagnostics")?;
    let mut warnings = Vec::new();
    for diagnostic in diagnostics
        .iter()
        .filter(|diagnostic| diagnostic["severity"] == "warning")
    {
        let subject = diagnostic["fields"]
            .as_array()
            .and_then(|fields| {
                fields
                    .iter()
                    .find(|field| field["name"] == "subject")
                    .and_then(|field| field["value"].as_str())
            })
            .ok_or("rich-source warning missing subject")?;
        warnings.push((diagnostic["code"].as_str().ok_or("warning code missing")?, subject));
    }
    warnings.sort_unstable();
    let mut expected = vec![
        ("BFD0002", "services[0].read_only_rootfs"),
        ("BFD0004", "services[0].runtime_name"),
    ];
    if target == "compose" {
        // The source reconstructs health intent, but the current Compose
        // generator boundary cannot emit it. This is an additional target loss.
        expected.insert(0, ("BFC0007", "services.docker-service-1.healthcheck"));
    } else {
        expected.push(("BFQ0009", "services.docker-service-1.restart_policy"));
    }
    assert_eq!(warnings, expected, "{target}: unreviewed loss or warning");
    for private in [FIRST, PRIVATE_NATIVE_CANARY, "engine.sock"] {
        assert!(!report.to_string().contains(private));
    }
    Ok(())
}

fn reimport_rich_docker_document(
    target: &str,
    artifact: &str,
    text: &str,
) -> Result<boxferry::ImportResult, Box<dyn Error>> {
    use boxferry::{Identifier, ImportAdapter};
    match target {
        "compose" => {
            use boxferry::compose::compose_lens::{
                loader::{DocumentInput, DocumentOrigin, LoadedProject},
                merge::merge_project,
                source::SourceId as NativeSourceId,
            };
            let loaded = LoadedProject::load([DocumentInput::new(
                NativeSourceId::new(1),
                DocumentOrigin::new(artifact, "."),
                text,
            )])?;
            let merged = merge_project(&loaded, None);
            let project = merged.project().ok_or("rich Compose project missing")?.clone();
            let source = boxferry::ComposeSource::new(project, Identifier::new("docker-import")?)?;
            Ok(boxferry::ComposeImporter::new()?.import(&source))
        }
        "quadlet" => {
            let source = boxferry::QuadletSource::parse(
                Identifier::new("docker-import")?,
                [boxferry::QuadletDocumentInput::new(
                    artifact,
                    boxferry::quadlet::quadlet_lens::source::SourceId::new(1),
                    text,
                )],
            )?
            .into_source();
            Ok(boxferry::QuadletImporter::new()?.import(&source))
        }
        _ => Err("unreviewed rich document target".into()),
    }
}

fn assert_rich_docker_document_semantics(target: &str, artifact: &str, text: &str) -> Result<(), Box<dyn Error>> {
    let imported = reimport_rich_docker_document(target, artifact, text)?;
    let application = imported.application().ok_or("reimported rich application missing")?;
    assert_eq!(application.name().as_str(), "docker-import");
    assert_eq!(application.services().len(), 1);
    assert!(application.service_groups().is_empty());
    assert!(application.networks().is_empty());
    assert!(application.volumes().is_empty());
    assert!(application.configs().is_empty());
    assert!(application.secrets().is_empty());
    assert!(application.image_acquisitions().is_empty());
    assert!(application.image_builds().is_empty());
    let service = application.services()[0].value();
    assert_eq!(service.name().as_str(), "docker-service-1");
    assert_eq!(
        service.runtime_name().map(|value| value.value().expose()),
        Some("relay")
    );
    assert_eq!(
        service.image().map(|value| value.value().as_str()),
        Some("registry.invalid/relay:7")
    );
    assert_rich_docker_execution(service)?;
    assert_rich_docker_ports(service);
    if target == "compose" {
        assert!(
            service.healthcheck().is_none(),
            "unreviewed Compose health generation escaped its declared loss"
        );
    } else {
        assert_rich_docker_healthcheck(service)?;
    }
    assert!(service.mounts().is_empty());
    assert!(service.networks().is_empty());
    assert!(service.environment().is_empty());
    assert!(service.environment_files().is_empty());
    assert!(service.dependencies().is_empty());
    assert!(
        service.read_only_root_filesystem().is_none(),
        "unsupported Docker rootfs state became authored intent"
    );
    assert!(!imported.outcomes().iter().any(|outcome| matches!(
        outcome.kind(),
        boxferry::ConversionKind::Unsupported | boxferry::ConversionKind::Invalid
    )));
    Ok(())
}

fn assert_rich_docker_execution(service: &boxferry::Service) -> Result<(), Box<dyn Error>> {
    use boxferry::{Command as NeutralCommand, Entrypoint, RestartPolicy};
    let Some(NeutralCommand::Exec(command)) = service.command().map(boxferry::Sourced::value) else {
        return Err("rich command lost exec form".into());
    };
    assert_eq!(
        command
            .iter()
            .map(boxferry::ProtectedString::expose)
            .collect::<Vec<_>>(),
        ["serve", "--threads", "3"]
    );
    let Some(Entrypoint::Exec(entrypoint)) = service.entrypoint().map(boxferry::Sourced::value) else {
        return Err("rich entrypoint lost exec form".into());
    };
    assert_eq!(
        entrypoint
            .iter()
            .map(boxferry::ProtectedString::expose)
            .collect::<Vec<_>>(),
        ["/usr/local/bin/relay", "--foreground"]
    );
    assert_eq!(service.user().map(|value| value.value().expose()), Some("1001"));
    assert_eq!(
        service.working_directory().map(|value| value.value().expose()),
        Some("/srv/relay")
    );
    assert_eq!(
        service.restart_policy().map(|value| *value.value()),
        Some(RestartPolicy::Always)
    );
    Ok(())
}

fn assert_rich_docker_ports(service: &boxferry::Service) {
    use boxferry::Protocol;
    assert_eq!(
        service
            .ports()
            .iter()
            .map(|value| {
                let port = value.value();
                (
                    port.container(),
                    port.published(),
                    port.host_address(),
                    port.protocol().clone(),
                )
            })
            .collect::<Vec<_>>(),
        [
            (8080, Some(18080), Some("127.0.0.1"), Protocol::Tcp),
            (9090, Some(19090), Some("127.0.0.1"), Protocol::Udp),
        ]
    );
}

fn assert_rich_docker_healthcheck(service: &boxferry::Service) -> Result<(), Box<dyn Error>> {
    use boxferry::HealthcheckCommand;
    let health = service.healthcheck().ok_or("rich healthcheck omitted")?.value();
    let Some(HealthcheckCommand::Exec(command)) = health.command().map(boxferry::Sourced::value) else {
        return Err("rich healthcheck lost exec form".into());
    };
    assert_eq!(
        command
            .iter()
            .map(boxferry::ProtectedString::expose)
            .collect::<Vec<_>>(),
        ["/usr/local/bin/probe", "--ready"]
    );
    assert_eq!(
        health.interval().map(|value| value.value().as_str()),
        Some("3000000000ns")
    );
    assert_eq!(
        health.timeout().map(|value| value.value().as_str()),
        Some("2000000000ns")
    );
    assert_eq!(
        health.start_period().map(|value| value.value().as_str()),
        Some("5000000000ns")
    );
    assert_eq!(health.retries().map(|value| value.value().as_str()), Some("4"));
    assert!(health.start_interval().is_none());
    assert!(!health.disabled().is_some_and(|value| *value.value()));
    Ok(())
}

fn assert_minimal_docker_read_only_acquisition(server: &FakeDocker, target: &str) -> Result<(), Box<dyn Error>> {
    let inspect_path = format!("/v1.49/containers/{FIRST}/json");
    let requests = server.requests()?;
    assert!(!requests.is_empty(), "{target} must acquire its source");
    for (method, path) in &requests {
        assert_eq!(method, "GET", "{target}: {path}");
        assert!(
            matches!(
                path.as_str(),
                "/version" | "/v1.49/info" | "/v1.49/containers/json?all=1"
            ) || path == &inspect_path,
            "{target}: unexpected runtime request {method} {path}"
        );
    }
    assert_eq!(
        requests.iter().filter(|(_, path)| path == &inspect_path).count(),
        1,
        "{target}"
    );
    assert_eq!(
        requests
            .iter()
            .filter(|(_, path)| path == "/v1.49/containers/json?all=1")
            .count(),
        1,
        "{target}"
    );
    Ok(())
}

fn assert_minimal_docker_source_artifacts(
    target: &str,
    destination: &Path,
    podman_target_version: &str,
) -> Result<(), Box<dyn Error>> {
    match target {
        "compose" => {
            let document = fs::read_to_string(destination.join("compose.yaml"))?;
            for line in [
                "---",
                "name: docker-import",
                "services:",
                "  docker-service-1:",
                "    container_name: web",
                "    image: docker.io/library/busybox:1",
            ] {
                assert!(
                    document.lines().any(|actual| actual == line),
                    "missing Compose line: {line}"
                );
            }
        }
        "quadlet" => {
            let document = fs::read_to_string(destination.join("docker-service-1.container"))?;
            for line in ["[Container]", "Image=docker.io/library/busybox:1", "ContainerName=web"] {
                assert!(
                    document.lines().any(|actual| actual == line),
                    "missing Quadlet line: {line}"
                );
            }
        }
        "podman" => assert_minimal_docker_source_podman_plan(destination, podman_target_version)?,
        "docker" => {
            let path = destination.join("docker-plan.json");
            let plan: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
            assert_eq!(plan["schema_version"], 1);
            assert_eq!(plan["context"]["kind"], "target");
            assert_eq!(plan["context"]["engine_release"], "29.8.1");
            assert_eq!(plan["context"]["daemon_mode"], "rootful");
            assert_eq!(plan["context"]["rendering_api_version"], "1.56");
            assert_eq!(plan["prerequisites"], serde_json::json!([]));
            let requests = plan["requests"].as_array().ok_or("Docker create requests missing")?;
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0]["method"], "POST");
            assert_eq!(requests[0]["path"], "/v1.56/containers/create?name=web");
            assert_eq!(requests[0]["body"]["Image"], "docker.io/library/busybox:1");
            assert_eq!(fs::metadata(destination)?.mode() & 0o777, 0o700);
            assert_eq!(fs::metadata(path)?.mode() & 0o777, 0o600);
        }
        _ => return Err(format!("unreviewed test target: {target}").into()),
    }
    Ok(())
}

fn assert_minimal_docker_source_podman_plan(destination: &Path, target_version: &str) -> Result<(), Box<dyn Error>> {
    let plan: serde_json::Value = serde_json::from_slice(&fs::read(destination.join("podman.json"))?)?;
    assert_eq!(plan["schema_version"], 1);
    assert!(plan["connection"].is_null());
    assert_eq!(plan["external_preconditions"], serde_json::json!([]));
    let operations = plan["operations"].as_array().ok_or("Podman operations missing")?;
    assert_eq!(
        operations
            .iter()
            .map(|operation| operation["action"].as_str())
            .collect::<Vec<_>>(),
        vec![Some("ensure_image"), Some("create"), Some("start_container")]
    );
    // The logical service remains docker-service-1; the authored inspect Name
    // /web independently supplies the native create/start container identity.
    assert_eq!(
        operations[0]["resource"],
        serde_json::json!({"kind": "image", "name": "docker-service-1-image"})
    );
    let create = &operations[1];
    assert_eq!(
        create["resource"],
        serde_json::json!({"kind": "container", "name": "web"})
    );
    assert_eq!(create["cli"]["program"], "podman");
    assert_eq!(
        create["cli"]["argv"],
        serde_json::json!([
            "container",
            "create",
            "--name",
            "web",
            "--pull=never",
            "docker.io/library/busybox:1"
        ])
    );
    assert_eq!(create["libpod"]["method"], "POST");
    assert_eq!(
        create["libpod"]["path_and_query"],
        format!("/v{target_version}/libpod/containers/create?name=web")
    );
    assert_eq!(
        create["libpod"]["body"],
        serde_json::json!({"kind": "json", "json": {"Networks": {}, "image": "docker.io/library/busybox:1"}})
    );
    let start = &operations[2];
    assert_eq!(
        start["resource"],
        serde_json::json!({"kind": "container", "name": "web"})
    );
    assert_eq!(start["cli"]["program"], "podman");
    assert_eq!(start["cli"]["argv"], serde_json::json!(["container", "start", "web"]));
    assert_eq!(start["libpod"]["method"], "POST");
    assert_eq!(
        start["libpod"]["path_and_query"],
        format!("/v{target_version}/libpod/containers/web/start")
    );
    assert_eq!(start["libpod"]["body"], serde_json::json!({"kind": "empty"}));
    let review = fs::read_to_string(destination.join("podman-commands.sh"))?;
    assert!(review.starts_with("#!/bin/sh\n"));
    assert!(review.contains("# Review generated Podman commands before running this file.\n"));
    assert_eq!(
        review.lines().filter(|line| line.starts_with("podman ")).count(),
        operations.len()
    );
    assert!(review.contains("'container' 'create' '--name' 'web'"));
    assert!(review.contains("'docker.io/library/busybox:1'"));
    assert!(review.contains("'container' 'start' 'web'"));
    Ok(())
}

#[test]
fn fully_qualified_docker_image_can_plan_for_podman_without_runtime_pull() -> Result<(), Box<dyn Error>> {
    let inspected = serde_json::json!({
        "Id": FIRST,
        "Name": "/web",
        "Config": {"Image": "docker.io/library/busybox:1"},
    });
    let server = FakeDocker::new(
        serde_json::json!([summary(FIRST, "web", serde_json::json!({}))]),
        BTreeMap::from([(FIRST, inspected)]),
    )?;
    let result = run(
        &server,
        &["validate", "docker", "podman"],
        &[
            "--podman-target-context",
            "unknown",
            "--podman-max-version",
            "6.1.0",
            "--loss-policy",
            "approximate",
        ],
    )?;
    assert_eq!(
        result.status.code(),
        Some(0),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(report["source_type"], "docker");
    assert_eq!(report["target_type"], "podman");
    assert_eq!(report["status"], "success");
    assert_minimal_docker_source_approximation(&report, "podman")?;
    assert!(server.paths()?.iter().all(|path| !path.contains("/images/create")));
    Ok(())
}

#[test]
fn docker_no_match_and_conflicting_selectors_do_not_inspect_containers() -> Result<(), Box<dyn Error>> {
    let empty = FakeDocker::new(serde_json::json!([]), BTreeMap::new())?;
    let result = run(
        &empty,
        &["validate", "docker", "compose"],
        &["--loss-policy", "partial"],
    )?;
    assert_eq!(result.status.code(), Some(1));
    assert!(
        empty
            .paths()?
            .iter()
            .all(|path| !path.contains("/containers/") || path.contains("/json?all=1"))
    );

    let server = FakeDocker::new(
        serde_json::json!([summary(FIRST, "web", serde_json::json!({}))]),
        BTreeMap::from([(FIRST, inspect(FIRST, "web"))]),
    )?;
    let result = run(
        &server,
        &["validate", "docker", "compose"],
        &["--docker-container", "web", "--docker-container-id", FIRST],
    )?;
    assert_eq!(result.status.code(), Some(1));
    assert!(server.paths()?.is_empty());
    Ok(())
}

struct DockerOutputCollisionDirectory(PathBuf);

impl Drop for DockerOutputCollisionDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn docker_output_collision_directory() -> Result<DockerOutputCollisionDirectory, Box<dyn Error>> {
    let path = std::env::temp_dir().join(format!(
        "boxferry-docker-output-collision-{}-{}-{}",
        std::process::id(),
        NEXT_CASE.fetch_add(1, Ordering::Relaxed),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos(),
    ));
    // Only install the cleanup guard after atomically creating this exact private path.
    fs::DirBuilder::new().mode(0o700).create(&path)?;
    Ok(DockerOutputCollisionDirectory(path))
}

#[test]
fn compose_to_docker_output_collision_preserves_existing_bytes_without_runtime() -> Result<(), Box<dyn Error>> {
    let root = docker_output_collision_directory()?;
    let source = root.0.join("compose.yaml");
    // Repository-authored offline intent; the synthetic image is never pulled or executed.
    fs::write(&source, b"---\nname: output-collision\nservices:\n  web:\n    image: example.invalid/offline:1\n    container_name: output-collision-web\n")?;
    let entries = |directory: &Path| -> io::Result<Vec<std::ffi::OsString>> {
        let mut names = fs::read_dir(directory)?
            .map(|entry| entry.map(|entry| entry.file_name()))
            .collect::<io::Result<Vec<_>>>()?;
        names.sort();
        Ok(names)
    };
    let convert = |destination: &Path| -> io::Result<Output> {
        Command::new(env!("CARGO_BIN_EXE_boxferry"))
            .current_dir(&root.0)
            .args(["convert", "compose", "docker", "--input-file"])
            .arg(&source)
            .args(["--docker-target", "upstream-29.8.1-rootful", "--console-format", "json"])
            .arg("--output-directory")
            .arg(destination)
            .output()
    };
    for with_native_names in [false, true] {
        let destination = root.0.join(if with_native_names {
            "occupied-plan"
        } else {
            "occupied-sentinel"
        });
        fs::DirBuilder::new().mode(0o700).create(&destination)?;
        let mut original = BTreeMap::from([("sentinel.txt", b"unrelated original bytes\n".as_slice())]);
        if with_native_names {
            original.insert("docker-plan.json", b"original native plan bytes\n".as_slice());
            original.insert("docker-dependencies.json", b"original sidecar bytes\n".as_slice());
        }
        for (name, bytes) in &original {
            fs::write(destination.join(name), bytes)?;
        }
        let parent_before = entries(&root.0)?;
        let result = convert(&destination)?;
        assert_eq!(
            result.status.code(),
            Some(1),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
        assert!(
            report["diagnostics"]
                .as_array()
                .ok_or("collision diagnostics missing")?
                .iter()
                .any(|diagnostic| diagnostic["code"] == "BFO2001")
        );
        let expected_names = original
            .keys()
            .map(|name| std::ffi::OsString::from(*name))
            .collect::<Vec<_>>();
        assert_eq!(
            entries(&destination)?,
            expected_names,
            "collision emitted an unexpected artifact"
        );
        for (name, bytes) in original {
            let original_path = destination.join(name);
            assert!(fs::symlink_metadata(&original_path)?.file_type().is_file());
            assert_eq!(fs::read(original_path)?, bytes, "collision changed {name}");
        }
        assert_eq!(
            entries(&root.0)?,
            parent_before,
            "collision emitted an unexpected sibling"
        );
    }
    for existing_empty in [false, true] {
        let destination = root
            .0
            .join(if existing_empty { "empty-control" } else { "new-control" });
        if existing_empty {
            fs::DirBuilder::new().mode(0o700).create(&destination)?;
        }
        let result = convert(&destination)?;
        assert!(
            result.status.success(),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            entries(&destination)?,
            vec![std::ffi::OsString::from("docker-plan.json")]
        );
        assert!(!fs::read(destination.join("docker-plan.json"))?.is_empty());
    }
    Ok(())
}

#[test]
fn docker_output_requires_an_exact_target_before_acquisition() -> Result<(), Box<dyn Error>> {
    let server = FakeDocker::new(
        serde_json::json!([summary(FIRST, "web", serde_json::json!({}))]),
        BTreeMap::from([(FIRST, inspect(FIRST, "web"))]),
    )?;
    let result = run(
        &server,
        &["validate", "docker", "docker"],
        &["--loss-policy", "partial"],
    )?;
    assert_eq!(result.status.code(), Some(2));
    assert!(server.paths()?.is_empty());
    Ok(())
}

#[test]
fn explicit_local_docker_socket_ignores_ambient_remote_endpoint() -> Result<(), Box<dyn Error>> {
    let server = FakeDocker::new(
        serde_json::json!([summary(FIRST, "web", serde_json::json!({}))]),
        BTreeMap::from([(FIRST, inspect(FIRST, "web"))]),
    )?;
    let result = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["validate", "docker", "compose", "--docker-socket"])
        .arg(server.socket())
        .args(["--loss-policy", "partial", "--console-format", "json"])
        .env("DOCKER_HOST", "tcp://remote.invalid:2375")
        .output()?;
    assert_eq!(result.status.code(), Some(0));
    assert!(server.paths()?.iter().any(|path| path == "/version"));
    assert!(!String::from_utf8_lossy(&result.stdout).contains("remote.invalid"));
    Ok(())
}

#[test]
fn same_host_bind_promotion_is_independent_and_reports_remain_path_safe() -> Result<(), Box<dyn Error>> {
    for (mode_name, mode) in [("default-mode", ""), ("explicit-rw", "rw")] {
        let inspected = serde_json::json!({
            "Id": FIRST,
            "Name": "/web",
            "Config": {"Image": "busybox:1"},
            "Mounts": [{"Type": "bind", "Source": "/private/canary-host-path", "Destination": "/data", "RW": true, "Mode": mode}],
        });
        let server = FakeDocker::new(
            serde_json::json!([summary(FIRST, "web", serde_json::json!({}))]),
            BTreeMap::from([(FIRST, inspected)]),
        )?;
        for (name, promote, emitted) in [("withheld-bind", false, false), ("same-host-bind", true, true)] {
            let destination = server.root.join(format!("{mode_name}-{name}"));
            let mut command = Command::new(env!("CARGO_BIN_EXE_boxferry"));
            command
                .args(["convert", "docker", "compose", "--docker-socket"])
                .arg(server.socket())
                .args(["--loss-policy", "partial"]);
            if promote {
                command.arg("--promote-docker-same-host-bind-mounts");
            }
            let result = command
                .arg("--output-directory")
                .arg(&destination)
                .args(["--console-format", "json"])
                .output()?;
            assert_eq!(
                result.status.code(),
                Some(0),
                "{name}: {}",
                String::from_utf8_lossy(&result.stdout)
            );
            assert!(!String::from_utf8_lossy(&result.stdout).contains("/private/canary-host-path"));
            let output = fs::read(destination.join("compose.yaml"))?;
            assert_eq!(
                String::from_utf8_lossy(&output).contains("/private/canary-host-path"),
                emitted,
                "{name}"
            );
            fs::remove_file(destination.join("compose.yaml"))?;
            fs::remove_dir(destination)?;
        }
    }
    Ok(())
}

#[test]
fn same_host_bind_permission_cannot_promote_missing_native_mode_evidence() -> Result<(), Box<dyn Error>> {
    // Same-host consent authorizes the source domain, not an invented mode.
    // RW alone does not establish the absence of configured z/Z relabel intent.
    let inspected = serde_json::json!({
        "Id": FIRST,
        "Name": "/web",
        "Config": {"Image": "busybox:1"},
        "Mounts": [{"Type":"bind", "Source":"/private/canary-host-path", "Destination":"/data", "RW":true}],
    });
    let server = FakeDocker::new(
        serde_json::json!([summary(FIRST, "web", serde_json::json!({}))]),
        BTreeMap::from([(FIRST, inspected)]),
    )?;
    let destination = server.root.join("same-host-missing-mode");
    let result = Command::new(env!("CARGO_BIN_EXE_boxferry"))
        .args(["convert", "docker", "compose", "--docker-socket"])
        .arg(server.socket())
        .args([
            "--loss-policy",
            "partial",
            "--promote-docker-same-host-bind-mounts",
            "--console-format",
            "json",
        ])
        .arg("--output-directory")
        .arg(&destination)
        .output()?;
    assert_eq!(result.status.code(), Some(0));
    assert!(result.stderr.is_empty());
    let report: serde_json::Value = serde_json::from_slice(&result.stdout)?;
    assert_eq!(report["status"], "success");
    assert!(
        report["fidelity"]["unsupported"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
    assert!(
        report["diagnostics"]
            .as_array()
            .ok_or("missing-mode diagnostics missing")?
            .iter()
            .any(|diagnostic| {
                diagnostic["code"] == "BFD0002"
                    && diagnostic["fields"].as_array().is_some_and(|fields| {
                        fields
                            .iter()
                            .any(|field| field["name"] == "subject" && field["value"] == "services[0].mounts[0].mode")
                    })
            })
    );
    assert!(!String::from_utf8_lossy(&result.stdout).contains("/private/canary-host-path"));
    let output = fs::read(destination.join("compose.yaml"))?;
    assert!(!String::from_utf8_lossy(&output).contains("/private/canary-host-path"));
    assert!(
        !String::from_utf8_lossy(&output).contains("/data"),
        "unknown mode must not produce an invented bind mount"
    );
    fs::remove_file(destination.join("compose.yaml"))?;
    fs::remove_dir(destination)?;
    Ok(())
}
