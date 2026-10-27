//! Independent framing, exact bytes, bounds and create-new extraction contracts.
#![cfg(feature = "quadlet")]
use boxferry::quadlet_bundle::*;
use std::{
    error::Error,
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};
fn frame(members: &[(&str, &[u8])]) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut bytes = format!("BOXFERRY-QUADLET-BUNDLE 1\nmembers {}\n", members.len()).into_bytes();
    for (name, text) in members {
        bytes.extend_from_slice(
            format!(
                "---\n{{\"name\":{},\"bytes\":{}}}\n",
                serde_json::to_string(name)?,
                text.len()
            )
            .as_bytes(),
        );
        bytes.extend_from_slice(text);
        bytes.push(b'\n');
    }
    Ok(bytes)
}
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self, Box<dyn Error>> {
        let path = std::env::temp_dir().join(format!(
            "boxferry-transport-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
#[test]
fn canonical_codec_retains_order_unicode_separators_crlf_and_final_newlines() -> Result<(), Box<dyn Error>> {
    let members=[("web.container","[Container]\r\nImage=example.invalid/web:1\r\nVolume=data.volume:/data\r\n# λ\n---\n{\"name\":\"fake.container\",\"bytes\":123}\n".as_bytes()),("data.volume",b"[Volume]".as_slice()),("front.network",b"[Network]\n".as_slice()),("group.pod",b"[Pod]\n".as_slice()),("app.image",b"[Image]\n".as_slice()),("app.build",b"".as_slice())];
    let encoded = frame(&members)?;
    let decoded = QuadletBundle::decode(&encoded)?;
    assert_eq!(decoded.members().len(), members.len());
    for (actual, (name, text)) in decoded.members().iter().zip(members) {
        assert_eq!(actual.name(), name);
        assert_eq!(actual.text().as_bytes(), text);
    }
    assert_eq!(decoded.encode()?, encoded);
    assert_eq!(
        decoded.members()[0].unit_type(),
        boxferry::quadlet::quadlet_lens::model::QuadletUnitType::Container
    );
    Ok(())
}
#[test]
fn versions_counts_headers_lengths_and_trailers_are_strict() -> Result<(), Box<dyn Error>> {
    let valid = frame(&[("web.container", b"abc")])?;
    for (from, to) in [
        ("BUNDLE 1", "BUNDLE 2"),
        ("members 1", "members 0"),
        ("members 1", "members 4097"),
        ("members 1", "members 01"),
        ("members 1", "members +1"),
        ("members 1", "members -1"),
        ("members 1", "members 1.0"),
        ("members 1", "members 9999999999999999999999999"),
        ("---\n", "--\n"),
        ("\"bytes\":3", "\"bytes\":2"),
        ("\"bytes\":3", "\"bytes\":4"),
        ("\"bytes\":3", "\"bytes\":03"),
        ("\"bytes\":3", "\"bytes\":-1"),
        ("\"bytes\":3", "\"bytes\":3.0"),
        ("\"bytes\":3", "\"bytes\":\"3\""),
        ("\"bytes\":3", "\"bytes\":3,\"bytes\":3"),
        ("\"bytes\":3", "\"bytes\":3,\"\\u0062ytes\":3"),
        ("\"bytes\":3", "\"bytes\":3,\"unknown\":\"private-header-canary\""),
    ] {
        let malformed = String::from_utf8(valid.clone())?.replace(from, to);
        let error = QuadletBundle::decode(malformed.as_bytes())
            .err()
            .ok_or("malformed transport accepted")?;
        assert!(!format!("{error:?} {error}").contains("private-header-canary"));
    }
    for end in [0, 1, MAGIC.len(), valid.len() - 1] {
        assert!(QuadletBundle::decode(&valid[..end]).is_err());
    }
    let mut trailing = valid.clone();
    trailing.push(b'\n');
    assert!(QuadletBundle::decode(&trailing).is_err());
    let mut truncated = valid;
    truncated.truncate(truncated.len() - 2);
    assert!(QuadletBundle::decode(&truncated).is_err());
    Ok(())
}
#[test]
fn unsafe_native_names_and_ascii_case_collisions_are_rejected_before_extraction() -> Result<(), Box<dyn Error>> {
    for name in [
        "../escape.container",
        "/absolute.container",
        "a/b.container",
        r"a\b.container",
        "C:drive.container",
        ".container",
        "unit.unknown",
        "unit\0.container",
        "unit\n.container",
    ] {
        assert!(
            QuadletBundle::decode(&frame(&[(name, b"")])?).is_err(),
            "unsafe member name accepted"
        );
    }
    for names in [["web.container", "web.container"], ["web.container", "WEB.container"]] {
        assert_eq!(
            QuadletBundle::decode(&frame(&[(names[0], b""), (names[1], b"")])?),
            Err(BundleError::MemberCollision)
        );
    }
    Ok(())
}
#[test]
fn utf8_byte_lengths_and_finite_size_budgets_are_enforced() -> Result<(), Box<dyn Error>> {
    let unicode = frame(&[("web.container", "λ".as_bytes())])?;
    assert_eq!(QuadletBundle::decode(&unicode)?.members()[0].text(), "λ");
    let bad = String::from_utf8(unicode)?.replace("\"bytes\":2", "\"bytes\":1");
    assert!(QuadletBundle::decode(bad.as_bytes()).is_err());
    assert_eq!(
        QuadletBundle::decode(&frame(&[("web.container", &[0xff])])?),
        Err(BundleError::InvalidUtf8)
    );
    let member = vec![b'a'; MAX_MEMBER_BYTES];
    assert_eq!(
        QuadletBundle::decode(&frame(&[("web.container", &member)])?)?.members()[0]
            .text()
            .len(),
        MAX_MEMBER_BYTES
    );
    let too_large = format!(
        "BOXFERRY-QUADLET-BUNDLE 1\nmembers 1\n---\n{{\"name\":\"web.container\",\"bytes\":{}}}\n",
        MAX_MEMBER_BYTES + 1
    );
    assert_eq!(
        QuadletBundle::decode(too_large.as_bytes()),
        Err(BundleError::LimitExceeded)
    );
    let oversized = vec![b'a'; MAX_BUNDLE_BYTES + 1];
    assert_eq!(QuadletBundle::decode(&oversized), Err(BundleError::LimitExceeded));
    let header = format!(
        "{{\"name\":\"web.container\",\"bytes\":0{}}}",
        " ".repeat(MAX_HEADER_BYTES)
    );
    assert_eq!(
        QuadletBundle::decode(format!("BOXFERRY-QUADLET-BUNDLE 1\nmembers 1\n---\n{header}\n\n").as_bytes()),
        Err(BundleError::LimitExceeded)
    );
    Ok(())
}
#[test]
fn extraction_preserves_bytes_and_uses_private_new_files() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let bundle = QuadletBundle::decode(&frame(&[
        ("web.container", b"[Container]\r\n# private-payload-canary\r\n"),
        ("data.volume", b"no final newline"),
    ])?)?;
    assert!(!format!("{bundle:?}").contains("private-payload-canary"));
    assert!(!format!("{:?}", bundle.members()[0]).contains("private-payload-canary"));
    let destination = scratch.0.join("output");
    bundle.extract_to(&destination)?;
    assert_eq!(
        fs::read(destination.join("web.container"))?,
        b"[Container]\r\n# private-payload-canary\r\n"
    );
    assert_eq!(fs::read(destination.join("data.volume"))?, b"no final newline");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&destination)?.permissions().mode() & 0o777, 0o700);
        assert_eq!(
            fs::metadata(destination.join("web.container"))?.permissions().mode() & 0o777,
            0o600
        );
    }
    Ok(())
}
#[test]
fn extraction_refuses_existing_entries_symlinks_and_retains_caller_directories() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let bundle = QuadletBundle::decode(&frame(&[("web.container", b"bytes")])?)?;
    let nonempty = scratch.0.join("nonempty");
    fs::create_dir(&nonempty)?;
    fs::write(nonempty.join(".existing"), b"preserve")?;
    assert!(bundle.extract_to(&nonempty).is_err());
    assert_eq!(fs::read(nonempty.join(".existing"))?, b"preserve");
    assert!(!nonempty.join("web.container").exists());
    let file = scratch.0.join("regular");
    fs::write(&file, b"preserve")?;
    assert!(bundle.extract_to(&file).is_err());
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let link = scratch.0.join("symlink");
        symlink(&nonempty, &link)?;
        assert!(bundle.extract_to(&link).is_err());
    }
    assert!(bundle.extract_to(&scratch.0.join("absent-parent/output")).is_err());
    let empty = scratch.0.join("empty");
    fs::create_dir(&empty)?;
    bundle.extract_to(&empty)?;
    assert_eq!(fs::read(empty.join("web.container"))?, b"bytes");
    Ok(())
}
#[test]
fn mid_write_failure_removes_only_invocation_owned_paths() -> Result<(), Box<dyn Error>> {
    let scratch = Scratch::new()?;
    let long = format!("{}.container", "x".repeat(300));
    let bundle = QuadletBundle::decode(&frame(&[("web.container", b"first"), (&long, b"second")])?)?;
    let absent = scratch.0.join("absent");
    assert!(bundle.extract_to(&absent).is_err());
    assert!(!absent.exists());
    let existing = scratch.0.join("existing");
    fs::create_dir(&existing)?;
    assert!(bundle.extract_to(&existing).is_err());
    assert!(existing.is_dir());
    assert_eq!(fs::read_dir(existing)?.count(), 0);
    Ok(())
}
