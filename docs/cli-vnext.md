# vNext command-line interface contract

## Status

This is the accepted implementation contract recorded by
[ADR 0018](decisions/0018-generic-cli-and-diagnostic-support-bundle.md) and amended by
[ADR 0019](decisions/0019-generic-cli-route-registry.md) and
[ADR 0021](decisions/0021-automatic-local-error-report-names.md). `convert` and `validate` are the only
document-conversion commands.

## Command surface

The implemented top-level commands are:

- `boxferry convert` converts one explicit source type to one explicit target type.
- `boxferry validate` parses and plans without writing generated artifacts.
- `boxferry capabilities` reports supported formats, versions, and fidelity boundaries.
- `boxferry help` and `boxferry help <COMMAND>` show help.
- `boxferry version` prints version information.

`-h`, `--help`, and `--version` remain visible conventional forms.
Subcommand help is contextual: `boxferry convert --help` describes `convert`, not only the root
command.

## Generic conversion

`convert` requires all of the following:

```console
boxferry convert \
  --input-type compose \
  --output-type quadlet \
  --input-file compose.yaml \
  --output-directory ./quadlet-output
```

The current registry exposes only `compose -> quadlet` and `quadlet -> compose`; other selected
pairs fail as unavailable. Quadlet input requires `--project-name` and does not accept stdin,
Compose interpolation, profile, environment, or project-directory options. Quadlet output uses
the Podman selectors and grouping options documented below. Compose output writes exactly
`compose.yaml` against the rolling Compose Specification target. It requires no provider or
runtime flags, never infers installed tools, and does not promise compatibility with every
historical Compose consumer. Its internal BoxFerry profile revision is not a Compose
Specification release version and is reported as `rolling`, never as a provider/version choice.

`--input-file` and `--input-directory` are repeatable. Their occurrences form one ordered input
sequence, even when the two options are interleaved:

```console
--input-file base.yaml \
--input-directory overlays/ \
--input-file production.yaml
```

The directory expansion is inserted at that exact position. An implementation must retain the
occurrence indices instead of collecting both options independently and concatenating them.

### Compose directory discovery

A Compose input directory is scanned non-recursively. Conventional names are alternatives, not
files to merge together. BoxFerry selects the first existing regular file in this order:

1. `compose.yaml`
2. `compose.yml`
3. `podman-compose.yaml`
4. `podman-compose.yml`
5. `docker-compose.yaml`
6. `docker-compose.yml`

This order prefers vendor-neutral names, then Podman over Docker, and `.yaml` over `.yml`.
BoxFerry reports the selected file and, in verbose mode, the ignored candidates. It fails when no
candidate exists. Override files are never discovered or merged implicitly; callers add them with
another ordered input option.

An existing conventional candidate that is not a regular non-symlink file is ignored and recorded
in the same verbose/report discovery detail; scanning continues to the next conventional name.

### Quadlet directory discovery

A Quadlet input directory contributes every supported unit extension in deterministic lexical
filename order. Discovery remains non-recursive. Unsupported files remain outside the input set
and are reported in verbose discovery details.

## Compose preprocessing

Compose `${NAME}` interpolation and runtime container environment are separate concepts.
Interpolation remains opt-in with `--interpolate` during the compatibility period and accepts:

- repeatable `--env-file PATH`; later files override earlier files;
- repeatable `--env NAME=VALUE`; explicit values override environment files; and
- `--env NAME`, which authorizes reading only that named process variable.

`--env-file` accepts UTF-8 blank lines, comments whose first non-whitespace character is `#`, and
`NAME=VALUE` assignments split at the first `=`. Names use the Compose interpolation variable
grammar. The value is otherwise literal after removal of a trailing carriage return; quoting,
escaping, `export`, multiline values, and nested interpolation are not interpreted. This is a
strict BoxFerry input format, not a claim of full Docker Compose `.env` compatibility.

An unset expression without a Compose default retains its native
`compose.interpolation.unset-variable` warning and substitutes an empty string. The warning remains
visible if that empty result subsequently causes an importer error.

