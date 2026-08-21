# Rust API

Use the `boxferry` facade unless an application needs one component crate directly.

```toml
[dependencies]
boxferry = { version = "0.5", default-features = false, features = ["compose", "quadlet"] }
```

The public flow is explicit:

1. Parse with the owning Lens library.
2. Create a BoxFerry source adapter.
3. Import into `Application`.
4. Select a target profile and `LossPolicy`.
5. Export a typed conversion plan.
6. Inspect or write the inert artifacts after caller authorization.

Core planning is side-effect free. File, environment, read-only runtime acquisition, and output
writes stay at caller-selected boundaries. Applying or deploying artifacts is outside BoxFerry.

All five supported crates use one pre-1.0 version. Minor releases may remove or replace APIs with
short migration notes; compatibility shims are not retained by default.

Build the API documentation locally with `RUSTDOCFLAGS="-D warnings" cargo ci-doc`.
