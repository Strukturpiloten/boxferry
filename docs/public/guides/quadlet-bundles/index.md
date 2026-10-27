# Share and extract Quadlet bundles

BoxFerry can package a complete generated Quadlet application in one transport file. Ordinary
native files remain the default. A bundle is **not a native Quadlet unit**: extract it before
reviewing or installing its members with your normal systemd/Podman workflow. BoxFerry does not
install units or run generated commands.

## Create a bundle

Using the literal `compose.yaml` from the [Compose to Quadlet guide](../convert/compose-to-quadlet/):

<!-- boxferry-example: compose-to-quadlet-bundle -->

```console
boxferry convert compose quadlet --input-file compose.yaml --output-layout quadlet-bundle --output-directory bundle-output
```

The output directory contains one `quadlet.bundle`. It retains each native filename, unit type and
contents, including references between `.container`, `.volume`, `.network` and `.pod` members.
The same layout is available for Quadlet and Podman inputs. Grouping and loss policy retain their
usual meaning; packaging does not authorize unsupported intent or change topology.

## Extract unchanged native files

After sharing or copying the transport as `quadlet.bundle`:

<!-- boxferry-example: quadlet-bundle-extract -->

```console
boxferry extract quadlet --input-file quadlet.bundle --output-directory quadlet-output
```

Extraction validates the entire transport, then writes unchanged native members. It preserves
member order, line endings and final-newline state, including native-only configuration that a
semantic conversion might reject. It does not evaluate content or claim native configuration is
valid. The destination must be absent or empty and must not be a symlink. Existing entries,
including dotfiles, are never overwritten. New extraction files and directories use owner-only
Unix permissions. Review the native files before installing them yourself.

Bundling relocates only the included native unit files. Relative bind sources, environment files,
build contexts and other referenced paths remain external dependencies; their interpretation can
depend on the bundle or extracted files' location. Review those paths and required external files
when moving a bundle or extracting it elsewhere. Packaging does not copy their contents or make
relative references location-independent.

## Read a bundle as conversion input

Supply a bundle explicitly with `--input-file` on a Quadlet input route. Conversion expands members
before native parsing and follows the usual neutral-model and loss-policy pipeline. A sole member
can infer the application name from its native filename; multiple members need `--application-name`.
Directory discovery selects native unit files and does not automatically load bundles. Mixed bundle
and native inputs must have distinct native names, including ASCII case distinctions.

The versioned transport uses length-delimited UTF-8 payloads, so payload text can contain `---`
without ambiguity. Version 1 permits 1–4096 members, 8 MiB per member, 16 KiB per JSON header line and
64 MiB total. Unsafe paths, duplicate names, unsupported versions, malformed framing and trailing
content fail before extraction writes anything.

JSON reports distinguish `operation` (`convert`, `validate` or `extract`), physical output artifacts
and native `bundle_members`. Input provenance uses `input_bundle_members` aliases. Extraction has
empty conversion-fidelity counts; conversion keeps every real diagnostic and loss. Payloads stay
out of Debug, reports and diagnostic ZIPs. The transport itself contains application data and may
contain protected values, so share it only with intended recipients.
