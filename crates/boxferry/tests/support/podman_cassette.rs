//! Exact-order Unix HTTP replay for the checked-in Podman cassette corpus.

use std::{
    error::Error,
    fs,
    io::{self, Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use serde::Deserialize;

static SOCKET_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct PodmanCassette {
    scenario_id: String,
    engine_version: String,
    execution_context: String,
    interactions: Vec<Interaction>,
}

impl PodmanCassette {
    pub(crate) fn load(path: &Path) -> Result<Self, Box<dyn Error>> {
        Ok(serde_json::from_slice(&fs::read(path)?)?)
    }

    pub(crate) fn scenario_id(&self) -> &str {
        &self.scenario_id
    }

    pub(crate) fn engine_version(&self) -> &str {
        &self.engine_version
    }

    pub(crate) fn execution_context(&self) -> &str {
        &self.execution_context
    }

    pub(crate) fn interaction_count(&self) -> usize {
        self.interactions.len()
    }

    pub(crate) fn set_status(&mut self, path: &str, status: u16) -> Result<(), Box<dyn Error>> {
        self.unique_interaction_mut(path)?.response.status = status;
        Ok(())
    }

    pub(crate) fn set_body(&mut self, path: &str, body: serde_json::Value) -> Result<(), Box<dyn Error>> {
        self.unique_interaction_mut(path)?.response.body = Some(body);
        Ok(())
    }

    pub(crate) fn insert_body_field(
        &mut self,
        path: &str,
        name: &str,
        value: serde_json::Value,
    ) -> Result<(), Box<dyn Error>> {
        let body = self
            .unique_interaction_mut(path)?
            .response
            .body
            .as_mut()
            .ok_or_else(|| format!("{path} has no JSON response body"))?;
        body.as_object_mut()
            .ok_or_else(|| format!("{path} response body is not an object"))?
            .insert(name.to_owned(), value);
        Ok(())
    }

    fn unique_interaction_mut(&mut self, path: &str) -> Result<&mut Interaction, Box<dyn Error>> {
        let matches = self
            .interactions
            .iter()
            .enumerate()
            .filter_map(|(index, interaction)| (interaction.request.path == path).then_some(index))
            .collect::<Vec<_>>();
        let [index] = matches.as_slice() else {
            return Err(format!(
                "expected exactly one cassette interaction for {path}, found {}",
                matches.len()
            )
            .into());
        };
        Ok(&mut self.interactions[*index])
    }
}

#[derive(Clone, Debug, Deserialize)]
struct Interaction {
    request: RecordedRequest,
    response: RecordedResponse,
}

#[derive(Clone, Debug, Deserialize)]
struct RecordedRequest {
    method: String,
    path: String,
}

#[derive(Clone, Debug, Deserialize)]
struct RecordedResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Option<serde_json::Value>,
}

pub(crate) struct PodmanCassetteServer {
    socket: PathBuf,
    shutdown: Arc<AtomicBool>,
    thread: Option<JoinHandle<Result<(), String>>>,
}

impl PodmanCassetteServer {
    pub(crate) fn start(cassette: PodmanCassette) -> Result<Self, Box<dyn Error>> {
        let id = SOCKET_ID.fetch_add(1, Ordering::Relaxed);
        let socket = std::env::temp_dir().join(format!("bfc-{}-{id}.sock", std::process::id()));
        if socket.exists() {
            fs::remove_file(&socket)?;
        }
        let listener = UnixListener::bind(&socket)?;
        listener.set_nonblocking(true)?;
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = Arc::clone(&shutdown);
        let scenario = cassette.scenario_id.clone();
        let thread = thread::spawn(move || replay(&listener, &scenario, cassette.interactions, &thread_shutdown));
        Ok(Self {
            socket,
            shutdown,
            thread: Some(thread),
        })
    }

    pub(crate) fn socket(&self) -> &Path {
        &self.socket
    }

    pub(crate) fn finish(mut self) -> Result<(), Box<dyn Error>> {
        self.shutdown.store(true, Ordering::Release);
        let result = self
            .thread
            .take()
            .ok_or("cassette replay thread missing")?
            .join()
            .map_err(|_| "cassette replay thread panicked")?;
        let _ = fs::remove_file(&self.socket);
        result.map_err(Into::into)
    }
}

impl Drop for PodmanCassetteServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = fs::remove_file(&self.socket);
    }
}

