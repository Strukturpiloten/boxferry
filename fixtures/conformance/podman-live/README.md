# Podman live conformance catalogues

These files are executable compatibility evidence, not deployment input.

- `matrix.tsv` is the accepted immutable amd64 inventory. Its columns are cell ID, image, declared
  Podman/package version, distribution, root mode, lane, and architecture.
- `scenarios.tsv` inventories assertions exercised by the shared runner.
- `limitations.tsv` names accepted image-specific exceptions. A limited row claims no resource
  coverage.
- `candidates.toml` contains only active replacement proposals. It may be empty after
  every proposal has a reviewed admission decision; archived proposal catalogues remain
  with their bounded evidence under `revalidation/`.
- `apply-target-containers.conf` configures the disposable nested apply target.
- `capture_proxy.py` is the privacy boundary for explicitly authorized cassette capture.

All three already-privileged disposable outer launch paths explicitly select
`apparmor=unconfined`, alongside the existing disabled SELinux label. Privileged
launch alone can inherit a named unconfined host profile such as VS Code's
`vscode`; a host `pasta` profile can then deny the nested Podman service's
termination signal during volume-preserving recreation. This setting is limited
to the disposable test container: no host profile is edited, PID isolation and
exact run-owned cleanup remain intact, and inner rootless mode is still verified.
It does not establish conformance under an enforcing workload security profile.

## Active patches and retained evidence

### Closed local and socket presence queries

The outer-storage helper and live runner use one typed, read-only presence query for
local Podman resources and explicitly selected Unix sockets. It covers exact outer
container/storage-volume checks, applied-target container/network/volume absence,
limitation cleanup readbacks and host image-cache checks before tagging or pulling.
Only completed native status 0 or 1 with empty combined stdout and stderr establishes
present or absent. Diagnostics (including error exit 1 and warning exit 0), other
statuses, output overflow, deadlines, cancellation and unverified process teardown
remain unknown. The shell requires the exact newline-terminated marker and matching
helper status; helper launch or timeout failures cannot establish absence.

`scripts/lib/bounded-native-read.py` is the sole reader, extracted from the Docker
application helper without changing its readiness diagnostics or query behavior.
It retains a three-second read bound, a 16-KiB combined-output cap for presence and
owned-process-group termination before leader reaping. Presence additionally requires
bounded read-only group disappearance afterward; no signal is sent to a reaped group.
The four-second helper budget remains inside the existing 30/90-second caller timeout
and kill-after bound. Both helper and timer run at the caller's native-client privilege;
the rootful live runner remains rootful and rootless storage probes remain rootless.
No additional privilege or runtime mutation is introduced by observation.

Presence never authorizes ownership. Outer cleanup retains exact run/outer volume
labels, successful authenticated container inspection and immutable-ID removal,
non-force volume removal, registration before creation and attempts on later resources.
Unknown queries cannot authorize creation, tagging, pulling or image ownership claims.
Unknown presence or failed cleanup retains the private runtime and discovery recovery
files as well as failed-run artifacts. Verified cleanup preserves the original failure
status. Offline fake tests exercise both initial and post-removal ambiguity, mutation
refusal, later-resource attempts and recovery retention; they supply no live admission.
The existing complete-gate outer-storage test directly runs the shared helper vectors.

