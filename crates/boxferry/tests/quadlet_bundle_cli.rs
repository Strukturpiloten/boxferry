//! CLI transport layout, expanded native identity, byte-preserving extraction and reports.
#![cfg(all(feature = "cli", feature = "compose", feature = "quadlet", feature = "podman"))]
#[path = "support/podman_cassette.rs"]
#[allow(dead_code)]
mod podman_cassette;
use boxferry::quadlet_bundle::QuadletBundle;
use podman_cassette::{PodmanCassette, PodmanCassetteServer};
use serde_json::Value;
use std::{
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self, Box<dyn Error>> {
        let path = std::env::temp_dir().join(format!(
            "boxferry-bundle-cli-{}-{}",
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
fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_boxferry"))
}
fn report(output: &Output) -> Result<Value, Box<dyn Error>> {
    Ok(serde_json::from_slice(&output.stdout)?)
}
fn source(root: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let file = root.join("compose.yaml");
    fs::write(
        &file,
        "---\nname: bundled\nx-vendor: {description: inert}\nservices:\n  web:\n    image: example.invalid/web:1\n    restart: 'no'\n    volumes: [data:/data]\n    networks: [front]\n  worker:\n    image: example.invalid/worker:1\n    restart: 'no'\nvolumes:\n  data: {}\nnetworks:\n  front: {}\n",
    )?;
    Ok(file)
}
fn frame(name: &str, text: &str) -> Result<Vec<u8>, Box<dyn Error>> {
    Ok(format!(
        "BOXFERRY-QUADLET-BUNDLE 1\nmembers 1\n---\n{{\"name\":{},\"bytes\":{}}}\n{text}\n",
        serde_json::to_string(name)?,
        text.len()
    )
    .into_bytes())
}
#[test]
fn compose_bundle_matches_default_files_and_extraction_is_byte_identical() -> Result<(), Box<dyn Error>> {
    let root = Scratch::new()?;
    let input = source(&root.0)?;
    let native = root.0.join("native");
    let bundled = root.0.join("bundled");
    let normal = cli()
        .args(["convert", "compose", "quadlet", "--input-file"])
        .arg(&input)
        .arg("--output-directory")
        .arg(&native)
        .args(["--console-format", "json"])
        .output()?;
    assert!(normal.status.success(), "{}", String::from_utf8_lossy(&normal.stdout));
    let result = cli()
        .args(["convert", "compose", "quadlet", "--input-file"])
        .arg(&input)
        .args([
            "--output-layout",
            "quadlet-bundle",
            "--console-format",
            "json",
            "--output-directory",
        ])
        .arg(&bundled)
        .output()?;
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stdout));
    assert_eq!(fs::read_dir(&bundled)?.count(), 1);
    let encoded = fs::read(bundled.join("quadlet.bundle"))?;
    let bundle = QuadletBundle::decode(&encoded)?;
    assert_eq!(bundle.members().len(), 4);
    for member in bundle.members() {
        assert_eq!(fs::read(native.join(member.name()))?, member.text().as_bytes());
    }
    let json = report(&result)?;
    assert_eq!(json["operation"], "convert");
    assert_eq!(json["output_artifacts"].as_array().ok_or("artifacts")?.len(), 1);
    assert_eq!(json["output_artifacts"][0]["name"], "quadlet.bundle");
    assert_eq!(json["bundle_members"].as_array().ok_or("members")?.len(), 4);
    assert!(
        json["choices"]
            .as_array()
            .ok_or("choices")?
            .iter()
            .any(|choice| choice["name"] == "output_layout" && choice["value"] == "quadlet-bundle")
    );
    let extracted = root.0.join("extracted");
    let result = cli()
        .args(["extract", "quadlet", "--input-file"])
        .arg(bundled.join("quadlet.bundle"))
        .arg("--output-directory")
        .arg(&extracted)
        .args(["--console-format", "json"])
        .output()?;
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stdout));
    for member in bundle.members() {
        assert_eq!(fs::read(extracted.join(member.name()))?, member.text().as_bytes());
    }
    let json = report(&result)?;
    assert_eq!(json["operation"], "extract");
    assert_eq!(
        json["fidelity"],
        serde_json::json!({"exact":0,"approximate":0,"unsupported":0,"invalid":0,"other":0})
    );
    assert_eq!(json["requested_versions"]["minimum"], "not-applicable");
    Ok(())
}
#[test]
fn bundle_expansion_precedes_identity_and_all_member_collision_checks() -> Result<(), Box<dyn Error>> {
    let root = Scratch::new()?;
    let input = source(&root.0)?;
    let generated = root.0.join("generated");
    assert!(
        cli()
            .args(["convert", "compose", "quadlet", "--input-file"])
            .arg(input)
            .args(["--output-layout", "quadlet-bundle", "--output-directory"])
            .arg(&generated)
            .output()?
            .status
            .success()
    );
    let bundle = generated.join("quadlet.bundle");
    let failed = cli()
        .args(["validate", "quadlet", "quadlet", "--input-file"])
        .arg(&bundle)
        .args(["--console-format", "json"])
        .output()?;
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stdout).contains("multiple Quadlet unit files"));
    let validated = cli()
        .args(["validate", "quadlet", "quadlet", "--input-file"])
        .arg(&bundle)
        .args([
            "--application-name",
            "bundled",
            "--output-layout",
            "quadlet-bundle",
            "--console-format",
            "json",
        ])
        .output()?;
    assert!(
        validated.status.success(),
        "{}",
        String::from_utf8_lossy(&validated.stdout)
    );
    let json = report(&validated)?;
    assert_eq!(json["operation"], "validate");
    assert_eq!(json["inputs"].as_array().ok_or("inputs")?.len(), 1);
    assert_eq!(json["inputs"][0]["kind"], "quadlet-bundle");
    assert_eq!(json["input_bundle_members"].as_array().ok_or("mapping")?.len(), 4);
    assert_eq!(json["output_artifacts"][0]["name"], "quadlet.bundle");
    assert!(!String::from_utf8_lossy(&validated.stdout).contains(root.0.to_str().ok_or("root")?));
    let second = root.0.join("second");
    let converted = cli()
        .args(["convert", "quadlet", "quadlet", "--input-file"])
        .arg(&bundle)
        .args([
            "--application-name",
            "bundled",
            "--output-layout",
            "quadlet-bundle",
            "--output-directory",
        ])
        .arg(&second)
        .output()?;
    assert!(
        converted.status.success(),
        "{}",
        String::from_utf8_lossy(&converted.stderr)
    );
    assert_eq!(fs::read(&bundle)?, fs::read(second.join("quadlet.bundle"))?);
    let native = root.0.join("WEB.container");
    fs::write(&native, "[Container]\nImage=example.invalid/web:1\n")?;
    let collision = cli()
        .args(["validate", "quadlet", "quadlet", "--input-file"])
        .arg(&bundle)
        .arg("--input-file")
        .arg(&native)
        .args(["--application-name", "bundled", "--console-format", "json"])
        .output()?;
    assert!(!collision.status.success());
    assert!(String::from_utf8_lossy(&collision.stdout).contains("member names collide"));
    Ok(())
}
#[test]
fn sole_member_infers_native_stem_and_directory_discovery_stays_native_only() -> Result<(), Box<dyn Error>> {
    let root = Scratch::new()?;
    let bundle = root.0.join("not-the-application.bundle");
    fs::write(
        &bundle,
        frame("web.container", "[Container]\nImage=example.invalid/web:1\n")?,
    )?;
    let result = cli()
        .args(["validate", "quadlet", "quadlet", "--input-file"])
        .arg(&bundle)
        .args(["--console-format", "json"])
        .output()?;
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stdout));
    assert_eq!(report(&result)?["application"], "web");
    fs::write(
        root.0.join("web.container"),
        "[Container]\nImage=example.invalid/web:1\n",
    )?;
    fs::write(root.0.join("future.bundle"), "BOXFERRY-QUADLET-BUNDLE 99\n")?;
    let result = cli()
        .args(["validate", "quadlet", "quadlet", "--input-directory"])
        .arg(&root.0)
        .args(["--console-format", "json"])
        .output()?;
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stdout));
    assert_eq!(report(&result)?["application"], "web");
    let invalid = cli()
        .args(["validate", "quadlet", "quadlet", "--input-file"])
        .arg(root.0.join("future.bundle"))
        .args(["--console-format", "json"])
        .output()?;
    assert!(!invalid.status.success());
    assert!(String::from_utf8_lossy(&invalid.stdout).contains("unsupported Quadlet bundle transport version"));
    Ok(())
}
#[test]
fn extraction_preserves_native_only_content_while_conversion_retains_real_losses() -> Result<(), Box<dyn Error>> {
    let root = Scratch::new()?;
    let input = root.0.join("native.bundle");
    let text =
        "[Container]\r\nImage=example.invalid/web:1\r\nPodmanArgs=--private-transport-canary\r\n# --- no final newline";
    fs::write(&input, frame("web.container", text)?)?;
    let extracted = root.0.join("extracted");
    let result = cli()
        .args(["extract", "quadlet", "--input-file"])
        .arg(&input)
        .arg("--output-directory")
        .arg(&extracted)
        .args(["--console-format", "json"])
        .output()?;
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stdout));
    assert_eq!(fs::read(extracted.join("web.container"))?, text.as_bytes());
    assert!(!String::from_utf8_lossy(&result.stdout).contains("private-transport-canary"));
    let output = root.0.join("converted");
    let result = cli()
        .args(["convert", "quadlet", "quadlet", "--input-file"])
        .arg(&input)
        .args(["--output-layout", "quadlet-bundle", "--output-directory"])
        .arg(&output)
        .args(["--console-format", "json"])
        .output()?;
    assert_eq!(
        result.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    assert!(!output.exists());
    let json = report(&result)?;
    assert!(json["fidelity"]["unsupported"].as_u64().is_some_and(|count| count > 0));
    assert!(!String::from_utf8_lossy(&result.stdout).contains("private-transport-canary"));
    Ok(())
}
#[test]
fn malformed_extract_validates_entire_bundle_before_creating_or_opening_outputs() -> Result<(), Box<dyn Error>> {
    let root = Scratch::new()?;
    let input = root.0.join("malformed.bundle");
    let mut bytes = frame("web.container", "valid first bytes")?;
    bytes = String::from_utf8(bytes)?.replace("members 1", "members 2").into_bytes();
    bytes.extend_from_slice(b"---\n{\"name\":\"../private-name-canary.container\",\"bytes\":0}\n\n");
    fs::write(&input, bytes)?;
    for existing in [false, true] {
        let destination = root.0.join(if existing { "existing" } else { "absent" });
        if existing {
            fs::create_dir(&destination)?;
        }
        let result = cli()
            .args(["extract", "quadlet", "--input-file"])
            .arg(&input)
            .arg("--output-directory")
            .arg(&destination)
            .args(["--console-format", "json"])
            .output()?;
        assert!(!result.status.success());
        assert_eq!(destination.exists(), existing);
        if existing {
            assert_eq!(fs::read_dir(&destination)?.count(), 0);
        }
        assert!(!String::from_utf8_lossy(&result.stdout).contains("private-name-canary"));
        assert!(
            report(&result)?["output_artifacts"]
                .as_array()
                .is_some_and(Vec::is_empty)
        );
    }
    Ok(())
}
#[test]
fn podman_quadlet_output_route_uses_the_same_authorized_bundle_layout() -> Result<(), Box<dyn Error>> {
    let root = Scratch::new()?;
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/differential/podman-lens-complex-corpus/complex-6.1.0-rootless.cassette.json");
    let server = PodmanCassetteServer::start(PodmanCassette::load(&fixture)?)?;
    let socket = server.socket().to_owned();
    let destination = root.0.join("podman");
    let result = cli()
        .args(["convert", "podman", "quadlet", "--podman-socket"])
        .arg(&socket)
        .args([
            "--podman-all",
            "--application-name",
            "complex",
            "--promote-podman-effective-named-volumes",
            "--promote-podman-effective-named-networks",
            "--loss-policy",
            "partial",
            "--output-layout",
            "quadlet-bundle",
            "--output-directory",
        ])
        .arg(&destination)
        .args(["--console-format", "json"])
        .output()?;
    server.finish()?;
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stdout));
    let bundle = QuadletBundle::decode(&fs::read(destination.join("quadlet.bundle"))?)?;
    assert!(!bundle.members().is_empty());
    let json = report(&result)?;
    assert_eq!(json["output_artifacts"][0]["name"], "quadlet.bundle");
    assert!(json["fidelity"]["unsupported"].as_u64().is_some_and(|count| count > 0));
    Ok(())
}

