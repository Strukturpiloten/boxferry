# ADR 0042: Bounded Podman creation evidence and explicit intent promotion

- Status: accepted
- Date: 2026-09-08
- Amends: [ADR 0034](0034-podman-lens-adapter-boundary.md) and
  [ADR 0037](0037-finite-podman-input-and-live-conformance.md)
- Builds on: [ADR 0039](0039-independent-migration-scenarios.md) and
  [ADR 0041](0041-portable-intent-and-retained-native-evidence.md)

## Context

Podman inspect combines several kinds of evidence. Configured values can express desired state,
effective values can combine configuration with defaults, runtime-assigned values describe one
instance, and local-resolution values describe only the source host. Podman also records a bounded
creation command on some containers. That command can corroborate authored image spelling or
SELinux mount relabel intent, but it can contain environment values, host paths, command payloads,
and other private data. It is neither a complete declarative definition nor permission to recover
desired state from arbitrary command text.

Portable migration also needs explicit boundaries. Effective named networks and volumes can be
reviewed as application resources. An absolute bind source is useful only when the target deliberately
reuses the same host path. A configured local image reference is desired image identity, but local
availability proves neither a build recipe nor registry or pull provenance. Treating all of those
observations alike would either discard useful intent or invent configuration.

PodmanLens 0.2.3 provides typed, bounded, privacy-safe creation evidence and typed mount SELinux
relabel observations. BoxFerry needs a stable policy for consuming that evidence without restoring
native command or mount-option parsing in the adapter.

## Decision

1. `boxferry-podman` consumes the crates.io `podman-lens` 0.2.3 contract. PodmanLens remains the
   sole owner of Libpod response decoding, creation-command bounds, native value parsing, and
   privacy filtering. BoxFerry does not inspect raw `CreateCommand`, `HostConfig.Binds`, or
   `Mounts[].Mode` values.

2. Creation evidence is corroboration only. It never creates a service, image build, mount, command,
   environment assignment, or other neutral desired state. A typed image hint that matches the
   configured image is exact corroboration. A hint that matches only the local image ID is an
   actionable limitation and does not create build or pull intent. A contradictory image hint is an
   actionable, value-free `services.<name>.image.authored_spelling` loss. Typed mount-index hints may
   corroborate only an already typed configured mount relabel. Contradictions are reported against
   `services.<name>.creation_evidence.mount_relabels[<index>]`, leaving the independently trusted
   typed mount relabel outcome exact.

3. Malformed or unavailable optional creation evidence does not invalidate otherwise complete typed
   inspect intent. It produces a field-specific diagnostic with `decision=omitted`,
   `available_promotion=none`, and remediation that says no automatic promotion exists. Raw command
   arguments, image spellings, environment values, paths, and post-image payloads never enter
   diagnostics, retained neutral evidence, reports, or support snapshots.

4. Configured image references are retained unchanged as neutral image intent, including
   `localhost/...`, unqualified, tagless, and locally resolved references. BoxFerry never infers a
   Containerfile, build context, registry origin, pull policy, remote availability, or source-host
   availability from the configured spelling, local image ID, or creation evidence. Exporters may
   diagnose target portability independently.

5. Effective named networks become application-owned only when
   `--promote-podman-effective-named-networks` authorizes the resource and
   `--promote-podman-portable-effective-settings` authorizes reviewed effective settings. The
   reviewed typed subset is internal state, subnets, gateways, lease ranges, and IPv6 implied by a
   typed IPv6 subnet. Container addresses, MAC addresses, interfaces, and other runtime assignments
   never become portable intent. PodmanLens 0.2.3 does not expose typed network driver, IPAM driver,
   or standalone `ipv6_enabled` values to this adapter; those fields remain exact actionable
   omissions with no BoxFerry promotion option. BoxFerry does not guess them.

6. Effective named volumes require
   `--promote-podman-effective-named-volumes`. Effective bind mounts require the independent
   `--promote-podman-effective-bind-mounts` authorization and are explicitly reviewed same-host
   intent, not portable storage. The target must provide the absolute source path, contents,
   ownership, permissions, and SELinux prerequisites. Typed configured `z` and `Z` observations map
   to neutral shared and private relabel intent and can be emitted by Compose and Quadlet
   exporters. The current PodmanLens deployment intent cannot emit host binds or SELinux relabel
   modes, so Podman output reports exact `.source` and `.selinux_relabel` omissions with no
   automatic promotion. Other native mount options remain actionable evidence.

7. Runtime-assigned and local-resolution observations remain structured evidence but are
   non-actionable when they describe facts no migration decision can preserve. Genuine unmapped
   native fields remain complete bounded structured findings. Every actionable omission identifies
   the exact resource and field, its decision, the available promotion flag or `none`, and a concrete
   remediation.

8. Environment assignments are sorted by key only when an exporter creates a new semantically
   equivalent assignment collection. Native document order, reset behavior, duplicate occurrences,
   and last-wins parsing remain owned by the native Lens and are not reordered during import.
   Sensitive values stay wrapped and redacted in diagnostics and support artifacts.

9. Independent migration-scenario contracts assert neutral intent and all registered exporters.
   They distinguish portable target prerequisites from reviewed same-host bind reuse and allow only
   exact reviewed loss tuples. Adapter regressions separately cover conflict, malformed, unavailable,
   future-unmodelled, and support-bundle privacy behavior. Live conformance remains read-only
   BoxFerry acquisition; harness-created resources may prove the same mappings without expanding
   product mutation authority.

## Consequences

- Useful portable Podman intent can survive conversion without treating an effective runtime
  snapshot as authored configuration.
- Same-host bind reuse is reviewable and testable without being advertised as portable migration.
- Local image references remain faithful while missing build and registry provenance stays visible.
- Creation evidence improves confidence but cannot silently override typed inspect evidence.
- Some network and mount fields remain actionable limitations until PodmanLens exposes additional
  typed contracts.
- Support bundles retain enough structured state to diagnose acquisition while excluding raw native
  creation values.

## Alternatives considered

### Parse `CreateCommand` or mount options in BoxFerry

Rejected because it duplicates PodmanLens native parsing, expands the privacy boundary, and makes
BoxFerry behavior depend on command syntax rather than typed observations.

### Treat effective and local observations as authored intent

Rejected because runtime defaults, assigned addresses, host paths, and local image availability do
not prove portable desired state.

### Infer a build or registry source for local images

Rejected because inspect and creation evidence do not contain a reproducible build recipe or
verified registry provenance.

### Suppress every unpromoted native observation

Rejected because genuine migration gaps must remain complete and actionable. Only proven
runtime-only or local-resolution facts become non-actionable evidence.