BoxFerry does not import the complete process environment or discover `.env` implicitly. Values
from every explicit interpolation source are protected in reports. Command-line values may be
visible in shell history, process lists, and CI logs, so documentation must discourage
`--env NAME=VALUE` for secrets.

Each Compose document is interpolated before the documents are merged. Service-level Compose
`env_file` declarations are runtime configuration and are preserved without reading their
contents. Earlier `--variable`, `--variable-from-environment`, and `--interpolation-env-file`
proposals are replaced by the smaller input contract above.

The environment options require `--interpolate`. A future change to default interpolation needs a
separate compatibility decision.

### Project directory

`--project-directory PATH` supplies the base for relative bind mounts, build contexts, environment
file declarations, and other source paths. It does not change BoxFerry's process working
directory. The default is the parent of the first resolved Compose input file. A project directory
is required for stdin because stdin has no parent path.

### Profiles

Without profile options, only unprofiled services are active. `--profile NAME` is repeatable.
`--all-profiles` explicitly activates every profile and all unprofiled services.

All-profile activation is syntactically valid but may select mutually exclusive alternatives,
such as two database services. BoxFerry never chooses an alternative. It reports active profiles
and fails if the selected target cannot represent the combined application safely.

## Runtime topology and file layout

Runtime grouping and physical output layout are independent:

- `--quadlet-grouping separate` keeps services in separate containers and is the default.
- `--quadlet-grouping pod` requests one compatible Podman pod.
- `--pod-name NAME` names that pod.
- `--output-layout files` writes ordinary Quadlet files and is the only native layout in this
  release.

Quadlet does not require a pod. Automatically placing every Compose or Docker service into one
pod changes network, namespace, port, and lifecycle semantics. Kubernetes output uses workload
pod templates as required by Kubernetes; it does not imply that one application becomes one pod.

When pod grouping is selected, BoxFerry first uses `--pod-name`, then a valid resolved application
or Compose project name. It fails and requests `--pod-name` when no valid name exists. It rejects
`--pod-name` with separate grouping. Podman's reviewed manuals do not define a `.quadlets` bundle.
A future BoxFerry transport archive and Kubernetes or Podman Kube YAML belong to separate output
types and contracts.

## Target version selectors

Podman minimum and maximum selectors accept `major.minor` or `major.minor.patch`. Major-only forms
are invalid.

- A minimum such as `5.4` resolves to the lowest reviewed patch in that line, normally `5.4.0`.
- A maximum such as `6.0` resolves to the greatest `6.0.z` present in BoxFerry's finite capability
  catalogue.
- A shortened maximum never promises compatibility with unknown future patches.

Human and JSON reports include both the requested selectors and their resolved finite bounds.

## Output safety

`--output-directory` is required for every conversion. BoxFerry never writes generated artifacts
to the working directory implicitly.

The output path may be absent or an existing empty, non-symlink directory. BoxFerry creates an
absent directory only after conversion and policy authorization. Every directory entry, including
a dotfile or child directory, makes an existing directory nonempty. Generated files use create-new
writes; merge and overwrite behavior remain unsupported.

## Console presentation

There is no `--silent` alias. Presentation modes are mutually exclusive:

- normal human output is the default;
- `--verbose` adds discovery, resolution, route-fidelity, and per-file detail;
- `--quiet` suppresses progress and success text but retains warnings and errors; and
- `--console-format json` writes one complete JSON result without human progress text.

Human modes group each stable rule with its findings and static help. Help is always visible
with the related diagnostics and never requires `--verbose`; it contains option names but no
source or environment values. JSON clients receive the same `help` field.

