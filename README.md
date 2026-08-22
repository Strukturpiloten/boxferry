# BoxFerry

BoxFerry converts container application definitions without hiding incompatible intent. It plans
through one neutral model, reports every non-exact decision, and writes output only after the
selected loss policy authorizes it.

## Supported routes

| Input   | Compose output        | Quadlet output          | Podman output         |
| ------- | --------------------- | ----------------------- | --------------------- |
| Compose | Neutral-model Compose | Canonical Quadlet files | Inert deployment plan |
| Quadlet | Neutral-model Compose | Canonical Quadlet files | Inert deployment plan |
| Podman  | Neutral-model Compose | Canonical Quadlet files | Inert deployment plan |

All nine routes pass through the neutral application model. Podman input uses explicit read-only
acquisition through the independent PodmanLens library; Podman output is a review-only plan, not an
apply operation. Docker and Kubernetes remain deferred.

## Install and convert

BoxFerry supports Linux. Windows users run it inside WSL2.

```console
cargo install boxferry --locked
boxferry convert compose quadlet --input-file compose.yaml --output-directory quadlet-output
```

The output directory may be absent or empty. BoxFerry rejects any directory containing an entry.
It writes inert artifacts only: it never executes generated commands, starts units, deploys
infrastructure, or sends mutating runtime API requests.

Compose interpolation is disabled by default. Enable it explicitly with `--interpolate`, then add
values with `--env-file FILE` or `--env NAME=VALUE`. BoxFerry reads no implicit `.env` file and no
process variable unless `--env NAME` authorizes that exact name.

## Documentation

The public documentation at [boxferry.dev/docs](https://boxferry.dev/docs/) contains installation,
all nine conversion guides, CLI reference, stable diagnostic-rule pages, compatibility, error
reports, and development guidance.

Every displayed BoxFerry command is executed by repository black-box tests.

## Rust libraries

The `boxferry` crate is the supported facade. Component crates expose the neutral model, engine,
Compose mapping, Quadlet mapping, and Podman mapping for applications that need narrower boundaries.

- [ComposeLens](https://github.com/Strukturpiloten/compose-lens) owns Compose parsing and rendering.
- [PodmanLens](https://github.com/Strukturpiloten/podman-lens) owns explicit read-only Podman
  acquisition, native observations, version evidence, deployment planning, and deterministic
  rendering.
- [QuadletLens](https://github.com/Strukturpiloten/quadlet-lens) owns Quadlet parsing, rendering, and
  Podman capability evidence.

## Development

Use the Dev Container and run the complete gate:

```console
./scripts/check-all.sh
```

Start with [repository documentation](docs/README.md) and [contribution guidance](AGENTS.md).

## License and stewardship

BoxFerry is maintained by [Martin “Becks” Beckert](https://github.com/TheRealBecks) through
[Strukturpiloten OHG](https://www.strukturpiloten.de/) and licensed under the
[Mozilla Public License 2.0](LICENSE).
