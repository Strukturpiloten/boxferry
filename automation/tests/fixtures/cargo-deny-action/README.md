# Nested Cargo Deny Action fixture

These authored, dependency-free Cargo files are synthetic metadata, not product
packages or a license change. The declared BSD license distinguishes policy selection:
`source/explicit-policy.toml` allows it, while the default `source/deny.toml` rejects it.
The separate temporary root manifest in the Python regression must also fail if selected.
No Rust compilation, runtime, image or dependency download is needed for those controls.

The producer's existing dependency job copies `source/` to its workspace root and
runs the same pinned Cargo Deny Action as the shared workflow. Its positive check
retains all four default audit categories, explicit nested manifest, all features and
non-default configuration. No expected-negative Action step emits error annotations.
The parser-backed dependency test compares both input contracts and checks native
Renovate ownership. Local selected-tool controls capture the old argument placement,
implicit/decoy configuration, wrong manifest and missing configuration failures as data.

The dependency suite parses both workflows with the pinned Renovate package's YAML
parser and verifies unique native GitHub Actions ownership. The Action occurrence
uses the existing full SHA/release comment, manager, grouping and approval rules;
fixture metadata stays outside updates. No software pin value or custom manager changes.

The interface was reviewed read-only on 2026-11-05 against
`EmbarkStudios/cargo-deny-action` v2.1.1's pinned
[descriptor](https://github.com/EmbarkStudios/cargo-deny-action/blob/3c6349835b2b7b196a839186cb8b78e02f7b5f25/action.yml),
[entrypoint](https://github.com/EmbarkStudios/cargo-deny-action/blob/3c6349835b2b7b196a839186cb8b78e02f7b5f25/entrypoint.sh),
and [Dockerfile](https://github.com/EmbarkStudios/cargo-deny-action/blob/3c6349835b2b7b196a839186cb8b78e02f7b5f25/Dockerfile).
The Dockerfile declares Cargo Deny 0.20.2; the local selected tool independently
reported `cargo-deny 0.20.2`. The Action is dual MIT/Apache-2.0 licensed. No upstream
implementation or oracle code is copied or redistributed here. The offline CLI controls
skip when the selected binary is absent; the positive selected-Action fixture must
still pass in the required dependency job.
