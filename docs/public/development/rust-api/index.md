# Rust API

Use the `boxferry` facade unless an application needs one component crate directly.

```toml
[dependencies]
boxferry = { version = "0.6", default-features = false, features = ["compose", "podman", "quadlet"] }
```

The public flow is explicit:

1. Parse with the owning Lens library or explicitly acquire a read-only Podman inventory.
2. Create a BoxFerry source adapter.
3. Import into `Application`.
4. Select a target profile and `LossPolicy`.
5. Export a typed conversion plan.
6. Inspect or write the reviewable artifacts after caller authorization.

Core planning is side-effect free. File, environment, read-only runtime acquisition, and output
writes stay at caller-selected boundaries. Podman input uses a caller-selected transport and
discovery request. Podman output is deterministic deployment-v1 JSON plus a command script, with no
execution method. Applying or deploying artifacts is outside BoxFerry.

All six supported crates use one pre-1.0 version. Minor releases may remove or replace APIs with
short migration notes; compatibility shims are not retained by default.

## Unreleased native-evidence migration

The next minor release removes `Service::podman_args` and its setters, plus
`Volume` getters and setters for `containers_conf_modules`,
`global_args`, and `podman_args`. Those values are opaque Quadlet source
evidence, not portable desired state.

After importing, inspect `Application::retained_native_evidence` for typed,
resource-qualified subjects, ordered value/reset events, protected physical
segments, and source provenance. `RetainedNativeEvidence::new` requires every
physical segment to be sensitive, plus provenance for the event and every
segment; plain segments are rejected before evidence can enter an application.
`Application::add_retained_native_evidence` requires the owning resource to
exist. There is intentionally no replacement setter that makes an exporter
render or execute opaque arguments. Every exporter instead returns one
unsupported outcome and value-free diagnostic per evidence subject; portable
typed intent remains independently exportable under the selected loss policy.

Build the API documentation locally with `RUSTDOCFLAGS="-D warnings" cargo ci-doc`.
