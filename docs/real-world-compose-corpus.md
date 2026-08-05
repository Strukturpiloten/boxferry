# Real-world Compose corpus

The corpus turns current, widely deployed Compose projects into reproducible compatibility
evidence and feature goals. The machine-readable catalogue is
[`fixtures/real-world/corpus.toml`](../fixtures/real-world/corpus.toml). Every source is pinned to
an immutable Git commit and blob; BoxFerry does not vendor the upstream file.

This is intentionally not a leaderboard. A small deployment proves that the common path remains
usable, while a large deployment exposes independent semantic gaps. Exact conversion of one
project does not imply complete Compose compatibility.

## Selected projects

| Tier | Project | Why it belongs in the corpus | Main goals not already exact end to end |
| --- | --- | --- | --- |
| Baseline | [Paperless-ngx](https://github.com/paperless-ngx/paperless-ngx/blob/47727de11673c2a08d27b144048bd5f1a2ac8179/docker/compose/docker-compose.postgres.yml) | Small, understandable application/database/broker stack | `env_file` |
| Migration | [Immich](https://github.com/immich-app/immich/blob/1c7c28bb0d5c4a2edba062806efff50921b489b2/docker/docker-compose.yml) | Popular self-hosted application with release-oriented Compose installation | interpolation input, `env_file`, `shm_size` |
| Migration | [LibreChat](https://github.com/danny-avila/LibreChat/blob/120ee2afa6579fa1efa28dfb69d99fa7e9a4c040/docker-compose.yml) | Exercises `host-gateway`, deferred identity, and both mount syntaxes | interpolation, `env_file` |
| Migration | [Mattermost](https://github.com/mattermost/docker/blob/497414659ee7127677d2b91b44bb4f3ea9d14695/docker-compose.yml) | Official deployment with hardening and resource settings | caller-supplied interpolation, `tmpfs`, `security_opt`, memory limits |
| Migration | [Nextcloud](https://github.com/nextcloud/docker/blob/4de1eed65a44c61ecce601ef1d27abd2059b52e6/.examples/docker-compose/with-nginx-proxy/postgres/fpm/compose.yaml) | Multi-service example using the SELinux short-volume forms BoxFerry must retain | build, entrypoint, `env_file` |
| Migration | [Plausible CE](https://github.com/plausible/community-edition/blob/ec6c4da776547516d8f48159ce1a704df4f475ad/compose.yml) | Compact health-gated production-style topology | shell command, `ulimits`, host environment |
| Stress | [Apache Superset](https://github.com/apache/superset/blob/816f37f5b97b2e060a30cc53eaa6ac87f58320b3/docker-compose.yml) | Anchors, merge keys, profiles, build definitions, and long `env_file` syntax | build, env files, shell command, `network_mode` |
| Stress | [Appwrite](https://github.com/appwrite/appwrite/blob/9595616bdad93e3e2579348afea645f244a9062c/docker-compose.yml) | Very large graph with labels, lifecycle controls, logging, and a Docker socket | entrypoint, logging, socket policy, stop signal |
| Stress | [mailcow](https://github.com/mailcow/mailcow-dockerized/blob/06424670fa5d60fee851f58bfc49f66086d5f0a6/docker-compose.yml) | Production-oriented network and security torture case | static IPs, DNS, capabilities, privileged mode, sysctls, `tmpfs`, `ulimits` |
| Stress | [Supabase](https://github.com/supabase/supabase/blob/9952d6f10fb9a7b2d9d8c3312b279bfab2c4ba96/docker/docker-compose.yml) | Large official self-hosting stack that documents a nested-interpolation Podman boundary | interpolation, aliases, entrypoint, large dependency graph |

## Verified ingestion result

The complete pinned corpus passed on 2026-08-05 against released ComposeLens 0.1.11. The result
covers 95 imported services and 2,423 explicit
source-to-neutral conversion outcomes:

| Project | Services | Exact outcomes | Unsupported outcomes | Invalid outcomes |
| --- | ---: | ---: | ---: | ---: |
| Paperless-ngx | 3 | 29 | 1 | 0 |
| Immich | 4 | 31 | 5 | 0 |
| LibreChat | 6 | 71 | 3 | 0 |
| Mattermost | 2 | 16 | 13 | 3 |
| Nextcloud | 7 | 62 | 4 | 0 |
| Plausible CE | 3 | 63 | 2 | 0 |
| Apache Superset | 10 | 130 | 26 | 0 |
| Appwrite | 31 | 1,070 | 57 | 0 |
| mailcow | 18 | 484 | 49 | 0 |
| Supabase | 11 | 299 | 5 | 0 |
| **Total** | **95** | **2,255** | **165** | **3** |

No outcome was approximate at this import boundary. Mattermost's three invalid outcomes are the
deliberately unresolved `${MATTERMOST_CONTAINER_READONLY}` boolean and two
`${RESTART_POLICY}` values: the corpus never reads the upstream environment file or the ambient
process environment. Counts are useful regression
signals, not a compatibility percentage. One supported field may create many outcomes, and this
test stops at the neutral importer instead of claiming that every exact import can already be
emitted as exact Quadlet behavior.

The corpus found three valid-input regressions that now have minimal offline ComposeLens tests:
hyphenated YAML anchor names and aliased block values from Superset, unquoted `--option` command
items from Appwrite, and a blank line before Mailcow's indented `services` mapping. BoxFerry now
consumes the released 0.1.11 correction from crates.io without a sibling path override.

The explicit-container-name slice promoted 52 former unsupported outcomes to exact source import
across Immich, LibreChat, Appwrite, and Supabase. Minimal offline and golden tests independently
prove merged provenance, neutral separation from service keys, target validation, and
`ContainerName=` generation; the network-dependent corpus is supporting evidence rather than the
only regression gate.

The authored-restart slice promoted 73 literal policies from unsupported to exact import. The two
remaining restart values are Mattermost's intentionally unresolved expressions described above.
Minimal offline tests cover every supported policy plus unresolved, explicit-zero, and
out-of-range retry limits; the public golden test additionally proves exact `restart: "no"` to
Quadlet `Restart=no` conversion with provenance and real Podman 6.0.2 generator acceptance.

The first processing-context slice derived from the corpus is implemented in the CLI. Compose
interpolation is opt-in, begins with an empty environment, and accepts only plain literal values or
individually authorized sensitive process variables. BoxFerry does not read other ambient values
or an implicit `.env` file. ComposeLens 0.1.12 and BoxFerry now retain service `env_file`
declarations without reading them and can generate required safe paths as approximate Quadlet
`EnvironmentFile=` entries. Real deployment execution still needs the separate caller-authorized
file-content boundary and parser conformance evidence. The corpus itself deliberately supplies no
environment, so its counts continue to measure environment-independent ingestion.

## Current compatibility reading

The corpus exercises these already usable paths:

- loss-aware YAML loading and merging, including unknown fields and native source provenance;
- explicit profile selection;
- image references, including tag plus digest;
- simple exec commands and literal environment values;
- explicit runtime container names, service labels, and `extra_hosts`, including
  `host.docker.internal:host-gateway`;
- single published ports, named/anonymous volumes, bind mounts, and short-form SELinux `z`/`Z`;
- named networks, regular health checks, start/healthy dependencies, identity/context fields, and
  external Podman secrets within the documented subsets; and
- deterministic `.container`, `.pod`, `.network`, and `.volume` output for Podman 5.4.0 through
  the finite reviewed 6.0.2 ceiling.

The corpus also makes the remaining work concrete:

1. caller-authorized `.env` and service `env_file` content processing plus Podman parser
   conformance; declaration conversion and explicit literal/named-process interpolation are complete;
2. entrypoint and shell-command semantics;
3. hostname/domain/DNS, exposed ports, `tmpfs`, and shared-memory sizing;
4. CPU, memory, PID and `ulimits` resource policy;
5. capabilities, devices, privileged mode, namespaces, security options, and sysctls;
6. network aliases, static addresses, `network_mode`, IPAM, and advanced network definitions;
7. build/pull policy and `.build`/`.image` Quadlet units;
8. logging, stop signal/grace, init, and lifecycle hooks; and
9. Compose configs plus application-owned secret materialization.

Items 3 through 9 need explicit target and rootless/systemd policy. They are not mechanical key
renames.

## Test policy

Normal pull-request tests remain offline and use authored minimal reproductions. They are the
stable regression proof for a feature promoted from this corpus.

The opt-in corpus test downloads only the immutable URLs represented by the catalogue, verifies
the Git blob and required Compose fields, then loads, merges, profile-selects, and imports every
project. It never starts the applications, reads upstream `.env` files, or sends credentials.
Unresolved-value diagnostics are expected until a catalogue entry receives an explicit processing
environment; reaching the importer with those diagnostics is still a valid ingestion result. Run
it with:

```shell
cargo ci-real-world-compose
```

An upstream update is deliberate: review the new Compose file and license, change the commit,
blob, feature inventory, and goals together, then run both the corpus test and the
normal offline suites. Once a missing feature is implemented, add an authored minimal fixture and
golden conversion before marking it end to end in
[`format-coverage.md`](format-coverage.md).