fn replay(
    listener: &UnixListener,
    scenario: &str,
    interactions: Vec<Interaction>,
    shutdown: &AtomicBool,
) -> Result<(), String> {
    let total = interactions.len();
    for (index, interaction) in interactions.into_iter().enumerate() {
        let mut stream = accept(listener, shutdown).map_err(|error| {
            format!("{scenario}: consumed {index}/{total} interactions before replay stopped: {error}")
        })?;
        let (method, path) = read_request(&mut stream).map_err(|error| {
            format!(
                "{scenario}: interaction {} request could not be read: {error}",
                index + 1
            )
        })?;
        if method != interaction.request.method || path != interaction.request.path {
            let _ = write_error(&mut stream);
            return Err(format!(
                "{scenario}: interaction {} expected {} {}, received {method} {path}",
                index + 1,
                interaction.request.method,
                interaction.request.path
            ));
        }
        write_response(&mut stream, &interaction.response).map_err(|error| {
            format!(
                "{scenario}: interaction {} response could not be written: {error}",
                index + 1
            )
        })?;
    }
    Ok(())
}

fn accept(listener: &UnixListener, shutdown: &AtomicBool) -> io::Result<UnixStream> {
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false)?;
                return Ok(stream);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if shutdown.load(Ordering::Acquire) {
                    return Err(io::Error::new(io::ErrorKind::Interrupted, "replay was finished"));
                }
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) => return Err(error),
        }
    }
}

fn read_request(stream: &mut UnixStream) -> io::Result<(String, String)> {
    const MAXIMUM_HEADER_BYTES: usize = 32 * 1024;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut request = Vec::new();
    let mut byte = [0_u8; 1];
    while !request.ends_with(b"\r\n\r\n") {
        if request.len() == MAXIMUM_HEADER_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP request headers exceed test limit",
            ));
        }
        stream.read_exact(&mut byte)?;
        request.push(byte[0]);
    }
    let request = std::str::from_utf8(&request).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let request_line = request
        .split("\r\n")
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "HTTP request line missing"))?;
    let mut fields = request_line.split_whitespace();
    let method = fields
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "HTTP request method missing"))?;
    let path = fields
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "HTTP request path missing"))?;
    let version = fields
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "HTTP version missing"))?;
    if fields.next().is_some() || !matches!(version, "HTTP/1.0" | "HTTP/1.1") {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid HTTP request line"));
    }
    Ok((method.to_owned(), path.to_owned()))
}

fn write_response(stream: &mut UnixStream, response: &RecordedResponse) -> io::Result<()> {
    let body = response
        .body
        .as_ref()
        .map(serde_json::to_vec)
        .transpose()?
        .unwrap_or_default();
    write!(stream, "HTTP/1.1 {} {}\r\n", response.status, reason(response.status))?;
    for (name, value) in &response.headers {
        if !name.eq_ignore_ascii_case("connection") && !name.eq_ignore_ascii_case("content-length") {
            write!(stream, "{name}: {value}\r\n")?;
        }
    }
    write!(stream, "Content-Length: {}\r\nConnection: close\r\n\r\n", body.len())?;
    stream.write_all(&body)?;
    stream.flush()
}

fn write_error(stream: &mut UnixStream) -> io::Result<()> {
    stream.write_all(b"HTTP/1.1 409 Conflict\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
}

const fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        409 => "Conflict",
        500 => "Internal Server Error",
        _ => "Recorded",
    }
}
