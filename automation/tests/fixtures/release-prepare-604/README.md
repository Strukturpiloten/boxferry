# Historical release-preparation postimages

These three non-executable `.txt` snapshots preserve the exact repository-owned #604
postimages admitted by [PR #608](https://github.com/Strukturpiloten/boxferry/pull/608)
at commit `fba206d732ea7ed9373386ae58101e76e1e3c9fb`.
The adjacent provenance records original paths, modes, Git blobs, sizes and SHA-256
values. Bytes are unmodified MPL-2.0 source, redistributable under the root license;
no external oracle was used.

`ProducerClosureTests` reads these files as inert bytes to construct historical
Git trees for the real verifier. It never imports or executes the snapshots and
requires no historical Git objects, fetch or network, including in shallow and
archived checkouts. Current release code remains independently maintained.
The existing twelve-file zero-age-delta repair inventory is unchanged.

Renovate's existing `**/tests/**` exclusion keeps these historical pins and
source snapshots outside live dependency extraction.
