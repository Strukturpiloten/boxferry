#!/usr/bin/env bash

set -Eeuo pipefail

if (($# < 1 || $# > 2)); then
  printf 'Usage: %s VERSION [CHANGELOG]\n' "$0" >&2
  exit 2
fi

readonly version="$1"
readonly changelog="${2:-CHANGELOG.md}"

if [[ ! "${version}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  printf 'Release version must use major.minor.patch form: %s\n' "${version}" >&2
  exit 2
fi
if [[ ! -f "${changelog}" ]]; then
  printf 'Changelog does not exist: %s\n' "${changelog}" >&2
  exit 2
fi

set +e
awk -v version="${version}" '
  BEGIN {
    prefix = "## [" version "]"
  }

  index($0, prefix) == 1 {
    if (found) {
      failure = 6
      exit
    }
    suffix = substr($0, length(prefix) + 1)
    if (suffix !~ /^(\([^()]+\))? - [0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]$/) {
      failure = 7
      exit
    }
    found = 1
    next
  }

  found && /^## \[/ {
    exit
  }

  found {
    lines[++count] = $0
    if ($0 ~ /[^[:space:]]/) {
      if (!first) {
        first = count
      }
      last = count
    }
  }

  END {
    if (failure) {
      exit failure
    }
    if (!found) {
      exit 4
    }
    if (!first) {
      exit 5
    }
    for (line = first; line <= last; line++) {
      print lines[line]
    }
  }
' "${changelog}"
status=$?
set -e

case "${status}" in
  0)
    ;;
  4)
    printf 'Changelog has no release section for version %s.\n' "${version}" >&2
    ;;
  5)
    printf 'Changelog release section for version %s is empty.\n' "${version}" >&2
    ;;
  6)
    printf 'Changelog contains more than one release section for version %s.\n' "${version}" >&2
    ;;
  7)
    printf 'Changelog release heading for version %s must include a YYYY-MM-DD date.\n' \
      "${version}" >&2
    ;;
  *)
    printf 'Could not extract release notes for version %s (exit %s).\n' \
      "${version}" "${status}" >&2
    ;;
esac

exit "${status}"
