# ADR 0020: rolling Compose Specification target for the generic CLI

- Status: accepted
- Date: 2026-08-11
- Amends: [ADR 0013](0013-explicit-compose-provider-and-runtime.md) for the CLI contract and
  [ADR 0019](0019-generic-cli-route-registry.md)

## Context

The generic Quadlet-to-Compose route needs one useful, reproducible output contract without
forcing every CLI user to select a provider implementation and a historical provider release.
Those selections remain meaningful compatibility evidence for embedded callers, but they add no
value to a provider-neutral document conversion route and incorrectly suggest that BoxFerry can
guarantee one generated document for every historical consumer.

QuadletLens reports syntax, typed-model, and cross-document diagnostics with source locations.
The CLI previously reduced parse failure to an aggregate summary, losing actionable native detail
before its privacy-safe report boundary.

## Decision

1. The generic Quadlet-to-Compose CLI route targets the rolling Compose Specification. It requires
   only explicitly ordered Quadlet inputs and `--project-name`; it accepts neither provider nor
   backend-runtime selection flags.
2. The route writes one deterministic `compose.yaml` whose generated document is parse-back
   validated. This is not a guarantee that every historical Compose consumer accepts the result.
3. The route neither infers nor invokes an installed provider or runtime. Its internal BoxFerry
   compatibility-profile revision is an implementation token, not a Compose Specification release
   version, and it never appears in CLI provider/version choices. The CLI reports its requested
   and resolved target as `rolling`.
4. `ComposeExporter` continues to support the exact provider-aware targets and optional exact
   backend runtime specified by ADR 0013 for embedded callers. This decision supersedes ADR 0013
   only where that ADR described the generic CLI contract.
5. `QuadletSource::parse` is the CLI parse boundary. Reports preserve each native stable
   code, severity, static summary, label message, and byte span in native collection order. Source
   names become invocation-local `<input-N>` aliases; source text, filenames, paths, and protected
   values remain excluded. Structured fatal parse failures carry only static stage and alias-safe
   location metadata.

## Consequences

- Generic CLI use is shorter and avoids false provider/runtime compatibility claims.
- Provider-aware applications retain explicit, exact compatibility selection through the public
  exporter API.
- Report consumers receive actionable native parse detail without scraping terminal output or
  widening the support-bundle privacy boundary.
- The Compose Specification target must remain documented as rolling rather than be represented as
  a synthetic consumer release.

## Alternatives considered

### Require a provider and runtime on every CLI invocation

Rejected because it burdens a provider-neutral output route and creates a false expectation of
historical-consumer compatibility.

### Infer local tooling

Rejected because installed tools are ambient, non-reproducible state and cannot provide a stable
compatibility claim.

### Render native diagnostics as terminal text and reparse them

Rejected because formatting is not a stable API and could leak paths or source values.
