# boxferry-compose

`boxferry-compose` maps source-aware Docker Compose projects from
[`compose-lens`](https://crates.io/crates/compose-lens) into BoxFerry's format-neutral application
model and exports neutral applications as Compose documents.

The adapter does not read files, inspect a runtime, or select profiles implicitly. Compose input and
output always pass through the neutral model, including Compose-to-Compose conversion, so fidelity
decisions remain visible to BoxFerry's loss policy.

Aliases in service network attachments retain the native per-scalar sensitivity classification.
Public literals and explicitly public interpolation values remain ordinary aliases; sensitive substitutions remain
protected. The adapter corroborates the exact service/network path, ordered values and source spans
against the same merged project. Missing or mismatched metadata keeps all aliases protected. This
does not authorize protected health commands, addresses or other values, and changes no diagnostic
or report redaction policy. The existing native dependency supplies this metadata; no dependency,
tooling, operational pin or Renovate ownership change is required.

Compose export refuses the entire candidate when any alias in a service network attachment is
protected, under every loss policy, rather than emitting unmarked private text or silently dropping
a service alias. Neutral values and provenance remain available, with indexed, value-free unsupported
diagnostics. Public service aliases can export and freshly reload their text exactly. Successful
protected-service-alias fresh-text reimport
and recovery of sensitivity from arbitrary historical plaintext are not promised; see
[ADR 0073](../../docs/decisions/0073-compose-alias-sensitivity-and-export-refusal.md).

Group-network aliases are not serialized. The existing structured unsupported outcomes and partial
loss-policy handling for unmapped group networking/runtime settings remain unchanged.

Most applications should use the [`boxferry`](https://crates.io/crates/boxferry) facade.

[BoxFerry documentation](https://boxferry.dev/docs/) ·
[Rust API](https://docs.rs/boxferry-compose) ·
[Source code](https://github.com/Strukturpiloten/boxferry)

Licensed under the
[Mozilla Public License 2.0](https://github.com/Strukturpiloten/boxferry/blob/main/LICENSE).
