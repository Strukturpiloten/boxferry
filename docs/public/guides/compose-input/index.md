# Compose input

Use Compose input when application intent lives in one or more Compose files. Repeat
`--input-file` to control merge order, or use `--input-directory` to discover input files.

- [Compose output](../convert/compose-to-compose/) merges, imports into the neutral model, and
  writes canonical Compose.
- [Podman output](../convert/compose-to-podman/) writes a reviewable plan and command script.
- [Quadlet output](../convert/compose-to-quadlet/) converts the application to Quadlet files.

## Production values and interpolation

Supplying `--env-file` or `--env` enables interpolation. Provide deployment values through an
explicit file:

```dotenv
IMAGE_TAG=2026.08.24
RESTART_POLICY=always
```

Add one deployment override with `--env LOG_LEVEL=warning`. Later files override earlier files,
and `--env` overrides the files. For a sensitive process value, use `--env REGISTRY_TOKEN`;
BoxFerry reads only that named variable and redacts it from reports. A missing named process
variable is an error.

Use `--interpolate` alone to resolve referenced variables from the process environment. When
combined with `--env-file` or `--env`, explicit values take precedence and process values are a
fallback. Process variables are looked up only as expressions request them. Without any of these
options, expressions remain literal. BoxFerry never reads an implicit `.env` file.

If interpolation supplies the top-level Compose `name`, generated artifacts retain that
application identity, while console and diagnostic reports show `<redacted>` for its name.

Do not confuse interpolation input with a service-level Compose `env_file:` declaration.
`--env-file` resolves `${NAME}` while converting; `env_file:` tells the target workload where to
load container environment at runtime and may require target-specific approximation.

These interpolation inputs do not authorize copied service environment values in generated files.
Use `--environment-values include` only when the target Compose or Quadlet artifact must contain
them; the default withholds each named value and reports the target prerequisite.

## Before production conversion

- Set `--project-directory` when bind mounts or service `env_file:` paths are relative to another
  checkout directory.
- Inspect secret and config diagnostics; a source path is not portable secret content.
- Keep unresolved variables out of the neutral model by interpolating required values explicitly.