#[test]
fn native_only_unit_kinds_extract_unchanged_and_conversion_diagnoses_their_loss() -> Result<(), Box<dyn Error>> {
    let root = Scratch::new()?;
    for (name, text, kind) in [
        ("workload.kube", "[Kube]\nYaml=workload.yaml\n", "kube"),
        (
            "source.artifact",
            "[Artifact]\nArtifact=example.invalid/source:1\n",
            "artifact",
        ),
    ] {
        let input = root.0.join(format!("{kind}.bundle"));
        fs::write(&input, frame(name, text)?)?;
        let destination = root.0.join(format!("extracted-{kind}"));
        let result = cli()
            .args(["extract", "quadlet", "--input-file"])
            .arg(&input)
            .arg("--output-directory")
            .arg(&destination)
            .args(["--console-format", "json"])
            .output()?;
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stdout));
        assert_eq!(fs::read(destination.join(name))?, text.as_bytes());
        assert_eq!(report(&result)?["bundle_members"][0]["unit_type"], kind);
        let result = cli()
            .args(["validate", "quadlet", "quadlet", "--input-file"])
            .arg(&input)
            .args(["--console-format", "json"])
            .output()?;
        assert_eq!(
            result.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
        assert!(
            report(&result)?["fidelity"]["unsupported"]
                .as_u64()
                .is_some_and(|count| count > 0)
        );
    }
    Ok(())
}

