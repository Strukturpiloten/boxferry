#!/usr/bin/env bash

set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repository_root="$(cd -- "${script_directory}/.." && pwd -P)"
readonly repository_root

cd -- "${repository_root}"

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
readonly version

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

if ! bash scripts/extract-release-notes.sh "${version}" > /dev/null; then
  echo "CHANGELOG.md does not contain usable release notes for ${version}." >&2
  exit 1
fi

printf 'Release metadata and changelog are valid for BoxFerry %s.\n' "${version}"
