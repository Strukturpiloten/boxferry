#!/usr/bin/env bash
# Manual exact-candidate hosted proof; use a disposable runner, never a production host.
set -Eeuo pipefail

if (($# != 2)); then
  printf 'Usage: %s <reviewed-matrix-cell> <boxferry-binary>\n' "$0" >&2
  exit 2
fi
case "$1" in
  podman-6.1-rootful | podman-6.1-rootless) ;;
  *)
    printf 'Not a reviewed cleanup-regression cell: %s\n' "$1" >&2
    exit 2
    ;;
esac
cell=$1
binary=$2
[[ -x "${binary}" ]] || {
  printf 'BoxFerry binary is not executable.\n' >&2
  exit 2
}

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
cd -- "${script_directory}/.."
image="$(awk -F '\t' -v selected="${cell}" '$1 == selected { print $2 }' \
  fixtures/conformance/podman-live/matrix.tsv)"
[[ "${image}" == *@sha256:* ]] || {
  printf 'Missing reviewed digest for %s.\n' "${cell}" >&2
  exit 2
}

sha256sum "${binary}"
printf 'Exact candidate: %s; hosted run: %s/%s; reviewed image: %s\n' \
  "${GITHUB_SHA:-local}" "${GITHUB_RUN_ID:-local}" "${GITHUB_RUN_ATTEMPT:-1}" "${image}"
if [[ "${cell}" == podman-6.1-rootless ]]; then
  # Prove the user store before privileged Podman can touch the runner's
  # default /run/user/<uid>/crun directory.
  rootless_mode="$(podman info --format '{{.Host.Security.Rootless}}')"
  printf 'Rootless host Podman: %s; rootless=%s\n' "$(podman --version)" "${rootless_mode}"
  [[ "${rootless_mode}" == true ]]
  podman pull "${image}"
  bash scripts/test-podman-live-outer-storage-native.sh "${image}"
fi
rootful_mode="$(sudo podman info --format '{{.Host.Security.Rootless}}')"
printf 'Rootful host Podman: %s; rootless=%s\n' "$(sudo podman --version)" "${rootful_mode}"
[[ "${rootful_mode}" == false ]]
sudo podman pull "${image}"
sudo podman image inspect --format '{{json .Config.Volumes}}' "${image}" |
  jq -e 'type == "object" and length > 0' > /dev/null

for iteration in 1 2; do
  before="$(sudo podman volume ls --format '{{.Name}}' | sort)"
  sudo env BOXFERRY_BIN="${binary}" bash scripts/podman-live-conformance.sh \
    --profile smoke --matrix-cell "${cell}" --engine podman
  after="$(sudo podman volume ls --format '{{.Name}}' | sort)"
  owned_containers="$(sudo podman ps --all --quiet --filter label=io.boxferry.live.run)"
  owned_volumes="$(sudo podman volume ls --quiet --filter label=io.boxferry.live.run)"
  if [[ "${after}" != "${before}" || -n "${owned_containers}" || -n "${owned_volumes}" ]]; then
    printf 'Run-owned storage or containers survived repetition %s.\n' "${iteration}" >&2
    exit 1
  fi
  printf 'Nested cleanup repetition %s passed; host volume count=%s\n' \
    "${iteration}" "$(sudo podman volume ls --quiet | wc -l)"
  sudo podman system df
done

sudo bash scripts/test-podman-live-outer-storage-native.sh "${image}"