This is the first query-hardening stage of
[BoxFerry #342](https://github.com/Strukturpiloten/boxferry/issues/342), not a completed
consumer rollout. Nextcloud and Forgejo shared-edge creation, Observability edge/peer
creation and application image checks inside `podman exec` remain follow-ups. Their
pre-API container-CLI transport needs a proven bounded inner-command termination
boundary: killing a host client process group alone cannot prove that nested workload
stopped. This stage does not change application provisioning or use a missing API
socket as permission to start an unverified inner query.

Renovate no-change evidence: the live runner's existing workload-image assignment and
its regex manager remain at the same path with identical bytes. Matrix images remain
owned by the `podman-live/matrix.tsv` manager; application images and DockerLens pins
retain their existing canonical sources. New helper/test files match no custom-manager
file patterns. No software pin, dependency, workflow, manager, historical catalogue
or captured evidence changes.

Compose reimports retain byte-identical canonical semantics. Quadlet comparisons
also retain every remaining field, but explicitly verify two diagnosed fixture
losses: the dual-network API service's `api`/`public-api` aliases and the options
service's finite `on-failure:3` retry policy. Each requires its exact warning
subject and reason; neither arbitrary aliases nor unrelated differences are
normalized. Mutation tests protect this narrow comparison boundary.

The upstream-source 5.8 and 6.1 lanes replace superseded patches with 5.8.7 and 6.1.2,
respectively, in both root modes. Their immutable OCI index digests select the declared
amd64 platform; the runner still verifies the observed engine version, package and root mode.
Distribution-package lanes are separate evidence and keep their exact package revisions until
those images receive their own reviewed replacement.

PodmanLens 0.2.5 supplies the read-only API boundary and QuadletLens 0.2.4 admits the generator
catalogue through 6.1.2. Runtime input, Quadlet generation and Podman deployment rendering are
distinct claims: PodmanLens's exact renderer catalogue still ends at 6.1.0, so BoxFerry retains
that output ceiling. Running a 6.1.0 API plan against 6.1.2 does not add a new renderer target.

Captured 6.1.0 Paperless/Immich cassettes and authored historical route fixtures remain unchanged.
They are offline regressions, not extra routine live patch lanes or evidence for 6.1.2.
The capture tool now names new candidates 6.1.2 and binds them to the current matrix, source
revision and actual API handshake. Its privacy-only verifier still accepts reviewed historical
captures. A new candidate needs independent privacy/provenance review before fixture admission.

Application checks that inspect generated Compose/Quadlet environment values explicitly select
`--environment-values include` for their synthetic fixtures. Podman plans retain default withholding;
their omission diagnostics and protected-value checks remain mandatory. Structured reports stay
redacted even when generated artifacts explicitly include values.

Renovate owns each matrix image once, including maintenance-line repositories; image updates
require dashboard approval and native revalidation. QuadletLens's maintained-line discovery
catalogue signals upstream patch drift independently of image publication. Discovery is not
admission. See [dependency policy](../../../docs/dependency-policy.md).

## Revalidating a limitation

The manual `Podman limitation revalidation` workflow runs only from the default branch. Select one
active candidate or `all`; candidates remain serial. When the active catalogue is empty, `all`
succeeds as an explicit no-op and starts no build or privileged runner. Each job runs the accepted baseline and candidate on
one disposable runner. The helper rejects catalogue drift before a pull, and the runner never edits
the matrix or limitation ledger.

For a local equivalent, use a disposable amd64 Linux host with Podman and `getcap`:

```console
cargo build --locked --package boxferry --bin boxferry --features podman
sudo env BOXFERRY_BIN="$PWD/target/debug/boxferry" \
  bash scripts/podman-live-conformance.sh \
    --profile limitation-revalidation \
    --candidate-cell podman-ubi-9-rootless \
    --engine podman
```

The run writes its bounded decision record to
`target/podman-revalidation/evidence-v1.json`. Raw diagnostics remain in ignored local storage and
must never be uploaded or committed. Schema validation rebinds the candidate, catalogues,
repository commit, results, and failure chronology. A passing result still changes no coverage.
Initialization failures retain the bounded requested candidate, repository commit, and input
catalogue hashes whenever those values can be recovered; an unbound parse failure cannot validate
as evidence for a valid workflow candidate.

Admission is a separate reviewed change. For a passing candidate, replace the exact baseline digest
in `matrix.tsv`, delete only its matching limitation, record the reviewed workflow run and artifact,
and remove the candidate entry. A failed candidate retains its baseline and limitation. The five
decisions from run `34418537575` are archived in `revalidation/34418537575/`; none changed
accepted coverage. Its original matrix is archived alongside the decision so active patch
updates cannot relabel or invalidate the historical evidence. Any shared
reproducer must be minimized and reviewed for private paths, addresses, environment values, and
runtime identifiers.
