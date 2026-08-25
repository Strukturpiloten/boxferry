#!/usr/bin/env bash

set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repository_root="$(cd -- "${script_directory}/.." && pwd -P)"
readonly repository_root

cd -- "${repository_root}"

if [[ "${BOXFERRY_RELEASE_METADATA_TEST_MODE:-0}" == "1" ]]; then
  version="${BOXFERRY_TEST_VERSION:?test version is required}"
  changelog="${BOXFERRY_TEST_CHANGELOG:?test changelog is required}"
  latest_tag="${BOXFERRY_TEST_LATEST_TAG:-}"
else
  metadata="$(cargo metadata --locked --no-deps --format-version 1)"
  readonly metadata

  if ! version="$(
    jq -er '
      [.packages[].version]
      | unique
      | if length == 1
        then .[0]
        else error("workspace packages must use one version")
        end
    ' <<< "${metadata}" 2> /dev/null
  )"; then
    echo 'Release metadata must declare one common workspace package version.' >&2
    exit 1
  fi

  readonly packages='[
    "boxferry-model",
    "boxferry-engine",
    "boxferry-compose",
    "boxferry-podman",
    "boxferry-quadlet",
    "boxferry"
  ]'

  if ! metadata_validation="$(
    jq -er --argjson expected "${packages}" '
      ([.packages[].name] | sort) as $actual |
      ([.packages[] | select(.publish == []) | .name] | sort) as $unpublishable |
      if $actual != ($expected | sort) then
        "expected packages " + ($expected | sort | join(", ")) +
        "; found " + ($actual | join(", "))
      elif ($unpublishable | length) != 0 then
        "packages must be publishable: " + ($unpublishable | join(", "))
      else
        "ok"
      end
    ' <<< "${metadata}" 2> /dev/null
  )"; then
    echo 'Could not inspect Cargo release package metadata.' >&2
    exit 1
  fi
  readonly metadata_validation

  if [[ "${metadata_validation}" != "ok" ]]; then
    echo "Release package metadata is invalid: ${metadata_validation}." >&2
    exit 1
  fi

  changelog=CHANGELOG.md
  latest_tag="$(git describe --tags --abbrev=0 --match 'v[0-9]*' 2> /dev/null || true)"
fi
readonly version changelog latest_tag

if [[ ! -f "${changelog}" ]]; then
  printf 'Changelog does not exist: %s\n' "${changelog}" >&2
  exit 1
fi

unreleased_count="$(
  awk '$0 == "## [Unreleased]" { count += 1 } END { print count + 0 }' "${changelog}"
)"
if [[ "${unreleased_count}" != "1" ]]; then
  printf 'CHANGELOG.md must contain exactly one Unreleased section; found %s.\n' \
    "${unreleased_count}" >&2
  exit 1
fi

newest_release="$(
  awk '
    /^## \[/ && $0 != "## [Unreleased]" {
      heading = $0
      sub(/^## \[/, "", heading)
      sub(/\].*$/, "", heading)
      print heading
      exit
    }
  ' "${changelog}"
)"
if [[ "${newest_release}" != "${version}" ]]; then
  printf 'Newest CHANGELOG.md release %s must match workspace version %s.\n' \
    "${newest_release:-<missing>}" "${version}" >&2
  printf 'Record pending changes under Unreleased; release-plz owns numbered release sections.\n' >&2
  exit 1
fi

release_heading_count="$(
  awk -v version="${version}" '
    /^## \[/ {
      heading = $0
      sub(/^## \[/, "", heading)
      sub(/\].*$/, "", heading)
      if (heading == version) count += 1
    }
    END { print count + 0 }
  ' "${changelog}"
)"
if [[ "${release_heading_count}" != "1" ]]; then
  printf 'CHANGELOG.md must contain exactly one release section for %s; found %s.\n' \
    "${version}" "${release_heading_count}" >&2
  exit 1
fi

release_notes="$(bash scripts/extract-release-notes.sh "${version}" "${changelog}")"
if ! grep --quiet '[[:alnum:]]' <<< "${release_notes}"; then
  printf 'CHANGELOG.md release section for %s contains no usable notes.\n' "${version}" >&2
  exit 1
fi

if [[ -n "${latest_tag}" && "${latest_tag#v}" != "${version}" ]]; then
  unreleased_notes="$(
    awk '
      $0 == "## [Unreleased]" {
        reading = 1
        next
      }
      reading && /^## \[/ { exit }
      reading { print }
    ' "${changelog}"
  )"
  if grep --quiet '[[:alnum:]]' <<< "${unreleased_notes}"; then
    printf 'Unreleased must be empty while preparing BoxFerry %s after %s.\n' \
      "${version}" "${latest_tag}" >&2
    printf 'Move every included change into the numbered release section before merging.\n' >&2
    exit 1
  fi
fi

printf 'Release metadata and changelog are valid for BoxFerry %s.\n' "${version}"
