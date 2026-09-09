# Podman live conformance catalogues

These files are executable compatibility evidence, not deployment input.

- `matrix.tsv` is the accepted immutable amd64 inventory. Its columns are cell ID, image, declared
  Podman/package version, distribution, root mode, lane, and architecture.
- `scenarios.tsv` inventories assertions exercised by the shared runner.
- `limitations.tsv` names accepted image-specific exceptions. A limited row claims no resource
  coverage.
- `candidates.toml` binds each limited baseline to one replacement digest and immutable
  container-repository source evidence. It is pending evidence, not accepted support data.
- `apply-target-containers.conf` configures the disposable nested apply target.
- `capture_proxy.py` is the privacy boundary for explicitly authorized cassette capture.

## Revalidating a limitation

The manual `Podman limitation revalidation` workflow runs only from the default branch. Select one
candidate or `all`; candidates remain serial. Each job runs the accepted baseline and candidate on
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
and remove the candidate entry. A failed candidate retains its baseline and limitation. Any shared
reproducer must be minimized and reviewed for private paths, addresses, environment values, and
runtime identifiers.
