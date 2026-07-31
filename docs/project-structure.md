# Target project structure

BoxFerry will be a Cargo workspace from its first implementation milestone. The following is the target structure, not a statement that every directory already exists.

```text
boxferry/
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
├── AGENTS.md
├── README.md
├── LICENSE
├── crates/
│   ├── boxferry/                 # CLI executable
│   ├── boxferry-model/           # format-independent application model
│   ├── boxferry-engine/          # planning, policies, capabilities, diagnostics
│   ├── boxferry-compose/         # ComposeLens mapping
│   ├── boxferry-quadlet/         # QuadletLens mapping
│   ├── boxferry-kubernetes/      # Kubernetes mapping and target policy
│   ├── boxferry-docker/          # Docker runtime inspection
│   ├── boxferry-podman/          # Podman runtime inspection
│   ├── boxferry-helm/            # Helm rendering/generation integration
│   ├── boxferry-kustomize/       # Kustomize rendering/generation integration
│   └── boxferry-testkit/         # shared test builders; never a runtime dependency
├── docs/
├── tests/
│   ├── conversions/              # cross-format golden scenarios
│   ├── real-world/               # licensed, provenance-recorded projects
│   └── runtimes/                 # opt-in Docker, Podman, and Kubernetes tests
└── .github/
    ├── workflows/
    └── ISSUE_TEMPLATE/
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
