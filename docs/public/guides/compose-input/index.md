# Compose input

Read one or more Compose documents, then choose the output you need:

- [Compose output](../convert/compose-to-compose/) merges and writes canonical Compose.
- [Quadlet output](../convert/compose-to-quadlet/) converts the application to Quadlet files.

Repeat `--input-file` to control merge order, or use `--input-directory` to discover input files.
Interpolation is opt-in: add `--interpolate` and provide values explicitly with `--env-file` or
`--env` when the selected route needs resolved values.
