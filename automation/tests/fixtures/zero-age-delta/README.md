# Zero-age-delta repair snapshots

These `.txt` payloads preserve the exact independently reviewed Podman issue158/PR159 and Quadlet issue174/PR175 A-pinned repair bytes. They are inert data, never executable tests or imported modules. `provenance.json` records hashes, original modes and custody for eight repair payloads and four unchanged manifest/policy blobs. The native files remain owned by their Lens repositories. Future automation revisions are substituted only into independently recognized operational slots.

Existing Renovate `**/tests/**` exclusion covers this directory; no manager or extraction changes are required.
