# Testing

Run the complete deterministic gate before every pull request:

```console
./scripts/check-all.sh
```

## Choose a test layer

| Layer         | Protects                                                    |
| ------------- | ----------------------------------------------------------- |
| Unit          | Model invariants, mappings, diagnostics, and redaction      |
| Adapter       | Native-to-neutral and neutral-to-native contracts           |
| Golden        | Exact artifacts and diagnostic sequences                    |
| CLI           | Arguments, exit status, output safety, and reports          |
| Documentation | Every displayed command and its expected result             |
| Conformance   | Reviewed external tools in scheduled or opt-in environments |

Every behavior change needs a positive case and a relevant failure case. A mapping is incomplete
until unsupported values and target-version boundaries are tested.

## Fixture route corpus

Fixtures live under `fixtures/<suite>/<id>/`; their complete contract is in
[`fixtures/README.md`](../fixtures/). Each positive adapter or conversion scenario declares its
input and one expectation for every exporter reported by `boxferry capabilities`.

The corpus verifies:

- the minimum authorized loss policy and all stricter blocking policies;
- exact diagnostics, artifact sets, bytes, redaction, and protected-value handling;
- every applicable re-import, chained-conversion, and fixed-point contract;
- importer and exporter coverage as independent dimensions across all nine routes.

Compose and Quadlet artifacts can be imported again. Podman output cannot: it is a deployment plan,
not an observed inventory. Podman assertions instead compare deterministic `podman.json` and
`review.sh`, semantic operations, complete findings, and redaction.

## Podman evidence

The offline cassette corpus covers reviewed Podman 5.4 through 6.1 observations in rootful and
rootless contexts. It includes all resource kinds, pods and standalone containers, isolated and
shared networks, volumes and bind mounts, dependencies, protected metadata, incomplete resources,
ambiguous references, malformed observations, and redaction.

The normal pull-request gate needs no live Podman service or historical Podman container image.
Live runtime conformance remains scheduled or explicit opt-in evidence.

## Gate contents

`./scripts/check-all.sh` formats and lints owned files, tests every Cargo target and feature
boundary, checks Rust 1.85.0, audits dependencies, builds Rustdoc, checks coverage floors and local
links, and validates publishable packages.
Changelog validation is a dedicated job required by the aggregate gate.

Coverage is a regression ratchet, not proof of semantic correctness.
