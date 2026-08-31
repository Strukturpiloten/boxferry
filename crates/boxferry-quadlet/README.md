# boxferry-quadlet

`boxferry-quadlet` maps source-aware Podman Quadlet documents from
[`quadlet-lens`](https://crates.io/crates/quadlet-lens) into BoxFerry's format-neutral application
model and exports neutral applications as validated Quadlet files.

The adapter preserves native findings for BoxFerry's loss policy and never installs units, reloads
systemd, or invokes Podman. Most applications should use the
[`boxferry`](https://crates.io/crates/boxferry) facade.

[BoxFerry documentation](https://boxferry.dev/docs/) ·
[Rust API](https://docs.rs/boxferry-quadlet) ·
[Source code](https://github.com/Strukturpiloten/boxferry)

Licensed under the
[Mozilla Public License 2.0](https://github.com/Strukturpiloten/boxferry/blob/main/LICENSE).