#[test]
fn packaging_limit_failure_retains_authorized_conversion_loss_diagnostics() -> Result<(), Box<dyn Error>> {
    use std::fmt::Write as _;
    let root = Scratch::new()?;
    let mut source = "---\nname: limit\nunknown_native_field: retained-loss-evidence\nservices:\n".to_owned();
    for index in 0..4097 {
        writeln!(
            source,
            "  service{index}:\n    image: example.invalid/web:1\n    restart: 'no'"
        )?;
    }
    let input = root.0.join("large.compose.yaml");
    fs::write(&input, source)?;
    let output = root.0.join("output");
    let result = cli()
        .args(["convert", "compose", "quadlet", "--input-file"])
        .arg(&input)
        .args([
            "--loss-policy",
            "partial",
            "--output-layout",
            "quadlet-bundle",
            "--output-directory",
        ])
        .arg(&output)
        .args(["--console-format", "json"])
        .output()?;
    assert_eq!(
        result.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&result.stdout)
    );
    let json = report(&result)?;
    assert_eq!(json["status"], "failure");
    assert_eq!(json["failed_stage"], "conversion");
    assert!(json["fidelity"]["unsupported"].as_u64().is_some_and(|count| count > 0));
    let diagnostics = json["diagnostics"].as_array().ok_or("diagnostics")?;
    assert!(diagnostics.iter().any(|diagnostic| diagnostic["code"] == "BFC0004"));
    assert!(
        diagnostics.iter().any(
            |diagnostic| diagnostic["fields"]
                .as_array()
                .is_some_and(|fields| fields.iter().any(|field| field["name"] == "reason"
                    && field["value"]
                        .as_str()
                        .is_some_and(|reason| reason.contains("count or size limit"))))
        )
    );
    assert!(json["output_artifacts"].as_array().is_some_and(Vec::is_empty));
    assert!(!output.exists());
    Ok(())
}
