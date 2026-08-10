#!/usr/bin/env bash

set -euo pipefail

package="${1:?package name is required}"
version="${2:?package version is required}"

if [[ ! "${package}" =~ ^[a-z0-9]+(-[a-z0-9]+)*$ ]]; then
  echo "Invalid package name: ${package}" >&2
  exit 1
fi
if [[ ! "${version}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "Invalid package version: ${version}" >&2
  exit 1
fi

registry_url="https://crates.io/api/v1/crates/${package}/${version}"
user_agent="BoxFerry release workflow (+https://github.com/Strukturpiloten/boxferry)"

registry_status() {
  curl --proto '=https' \
    --tlsv1.2 \
    --silent \
    --show-error \
    --user-agent "${user_agent}" \
    --retry 3 \
    --retry-all-errors \
    --connect-timeout 10 \
    --max-time 60 \
    --output /dev/null \
    --write-out '%{http_code}' \
    "${registry_url}"
}

status="$(registry_status)"
case "${status}" in
  200)
    echo "${package} ${version} is already published; continuing the release."
    exit 0
    ;;
  404)
    ;;
  *)
    echo "Unexpected crates.io response for ${package} ${version}: HTTP ${status}." >&2
    exit 1
    ;;
esac

cargo publish --locked --package "${package}"

for _attempt in {1..60}; do
  status="$(registry_status)"
  case "${status}" in
    200)
      echo "${package} ${version} is visible on crates.io."
      exit 0
      ;;
    404)
      sleep 5
      ;;
    *)
      echo "Unexpected crates.io response while waiting for ${package} ${version}: HTTP ${status}." >&2
      exit 1
      ;;
  esac
done

echo "Timed out waiting for ${package} ${version} to become visible on crates.io." >&2
exit 1

