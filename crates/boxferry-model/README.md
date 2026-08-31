# boxferry-model

`boxferry-model` provides the format-neutral container application types used by BoxFerry. It
represents services, networks, volumes, secrets, configuration, image acquisition, provenance, and
protected values without exposing Docker Compose, Podman, or Quadlet types.

Most applications should depend on the [`boxferry`](https://crates.io/crates/boxferry) facade. Use
this crate directly when implementing a format adapter or inspecting BoxFerry's neutral model.

[BoxFerry documentation](https://boxferry.dev/docs/) ·
[Rust API](https://docs.rs/boxferry-model) ·
[Source code](https://github.com/Strukturpiloten/boxferry)

Licensed under the
[Mozilla Public License 2.0](https://github.com/Strukturpiloten/boxferry/blob/main/LICENSE).
