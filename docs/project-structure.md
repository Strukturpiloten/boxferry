# Project structure

BoxFerry is a Cargo workspace. The CLI, neutral model, and engine crates are scaffolded; entries marked `planned` are created only when their milestone begins.

```text
boxferry/
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
│   ├── boxferry/                 # CLI executable
│   ├── boxferry-model/           # format-independent application model
│   ├── boxferry-engine/          # planning, policies, capabilities, diagnostics
│   ├── boxferry-compose/         # planned: ComposeLens mapping
│   ├── boxferry-quadlet/         # planned: QuadletLens mapping
│   ├── boxferry-kubernetes/      # planned: Kubernetes mapping and target policy
│   ├── boxferry-docker/          # planned: Docker runtime inspection
│   ├── boxferry-podman/          # planned: Podman runtime inspection
│   ├── boxferry-helm/            # planned: Helm rendering/generation integration
│   ├── boxferry-kustomize/       # planned: Kustomize rendering/generation integration
│   └── boxferry-testkit/         # planned: shared test builders; test-only
├── docs/
├── tests/                        # planned with the first conversion scenarios
│   ├── conversions/              # cross-format golden scenarios
│   ├── real-world/               # licensed, provenance-recorded projects
│   └── runtimes/                 # opt-in Docker, Podman, and Kubernetes tests
└── .github/
    ├── renovate.json
    └── workflows/
        └── ci.yml
```

## Crate publication policy

- `boxferry` is the installable application.
- Internal crates start with `publish = false` even when their boundaries are designed for reuse.
- A library is published only after its API, support policy, documentation, and semver commitments are explicit.
- Adapter crates remain independently testable whether or not they are published.

## Placement rules

| Concern                                     | Owner                    |
| ------------------------------------------- | ------------------------ |
| CLI parsing and presentation                | `boxferry`               |
| Neutral domain types                        | `boxferry-model`         |
| Planning and loss policy                    | `boxferry-engine`        |
| Compose semantic mapping                    | `boxferry-compose`       |
| Quadlet semantic mapping                    | `boxferry-quadlet`       |
| Kubernetes resource selection               | `boxferry-kubernetes`    |
| Runtime command/API handling                | Docker or Podman adapter |
| Native Compose syntax                       | ComposeLens repository   |
| Native Quadlet syntax and version catalogue | QuadletLens repository   |

Do not place conversion logic in the CLI, native syntax handling in an adapter, or target-specific fields in the neutral model merely to avoid defining a proper mapping.
