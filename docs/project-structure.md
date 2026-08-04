# Project structure

BoxFerry is a Cargo workspace. The public facade/CLI package, neutral model, and engine crates are
scaffolded; entries marked `planned` are created only when their milestone begins.

```text
boxferry/
├── .devcontainer/                   # digest-pinned VS Code environment and feature lock
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
├── rustfmt.toml
├── clippy.toml
├── deny.toml
├── .cargo/
│   └── config.toml              # canonical Cargo aliases
├── AGENTS.md
├── README.md
├── LICENSE
├── crates/
│   ├── boxferry/                 # public library facade and CLI executable
│   │   ├── src/lib.rs            # re-exports supported core and optional adapter APIs
│   │   ├── src/main.rs           # implemented Compose-to-Quadlet CLI and presentation
│   │   └── tests/                # facade contracts and repository-policy tests
│   ├── boxferry-model/           # ordered application graph, provenance, protected values
│   ├── boxferry-engine/          # adapters, planning, loss policy, targets, diagnostics
│   ├── boxferry-compose/         # implemented: ComposeLens import mapping
│   ├── boxferry-quadlet/         # implemented: validated QuadletLens export mapping
│   ├── boxferry-runtime/         # implemented: runtime-neutral observations and reconstruction
│   ├── boxferry-kubernetes/      # planned: Kubernetes mapping and target policy
│   ├── boxferry-docker/          # implemented: versioned Docker inspection and acquisition
│   ├── boxferry-podman/          # implemented: Podman inspection and acquisition
│   ├── boxferry-helm/            # planned: Helm rendering/generation integration
│   ├── boxferry-kustomize/       # planned: Kustomize rendering/generation integration
│   └── boxferry-testkit/         # planned: shared test builders; test-only
├── fixtures/
│   └── README.md                  # fixture location and safety rules
├── tests/
│   └── README.md                  # cross-crate scenario ownership
├── tools/
│   ├── docker-runtime-matrix.toml # exact Engine image and reviewed API bounds
│   └── podman-runtime-matrix.toml # exact executable lanes and explicit evidence gaps
├── docs/
│   └── fixture-format.md          # versioned fixture manifest contract
└── .github/
    ├── renovate.json
    └── workflows/
        ├── ci.yml
        ├── docker-runtime-conformance.yml
        └── podman-runtime-conformance.yml
```

## Crate publication policy

- `boxferry` is the supported high-level library facade and the package containing the installable
  application.
- `boxferry-model` and `boxferry-engine` are reusable component crates. They are intended for
  publication once T4 establishes their supported API contracts.
- Format adapters are independently testable and may be published when their first supported
  native mapping is complete.
- `boxferry-runtime` is a reusable pure component. It remains unpublished until the observation
  and reconstruction contract has been exercised by both native runtime adapters.
- Runtime adapters are published only when an embedded caller can use them safely without relying
  on CLI-global state.
- Test utilities and repository-only tools remain unpublished.
- Every crate starts with `publish = false`; publication is enabled only with API policy,
  documentation, package-content checks, and release automation.
- Supported BoxFerry crates use one lockstep pre-1.0 version so cross-crate requirements and
  release notes remain understandable.

## Placement rules

| Concern                                      | Owner                    |
| -------------------------------------------- | ------------------------ |
| Public facade, CLI parsing, and presentation | `boxferry`               |
| Neutral domain types                         | `boxferry-model`         |
| Planning and loss policy                     | `boxferry-engine`        |
| Compose semantic mapping                     | `boxferry-compose`       |
| Quadlet semantic mapping                     | `boxferry-quadlet`       |
| Kubernetes resource selection                | `boxferry-kubernetes`    |
| Runtime-neutral observations and inference   | `boxferry-runtime`       |
| Runtime command/API handling                 | Docker or Podman adapter |
| Native Compose syntax                        | ComposeLens repository   |
| Native Quadlet syntax and version catalogue  | QuadletLens repository   |

Do not place conversion logic in the CLI, native syntax handling in an adapter, or target-specific
fields in the neutral model merely to avoid defining a proper mapping. Public high-level behavior
must be callable through the `boxferry` library target before the CLI exposes it.
