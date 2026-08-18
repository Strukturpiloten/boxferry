# ADR 0006: Finite Podman inspect decoder with separate acquisition

- Status: superseded
- Date: 2026-08-04
- Superseded by: [ADR 0032](0032-future-native-lens-boundaries.md)

## Context

Podman inspection is useful for reconstructing reusable definitions from deployed resources, but
its JSON is an implementation response rather than an authored declaration. It contains effective
defaults, opaque runtime identities, potentially sensitive environment and command values, and
configuration that the first neutral model cannot represent. Podman 5.4.0 is BoxFerry's support
floor, while support must extend to a finite reviewed current version instead of silently assuming
future response compatibility.

Command execution and response decoding also have different trust and test boundaries. A parser
can be deterministic and fixture-tested without a daemon. A command runner must select resources,
handle permissions and failures, prove read-only behavior, and avoid leaking host state.

## Decision

Create an unpublished `boxferry-podman` component crate and additive, non-default
`podman-runtime` facade feature. The first increment accepts five caller-supplied JSON arrays for
containers, images, networks, volumes, and pods plus the exact producing `PlatformVersion`. It
performs no command execution, socket access, filesystem discovery, or environment lookup.

The reviewed range is inclusive Podman 5.4.0 through 6.1.0. Decoding fails closed outside that
range. Private Serde response types cover only reviewed fields; additive fields remain tolerated.
Every meaningful reusable field outside the mapped subset produces `BFP0002` and an unsupported
outcome. Invalid data, unsupported versions, and incomplete relationships use distinct stable
diagnostics.

Raw runtime IDs are used only inside decoding to resolve relationships. Public observations use
stable kind-and-name source identities. Inspect documents, environment values, effective command
arguments, and optional creation commands are sensitive in debug output. Podman acquisition uses
a replaceable executor and a closed command type that permits only explicit container, image,
network, volume, and pod `inspect` operations.

## Consequences

- Embedded callers can feed API, CLI, remote, recorded, or sanitized responses through one pure
  public contract.
- Unit and fixture tests need no installed Podman, privileged container, or host socket.
- Accepting a Podman version means its reviewed response subset can be decoded; it does not mean
  every inspected native configuration field is reconstructable.
- The finite ceiling must move only with exact-version source review and fixtures.
- The process command runner is independently testable and cannot make the decoder depend on
  global CLI state.
- Docker retains a separate native adapter even where response shapes look similar.

## 6.1.0 review update

Podman 6.1.0 changes the reviewed container-inspect source only by adding a Go-template helper
method; the JSON response structs consumed by this adapter are unchanged from 6.0.2. The ceiling
therefore advances to 6.1.0 with a dedicated sanitized fixture. This is decoder-shape evidence,
not live-runtime conformance or proof that every native field is reconstructable.

## Alternatives

Invoking `podman inspect` directly inside the shared runtime crate was rejected because it couples
the neutral reconstruction layer to one implementation and ambient host state. Depending on
Podman's internal Go types was rejected because BoxFerry is a Rust library and needs a stable
caller-supplied boundary. Accepting all later Podman versions optimistically was rejected because
unknown response changes could become silent conversion loss.
