# boxferry-podman

`boxferry-podman` maps native resources acquired by
[`podman-lens`](https://crates.io/crates/podman-lens) into BoxFerry's format-neutral application
model. It also exports neutral applications as reviewable Podman deployment JSON and command
artifacts.

Acquisition is explicitly read-only. Connection selection, discovery boundaries, file writes, and
execution remain caller concerns; this crate never opens a connection or executes Podman commands.

Most applications should use the [`boxferry`](https://crates.io/crates/boxferry) facade.

[BoxFerry documentation](https://boxferry.dev/docs/) ·
[Rust API](https://docs.rs/boxferry-podman) ·
[Source code](https://github.com/Strukturpiloten/boxferry)

Licensed under the
[Mozilla Public License 2.0](https://github.com/Strukturpiloten/boxferry/blob/main/LICENSE).
