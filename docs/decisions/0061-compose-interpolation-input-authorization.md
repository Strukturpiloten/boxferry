# ADR 0061: Compose interpolation input authorization

- Status: accepted
- Date: 2026-09-26
- Amends: [ADR 0018](0018-generic-cli-and-diagnostic-support-bundle.md) decision 4
- Amends: [ADR 0030](0030-native-compose-same-format-canonicalization.md) decision 2's
  historical `--interpolate` wording; its same-format shortcut was already superseded by ADR 0033

## Context

ADR 0018 made `--interpolate` mandatory alongside `--env` and `--env-file`. This extra flag
obscures the intent of a user who has already supplied interpolation values. An explicit
`--interpolate` without those options should also have useful, predictable behavior.

## Decision

1. `--env NAME=VALUE`, `--env NAME`, and `--env-file FILE` each enable Compose interpolation.
   An explicitly supplied empty environment file still enables it. With none of these options
   and no `--interpolate`, Compose expressions remain unevaluated, including on same-format
   routes. No `.env` file is discovered implicitly.
2. Explicit `--interpolate` authorizes lazy lookups of process variables referenced by Compose
   expressions. It works alone or with supplied inputs. Explicit `--env` values override
   `--env-file` values, later files override earlier files, and those values override the process
   fallback. `--env NAME` reads that named process variable eagerly and fails if absent or not
   valid Unicode. A missing variable in the fallback remains an ordinary Compose interpolation
   diagnostic.
3. `--env` and `--env-file` without explicit `--interpolate` do not authorize fallback lookups
   of other process variables. Reading the process environment never enumerates or imports its
   complete contents; only referenced variable names are queried.
4. Every supplied or resolved value is classified as sensitive. Existing output authorization,
   report redaction, and interpolation error boundaries continue to apply. Literal dollar escapes
   retain their Compose meaning.

## Consequences

Simple conversions need fewer options, while explicit process environment use remains visible.
The same interpolation policy applies to every Compose input route through the generic CLI.
Historical commands with `--interpolate` continue to work, but may now resolve otherwise unset
variables from the process environment.
