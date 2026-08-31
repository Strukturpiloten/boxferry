# boxferry-compose

`boxferry-compose` maps source-aware Docker Compose projects from
[`compose-lens`](https://crates.io/crates/compose-lens) into BoxFerry's format-neutral application
model and exports neutral applications as Compose documents.

The adapter does not read files, inspect a runtime, or select profiles implicitly. Compose input and
output always pass through the neutral model, including Compose-to-Compose conversion, so fidelity
decisions remain visible to BoxFerry's loss policy.

Most applications should use the [`boxferry`](https://crates.io/crates/boxferry) facade.

[BoxFerry documentation](https://boxferry.dev/docs/) ·
[Rust API](https://docs.rs/boxferry-compose) ·
[Source code](https://github.com/Strukturpiloten/boxferry)

Licensed under the
[Mozilla Public License 2.0](https://github.com/Strukturpiloten/boxferry/blob/main/LICENSE).
