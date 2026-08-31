# boxferry-engine

`boxferry-engine` provides the loss-aware conversion engine behind BoxFerry: adapter contracts,
target profiles, conversion outcomes, diagnostic rules, and policies that decide whether a
candidate output may be returned.

The engine operates on [`boxferry-model`](https://crates.io/crates/boxferry-model) applications and
does not parse native formats or write files. Most applications should use the
[`boxferry`](https://crates.io/crates/boxferry) facade unless they are implementing an adapter or a
custom conversion workflow.

[BoxFerry documentation](https://boxferry.dev/docs/) ·
[Rust API](https://docs.rs/boxferry-engine) ·
[Source code](https://github.com/Strukturpiloten/boxferry)

Licensed under the
[Mozilla Public License 2.0](https://github.com/Strukturpiloten/boxferry/blob/main/LICENSE).
