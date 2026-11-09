# BoxFerry 🚢

Give your container definitions a new destination. Moving a Compose application to Quadlet, or exploring a move from Podman? BoxFerry is an open-source CLI and Rust library that helps you make the journey and see what needs attention.

[Website](https://boxferry.dev/?utm_source=github.com&utm_medium=referral&utm_campaign=boxferry&utm_content=boxferry_readme_website) · [Getting started](https://boxferry.dev/docs/getting-started/?utm_source=github.com&utm_medium=referral&utm_campaign=boxferry&utm_content=boxferry_readme_getting_started) · [crates.io](https://crates.io/crates/boxferry) · [Rust API](https://docs.rs/boxferry)

## Try a conversion

Install the CLI with Cargo, then convert a Compose file into Quadlet files:

```console
cargo install boxferry --locked
boxferry convert compose quadlet --input-file compose.yaml --output-directory quadlet-output
```

Review the generated files and diagnostics before using them. The output directory must be new or empty; BoxFerry refuses to overwrite an existing entry.

BoxFerry supports Linux and client-side conversion on macOS. On Windows, run the Linux CLI inside WSL2. See [installation](docs/public/getting-started/) for release archives and platform limits.

## 🧭 Choose your route

Start with any of these inputs and choose any of the three outputs:

| Input                      | Output                      |
| -------------------------- | --------------------------- |
| Docker Compose             | Docker Compose              |
| Read-only Podman resources | Reviewable Podman artifacts |
| Podman Quadlet             | Podman Quadlet              |

All nine routes use an importer, a neutral model, and an exporter; same-format conversion is not a native pass-through. Docker Engine input and Kubernetes are deferred. Structured diagnostics explain conversion decisions that need attention.

## 🔎 Review with confidence

- **Podman input:** Acquisition is read-only. The CLI checks conventional local sockets unless you supply an explicit socket; macOS requires an explicit socket path.
- **Podman output:** Artifacts describe desired operations, not an observed inventory. BoxFerry does not execute generated commands or units, apply output, or deploy infrastructure. Running a generated script yourself performs real Podman operations.
- **Interpolation and secrets:** Compose interpolation starts only with `--env`, `--env-file`, or `--interpolate`; BoxFerry reads no implicit `.env` file. Secrets are redacted from diagnostics and reports by default.

## Keep exploring

- [Install and run a first conversion](docs/public/getting-started/), or follow a [conversion guide](docs/public/guides/).
- Find options in the [CLI reference](docs/public/reference/cli/) and learn about the [model and loss policy](docs/public/concepts/).
- Browse the published [documentation](https://boxferry.dev/docs/?utm_source=github.com&utm_medium=referral&utm_campaign=boxferry&utm_content=boxferry_readme_documentation).

The `boxferry` crate is the supported Rust facade.
[ComposeLens](https://github.com/Strukturpiloten/compose-lens), [PodmanLens](https://github.com/Strukturpiloten/podman-lens), and [QuadletLens](https://github.com/Strukturpiloten/quadlet-lens) own their respective native formats.

## Contribute

Have a question, found a bug, or have a migration story to share? We'd love to hear from you in the [issue tracker](https://github.com/Strukturpiloten/boxferry/issues). To contribute a change, start with the [development environment guide](docs/development-environment.md).

For a quick cleanup, use `./scripts/format-lint.sh --fix`; it runs no tests and is not validation evidence.
Run `./scripts/check-all.sh` for complete validation before submitting a change.

BoxFerry is maintained by [Martin “Becks” Beckert](https://github.com/TheRealBecks) through [Strukturpiloten OHG](https://www.strukturpiloten.de/?utm_source=github.com&utm_medium=referral&utm_campaign=boxferry&utm_content=boxferry_readme_company) and released under the [Mozilla Public License 2.0](LICENSE).
