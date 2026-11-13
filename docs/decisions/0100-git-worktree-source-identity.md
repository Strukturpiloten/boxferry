# ADR 0100: Git runtime and mounted worktree identity

- Status: accepted contract; image, architecture and VS Code acceptance require execution evidence
- Date: 2026-11-13
- Preserves: ADR 0079, ADR 0092, ADR 0094 and ADR 0096

## Decision

BoxFerry selects one official Git source archive and SHA256 in the existing canonical
inventory, built by the admitted complete-prefix installer. Both ordinary PATH and
isolated `/usr/bin` Git execution use this prefix. Binary tool profiles and other
image owners' definitions retain independent contracts and qualification.

Host-created relative worktrees require an executable supporting their repository
extension and a mount preserving access to the common metadata. Use the official
Dev Containers common-directory mount option, retaining conventional Podman storage.
VS Code's corresponding application setting requires explicit user configuration;
repository workspace settings cannot silently opt into it. Do not rewrite maintainer
extensions, relocate existing metadata, inject routing variables or broaden Git trust.

Every source check inspects its explicit root and index. An inaccessible legacy
absolute layout receives an actionable diagnostic. The explicit host-side fallback
creates a new task-owned standalone clone and verifies baseline HEAD, canonical index
entries and working bytes/modes before selection. It never substitutes a clone
implicitly, copies credentials/configuration, or changes the maintainer checkout.
Unsupported index/file layouts fail closed. Preserve published/development API
baseline distinctions and complete local/PR/queue/main/release requirements.

## Admission and evidence

One native Renovate custom owner pairs official stable archive versions and SHA256,
with canonical downloaded-tool grouping and existing approvals. The checksum feed
provides no trustworthy publication timestamp; the unchanged trusted 72-hour first-seen
hold remains mandatory. No updater schedule or image publication is activated.

Admit the new host-created fixture producer before selecting it operationally.
Coherently adopt that distributed commit in a subsequent change and require fresh
native receipts; a source-preparation PR cannot claim those hosted checks ran.

Record exact Git versions, format, source/index identity and mount paths for primary,
relative, legacy absolute and standalone layouts. Native amd64/arm64 image execution
and actual VS Code acceptance are distinct evidence. Partial success leaves the
corresponding acceptance open; it cannot close the broader programme.
