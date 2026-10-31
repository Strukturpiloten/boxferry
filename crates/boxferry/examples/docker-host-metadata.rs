//! Test-only read-only daemon metadata probe; never discovers or selects resources.
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use boxferry::docker::docker_lens::{acquisition, decoder, evidence, version};

fn metadata(socket: std::path::PathBuf, elapsed: Duration, cancelled: &AtomicBool) -> Result<String, ()> {
    let capture = acquisition::acquire(
        &acquisition::Endpoint::unix_socket(socket),
        acquisition::Selector::ContainerIds(Vec::new()),
        acquisition::Limits {
            max_requests: 2,
            max_selected_resources: 1,
            max_expansions: 1,
            max_response_bytes: 65_536,
            max_total_bytes: 131_072,
            max_elapsed: elapsed,
        },
        cancelled,
    )
    .map_err(|_| ())?;
    let exchanges = capture.exchanges();
    let bounds = capture.bounds();
    if capture.route() != evidence::CaptureRoute::ExplicitUnixSocket
        || bounds.request_count != 2
        || bounds.selected_resources != 0
        || bounds.expansions != 0
        || !capture.selected_roots().is_empty()
        || exchanges.len() != 2
        || !matches!(exchanges[0].request(), acquisition::ReadRequest::DaemonVersion)
        || !matches!(exchanges[1].request(), acquisition::ReadRequest::DaemonInfo)
        || exchanges.iter().any(|exchange| exchange.status().code() != 200)
    {
        return Err(());
    }
    let decoded = decoder::decode_capture(&capture).map_err(|_| ())?;
    if !decoded.containers.is_empty()
        || !decoded.discovered_containers.is_empty()
        || !decoded.networks.is_empty()
        || !decoded.volumes.is_empty()
        || !decoded.selected_roots.is_empty()
    {
        return Err(());
    }
    let daemon = decoded.version.daemon;
    let release = daemon.release.ok_or(())?;
    let release = release.as_str();
    // Only finite, value-free release tokens may leave the protected decoder.
    let components: Vec<_> = release.strip_suffix("+dfsg1").unwrap_or(release).split('.').collect();
    if components.len() != 3
        || components.iter().any(|component| {
            component.is_empty() || component.len() > 3 || !component.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return Err(());
    }
    let advertised = daemon.api_version.ok_or(())?;
    let negotiated = exchanges[1].api_version().ok_or(())?;
    if advertised.major.get() != 1
        || !(40..=99).contains(&advertised.minor)
        || negotiated.major.get() != 1
        || !(41..=49).contains(&negotiated.minor)
    {
        return Err(());
    }
    let mode = match daemon.mode {
        version::DaemonMode::Rootful => "rootful",
        version::DaemonMode::Rootless => "rootless",
        version::DaemonMode::Unknown => "unknown",
    };
    Ok(format!(
        "{release} 1.{} 1.{} {mode}",
        advertised.minor, negotiated.minor
    ))
}

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let result = (|| {
        if args.len() != 4 || args[0] != "--socket" || args[2] != "--timeout-ms" {
            return Err(());
        }
        let milliseconds = args[3].to_str().ok_or(())?.parse::<u64>().map_err(|_| ())?;
        if !(1..=4000).contains(&milliseconds) {
            return Err(());
        }
        metadata(
            args[1].clone().into(),
            Duration::from_millis(milliseconds),
            &AtomicBool::new(false),
        )
    })();
    if let Ok(record) = result {
        println!("{record}");
    } else {
        eprintln!("host metadata unavailable");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixListener;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;

    type ProbeResult = (Result<String, ()>, Vec<String>, Duration);

    fn probe(version: &str, info: &str) -> std::io::Result<(Result<String, ()>, Vec<String>)> {
        let (result, paths, _) = probe_with(version, info, 200, false, false, false)?;
        Ok((result, paths))
    }

    fn probe_with(
        version: &str,
        info: &str,
        info_status: u16,
        stall: bool,
        pre_cancel: bool,
        cancel_after_version: bool,
    ) -> std::io::Result<ProbeResult> {
        let path = std::env::temp_dir().join(format!(
            "bf-metadata-{}-{:?}.sock",
            std::process::id(),
            std::thread::current().id()
        ));
        let listener = UnixListener::bind(&path)?;
        let version = version.to_owned();
        let info = info.to_owned();
        let cancelled = Arc::new(AtomicBool::new(pre_cancel));
        let server_cancelled = Arc::clone(&cancelled);
        let server = std::thread::spawn(move || {
            let mut paths = Vec::new();
            listener.set_nonblocking(true)?;
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            for (index, body) in [version, info].into_iter().enumerate() {
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            if std::time::Instant::now() >= deadline {
                                return Ok(paths);
                            }
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => return Err(error),
                    }
                };
                stream.set_read_timeout(Some(Duration::from_secs(1)))?;
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    if stream.read(&mut byte)? == 0 {
                        break;
                    }
                    request.push(byte[0]);
                }
                paths.push(
                    String::from_utf8(request)
                        .map_err(std::io::Error::other)?
                        .lines()
                        .next()
                        .ok_or_else(|| std::io::Error::other("request line absent"))?
                        .to_owned(),
                );
                if stall {
                    std::thread::sleep(Duration::from_millis(600));
                }
                if cancel_after_version && index == 0 {
                    server_cancelled.store(true, Ordering::Relaxed);
                }
                let status = if index == 0 { 200 } else { info_status };
                if write!(
                    stream,
                    "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .is_err()
                {
                    return Ok(paths);
                }
            }
            Ok(paths)
        });
        let start = std::time::Instant::now();
        let result = metadata(path.clone(), Duration::from_millis(500), &cancelled);
        let elapsed = start.elapsed();
        let joined = server.join();
        let cleanup = std::fs::remove_file(path);
        let paths = joined.map_err(|_| std::io::Error::other("test server failed"))??;
        cleanup?;
        Ok((result, paths, elapsed))
    }

    #[test]
    fn exact_empty_selector_reads_only_metadata_and_preserves_unknown() -> std::io::Result<()> {
        let (result, paths) = probe(
            r#"{"Version":"29.8.1","ApiVersion":"1.56","MinAPIVersion":"1.40"}"#,
            r#"{"ServerVersion":"29.8.1","SecurityOptions":[]}"#,
        )?;
        assert_eq!(result, Ok("29.8.1 1.56 1.49 unknown".into()));
        assert_eq!(paths, ["GET /version HTTP/1.1", "GET /v1.49/info HTTP/1.1"]);
        Ok(())
    }

    #[test]
    fn explicit_rootless_is_retained() -> std::io::Result<()> {
        assert_eq!(
            probe(
                r#"{"Version":"20.10.5+dfsg1","ApiVersion":"1.41"}"#,
                r#"{"SecurityOptions":["name=rootless"]}"#
            )?
            .0,
            Ok("20.10.5+dfsg1 1.41 1.41 rootless".into())
        );
        Ok(())
    }

    #[test]
    fn malformed_unsupported_or_oversized_metadata_fails() -> std::io::Result<()> {
        for version in [
            "invalid".to_owned(),
            r#"{"Version":"private value","ApiVersion":"1.56"}"#.into(),
            r#"{"Version":"29.8.1","ApiVersion":"1.56","MinAPIVersion":"1.50"}"#.into(),
            " ".repeat(65_537),
        ] {
            assert!(probe(&version, "{}")?.0.is_err());
        }
        assert!(
            probe(r#"{"Version":"29.8.1","ApiVersion":"1.56"}"#, "invalid")?
                .0
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn cancellation_and_zero_deadline_fail_before_connection() {
        let cancelled = AtomicBool::new(false);
        cancelled.store(true, Ordering::Relaxed);
        assert!(metadata("/not-an-endpoint".into(), Duration::from_millis(100), &cancelled).is_err());
        assert!(metadata("/not-an-endpoint".into(), Duration::ZERO, &AtomicBool::new(false)).is_err());
    }

    #[test]
    fn cancellation_prevents_requests_or_stops_before_info() -> std::io::Result<()> {
        let version = r#"{"Version":"29.8.1","ApiVersion":"1.56"}"#;
        let (result, paths, _) = probe_with(version, "{}", 200, false, true, false)?;
        assert!(result.is_err());
        assert!(paths.is_empty());
        let (result, paths, _) = probe_with(version, "{}", 200, false, false, true)?;
        assert!(result.is_err());
        assert_eq!(paths, ["GET /version HTTP/1.1"]);
        Ok(())
    }

    #[test]
    fn stalled_response_and_non_success_info_never_emit_partial_metadata() -> std::io::Result<()> {
        let version = r#"{"Version":"29.8.1","ApiVersion":"1.56"}"#;
        let (result, paths, elapsed) = probe_with(version, "{}", 200, true, false, false)?;
        assert!(result.is_err());
        assert_eq!(paths, ["GET /version HTTP/1.1"]);
        assert!(elapsed < Duration::from_millis(750));
        let (result, paths, _) = probe_with(version, "{}", 503, false, false, false)?;
        assert!(result.is_err());
        assert_eq!(paths, ["GET /version HTTP/1.1", "GET /v1.49/info HTTP/1.1"]);
        Ok(())
    }
}
