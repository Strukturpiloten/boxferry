# ADR 0059: Evidence-based local Podman application selection

- Status: accepted
- Date: 2026-09-26
- Supersedes: [ADR 0036](0036-local-podman-cli-discovery-and-selectors.md)

## Context

ADR 0036 removed the mandatory socket and application-name flags but still required a native
selector. A user with one deployed application had to know a low-level resource or label before
conversion. A name or Compose label alone is not reliable authority for selecting a graph.

## Decision

1. Keep the CLI's finite local rootless-first, then rootful socket candidates and explicit
   `--podman-socket` override. Show the selected endpoint and context in human output. Never scan
   other paths, read remote connection configuration, start a service, or elevate privileges.
2. When no selector is supplied, acquire the bounded read-only inventory and derive application
   candidates from PodmanLens native pod membership and container-dependency evidence. A Compose
   ownership label never joins two automatic candidates. Select exactly one candidate without a
   prompt; if there are several, offer a one-shot, bounded numbered chooser only on a terminal.
   A complete Compose-project group may be offered as an advisory choice, but only explicit
   terminal consent or a label selector authorizes selecting its members together. The chooser
   shows bounded member identities and counts, not the project label value.
   Noninteractive ambiguity, absent applications, and invalid choices fail without a selection.
3. Preserve `--podman-all` as explicit selection of every eligible root. Keep exact resource,
   literal prefix, label, and network-boundary controls. A bare `--podman-resource NAME`
   denotes an exact container, avoiding a kind prefix for the common
   case. No selector accepts glob or regular-expression syntax.
4. `--application-name` is output naming only. Ambient label values never become default neutral
   application names or report values, even after a project group is explicitly selected. Human
   output shows the selected roots, group and
   shared-prerequisite counts, and stopped shared boundaries before artifact publication. The
   JSON report and privacy-safe support bundle keep their existing redaction boundaries; connection
   paths and chooser text are not copied into them.
5. The reusable facade still requires a caller-owned connection and discovery request. The CLI
   alone owns the convenience policy. ADR 0036's one-leaf support-bundle directory rule remains.

## Consequences

Ordinary one-application conversion needs neither a socket flag nor a selector. Scripts must
specify an exact selector when the inventory is ambiguous. Automatic selection may not include
resources grouped only by Compose labels; explicit selection or a reviewed boundary override is
needed when native evidence is insufficient.

## Alternatives considered

Selecting all on ambiguity would silently broaden a migration. Trusting a project name or
Compose labels alone would make metadata an ownership authority. Running the Podman CLI or
probing configured remote endpoints would cross the read-only local connection boundary.
