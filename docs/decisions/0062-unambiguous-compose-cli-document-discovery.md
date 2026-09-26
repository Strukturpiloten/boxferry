# ADR 0062: Unambiguous current-directory Compose document discovery

- Status: accepted
- Date: 2026-09-26
- Amends: [ADR 0018](0018-generic-cli-and-diagnostic-support-bundle.md) decision 3's explicit-document requirement

## Context

The authored Compose file is usually in the directory where a user invokes BoxFerry, but requiring
`--input-file compose.yaml` for every conversion obscures the first command. Automatic discovery
must not silently choose between competing documents or acquire unrelated host state.

## Decision

1. On a Compose input route, with neither `--input-file` nor `--input-directory`, inspect only the
   invocation's current directory. `--project-directory` changes relative-path resolution, not
   input discovery. There is no recursive search, parent search, prompt, or implicit `.env` read.
2. Inspect exactly `compose.yaml`, `compose.yml`, `podman-compose.yaml`,
   `podman-compose.yml`, `docker-compose.yaml`, and `docker-compose.yml`. Select only one existing
   regular, non-symlink candidate. No candidate, multiple candidates, a symlink or non-regular
   candidate, and an unreadable selected file fail before output is written. Open the selected
   file without following a final symlink, verify its identity against discovery, and retain the
   opened handle through reading so a pathname swap cannot redirect source bytes. On Unix targets
   without the reviewed no-follow flags, implicit discovery fails closed and explicit input stays
   available. A collision is not interpreted as a merge or an override; the user can select files
   explicitly in order.
3. Explicit `--input-file` ordering and `--input-directory`'s existing conventional-file priority
   remain unchanged. Neither one silently adds other documents. Quadlet still requires an
   explicit input selection and application name. Podman continues its separate live selection.
4. Human output names the selected document. Structured reports use existing invocation-local
   aliases so the absolute working-directory path does not leak into JSON or support bundles.
   Discovery applies to `convert` and `validate` and every Compose output route.

## Consequences

The ordinary one-file Compose project needs only route and output destination. A directory with
both modern and legacy files needs an explicit selection, even if one name would win an explicit
directory lookup. Discovery reads authored definitions, not deployed containers, named volume
contents, or application data. Existing no-clobber and interpolation boundaries remain in force.

## Alternatives considered

Prioritizing one implicit candidate was rejected because it hides collisions and may select an
unintended workload. Reading `.env`, recursively searching, and prompting in automation were
rejected because they enlarge authority or make input selection nondeterministic.
