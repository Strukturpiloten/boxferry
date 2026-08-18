# Project structure

BoxFerry is a Cargo workspace. The public facade/CLI package, neutral model, and engine crates are
scaffolded; entries marked `planned` are created only when their milestone begins.

```text
boxferry/
├── .devcontainer/                   # digest-pinned VS Code environment and feature lock
├── Cargo.toml
├── Cargo.lock
├── release-plz.toml               # release-PR preparation only; publication stays protected
├── package.json                    # non-Rust Node development tools only
├── package-lock.json               # locked Markdownlint and Prettier graph
├── rust-toolchain.toml
├── rustfmt.toml
├── clippy.toml
├── deny.toml
├── lychee.toml                    # local/offline and rate-limited external link policy
├── .cargo/
│   └── config.toml              # canonical Cargo aliases
├── AGENTS.md
├── README.md
├── LICENSE
├── crates/
│   ├── boxferry/                 # public library facade and CLI executable
│   │   ├── src/lib.rs            # re-exports supported core and optional adapter APIs
│   │   ├── src/main.rs           # CLI, route dispatch, reports, and presentation
│   │   ├── src/route.rs          # finite typed CLI route registry
│   │   └── tests/                # facade contracts and repository-policy tests
│   ├── boxferry-model/           # ordered application graph, provenance, protected values
│   ├── boxferry-engine/          # adapters, planning, loss policy, targets, diagnostics
│   ├── boxferry-compose/         # implemented: ComposeLens import/export mapping
│   ├── boxferry-quadlet/         # implemented: validated QuadletLens import/export mapping
│   └── ...                       # future adapters only after their Lens contracts exist
├── fixtures/
│   └── README.md                  # fixture location and safety rules
├── tests/
│   └── README.md                  # cross-crate scenario ownership
├── scripts/
│   ├── check-all.sh               # complete deterministic local validation
│   ├── check-files.sh             # tracked non-Rust formatting and lint contract
│   ├── extract-release-notes.sh   # validated CHANGELOG section extraction
│   └── install-file-tools.sh      # pinned Linux file-quality tool installer
├── docs/
│   └── fixture-format.md          # versioned fixture manifest contract
└── .github/
    ├── scripts/
    │   └── publish-crate.sh       # resumable ordered crates.io publication helper
    ├── renovate.json
    └── workflows/
        ├── ci.yml
        ├── documentation-links.yml
        ├── release-plz.yml        # release PR creation and guarded publication dispatch
        └── release.yml            # protected lockstep crate publication
```

## Crate publication policy

- `boxferry` is the supported high-level library facade and the package containing the installable
  application.
- `boxferry-model` and `boxferry-engine` are reusable component crates. They are intended for
  publication once T4 establishes their supported API contracts.
- Format adapters are independently testable and may be published when their first supported
  native mapping is complete.
- New native Docker, Podman, and Kubernetes behavior is deferred to future independent Lens
  projects. Future BoxFerry adapters become thin semantic mappings after those libraries exist.
- Test utilities and repository-only tools remain unpublished.
- Publication remains enabled only for crates with API policy, documentation, package-content
  checks, and release automation.
- Supported BoxFerry crates use one lockstep pre-1.0 version so cross-crate requirements and
  release notes remain understandable.

## Placement rules

| Concern                                      | Owner                  |
| -------------------------------------------- | ---------------------- |
| Public facade, CLI parsing, and presentation | `boxferry`             |
| Neutral domain types                         | `boxferry-model`       |
| Planning and loss policy                     | `boxferry-engine`      |
| Compose semantic mapping                     | `boxferry-compose`     |
| Quadlet semantic mapping                     | `boxferry-quadlet`     |
| Docker protocol, plans, and execution        | Future DockerLens      |
| Podman protocol, plans, and execution        | Future PodmanLens      |
| Kubernetes native resources and versions     | Future KubernetesLens  |
| Native Compose syntax                        | ComposeLens repository |
| Native Quadlet syntax and version catalogue  | QuadletLens repository |

Do not place conversion logic in the CLI, native syntax handling in an adapter, or target-specific
fields in the neutral model merely to avoid defining a proper mapping. Public high-level behavior
must be callable through the `boxferry` library target before the CLI exposes it.
