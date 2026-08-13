# ADR 0025: Empty output directories and final human status

- Status: accepted
- Date: 2026-08-12
- Amends: [ADR 0004](0004-first-cli-feature-and-write-safety.md)
- Amends: [ADR 0018](0018-generic-cli-and-diagnostic-support-bundle.md)
- Amended by: [ADR 0026](0026-typed-diagnostic-rule-catalogue.md)

## Context

The original write-safety contract required an absent output directory. Development tools and
automation commonly create an empty destination before invoking a converter, so rejecting that
directory adds friction without protecting existing content. Human conversion output also printed
its success result before warnings because progress used standard output and diagnostics used
standard error, leaving no conclusive final line after the diagnostic section.

## Decision

1. `--output-directory` remains required. It may name an absent path whose parent exists or an
   existing empty, non-symlink directory. BoxFerry creates the absent directory.
2. Emptiness means directory enumeration returns no entry. Files, child directories, dotfiles, and
   other directory entries all make the directory nonempty. Regular files and symbolic links are
   not accepted as output directories.
3. Conversion and policy authorization still finish before writes. Every generated file uses a
   create-new write and no existing file is replaced. On failure, BoxFerry removes only files
   created by the invocation. It removes the output directory only when the invocation created it;
   a caller-provided empty directory remains.
4. Normal and verbose conversion presentation has three sections: progress, diagnostics and hints,
   and final result. Blank lines separate sections and individual diagnostics. Standard output is
   flushed before diagnostic writes, standard error is flushed afterward, and an explicit success,
   blocked, or failure result is the final human line.
5. Quiet mode keeps progress and final success text suppressed. JSON presentation remains one
   structured document and does not acquire terminal formatting or a duplicate final line.

## Consequences

- Dev containers, build scripts, and users may prepare an empty destination without weakening the
  no-overwrite guarantee.
- Hidden files are protected exactly like visible files.
- Existing nonempty directories fail before any generated file is opened.
- Human readers can distinguish warnings from the command outcome and see the outcome last.

## Alternatives considered

Keeping the absent-only rule was rejected because an empty directory contains no user content to
protect and is routinely pre-created. Allowing merge or overwrite behavior was rejected because it
would require conflict, rollback, stale-file, and ownership contracts. Sending all human text to a
single stream was rejected because the accepted stdout/stderr roles remain useful to scripts.