Human diagnostic groups render as `CODE NAME [severity]`, one shared explanation, fields common to
every finding, a numbered list of only the varying evidence, help, and `boxferry explain CODE`.
Native Lens rule identifiers remain JSON provenance instead of terminal noise. Groups are sorted by
code and findings by input and source position. Loss-policy authorization never removes the warning
for an approximation or partial result. Progress, diagnostics, and the final result are separated
by blank lines. Normal and verbose output flush diagnostics before writing an explicit success,
blocked, or failure line as the last line. A blocked or failed final line names the primary causal
rule and explanation.

For `capabilities`, each JSON route includes stable route-specific target selectors and
fidelity-boundary fields. Compose-to-Quadlet reports finite Podman bounds and `pod-grouping` as
approximate; Quadlet-to-Compose reports the rolling Compose Specification target and
environment-file reconstruction as approximate. Both routes identify unsupported fields as
policy-controlled.

Normal output includes selected inputs, the route, the resolved application name, stage
summaries, every non-exact diagnostic, and the write summary. Verbose output additionally includes resolved input
order, ignored directory candidates, selected profiles, environment variable names and sources
without values, resolved target bounds, concise route fidelity boundaries, and every written path.

Verbosity affects only live human presentation. It never changes diagnostics or a report file.
Showing only `source import failed with N diagnostic(s)` is a defect: every contained diagnostic
must remain available in normal human output and structured JSON.

### Streams

In human mode, progress and success messages use standard output. Warnings and errors use standard
error. Quiet mode leaves standard output empty and retains diagnostics on standard error.

JSON mode writes exactly one document to standard output. It contains structured diagnostics and
does not mix in human progress. Standard error is reserved for failures that occur before a JSON
result can be constructed. Scripts should normally use JSON and stable exit statuses; `--quiet`
is suitable when a script needs only filesystem effects and an exit status.

The JSON result includes a schema version, BoxFerry version, status, stable exit category, primary
diagnostic code, causal failure summary, resolved input order, source and target types, application
identity, compatibility bounds, fidelity counts, structured diagnostics, output paths, and
redacted provenance. Each diagnostic includes its BoxFerry code and name, optional native source
code, severity, summary, help, fields, and spans.

`--report-file PATH` writes the same complete canonical JSON report independently of console mode.
The value-less `--generate-error-report` creates a local stored ZIP with exactly `README.md` and
`report.json`; `--error-report-directory DIR` optionally selects its existing non-symlink output
directory, otherwise the current directory is used. The local-clock name is
`boxferry-error-report-YYYY-MM-DD_HH-MM-SS.zip`, with create-new retries through `-99` for a
same-second collision (the base name plus 99 suffixes, 100 candidates). Normal and verbose output append its absolute path, quiet output contains
only that path, and JSON console output adds optional `error_report_path`. Saved and bundled reports
exclude that host-local path. When both outputs are requested, BoxFerry writes `--report-file`
first and then the support bundle; the bundle therefore records a report-file failure, while a
later bundle-write failure is present in the final console report only. Either requested-output
failure makes an otherwise successful conversion fail. The privacy-safe diagnostic support bundle
is independent of presentation modes; see [Error reports](error-reports.md).

## Full example

```console
boxferry convert \
  --input-type compose \
  --output-type quadlet \
  --input-directory ./deployment \
  --input-file ./production.override.yaml \
  --project-directory . \
  --interpolate \
  --env-file ./deployment.env \
  --env IMMICH_VERSION \
  --output-layout files \
  --podman-minimum-version 5.4 \
  --podman-maximum-version 6.0 \
  --output-directory ./quadlet-output
```

An explicit compatible single-pod conversion adds:

```console
--quadlet-grouping pod --pod-name immich
```

For a script that consumes the result:

```console
boxferry convert <OPTIONS> --console-format json >boxferry-result.json
jq -e '.status == "success"' boxferry-result.json
```

The script must still check BoxFerry's process status; a pipeline should not accidentally replace
it with the status of a later command.

## Deferred features

- Additional generic source and target pairs as their adapters become complete.
- Overwrite or merge into nonempty output directories.
- Default interpolation after the compatibility period.
- A non-native BoxFerry single-file transport archive with an explicit unpack/install contract.
