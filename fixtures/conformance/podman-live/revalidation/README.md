# Podman limitation revalidation decisions

Each numbered directory preserves one reviewed manual workflow decision. Its
`admission.toml` binds the exact default-branch commit, run and conclusion, job
URLs, uploaded artifact identities and archive digests, checked-in evidence
hashes, catalogue hashes, observed metadata and explicit observation gaps,
cleanup results, and follow-up.

The evidence documents are the exact privacy-safe `evidence-v1.json` payloads
validated and uploaded by the named jobs. Raw retained runner diagnostics are
not fixtures and must not be committed.

An outcome may promote a replacement only when every required result is true.
A retained outcome leaves the accepted matrix digest and limitation unchanged.
Active replacement proposals belong only in the parent `candidates.toml`; a
reviewed outcome is removed from that temporary catalogue. Run `34418537575`
retained all five limitations after every replacement remained ineligible at
`replacement-runtime`. It does not claim that later resource, exporter,
re-import, external-apply, SELinux, systemd, or cgroup checks ran.
