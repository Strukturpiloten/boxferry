#!/usr/bin/env bash
set -Eeuo pipefail

if [[ "$#" -lt 2 || "$#" -gt 4 ]]; then
  printf 'usage: %s EXPECTED_VERSION OUTPUT_DIRECTORY [RESOURCE_PREFIX] [PODMAN_EXECUTABLE]\n' "$0" >&2
  exit 64
fi

expected_version="$1"
output_directory="$2"
resource_prefix="${3:-boxferry-conformance}"
podman_executable="${4:-podman}"
podman_command=("$podman_executable")
work_directory="$(mktemp -d)"

cleanup() {
  "${podman_command[@]}" pod rm --force "${resource_prefix}-pod" >/dev/null 2>&1 || true
  "${podman_command[@]}" volume rm --force "${resource_prefix}-data" >/dev/null 2>&1 || true
  "${podman_command[@]}" network rm --force "${resource_prefix}-network" >/dev/null 2>&1 || true
  "${podman_command[@]}" image rm --force "localhost/${resource_prefix}:1" >/dev/null 2>&1 || true
  rm -rf -- "$work_directory"
}
trap cleanup EXIT

actual_version="$("${podman_command[@]}" version --format '{{.Client.Version}}')"
if [[ "$actual_version" != "$expected_version" ]]; then
  printf 'expected Podman %s but image contains %s\n' "$expected_version" "$actual_version" >&2
  exit 65
fi

mkdir -p -- "$output_directory" "$work_directory/rootfs"
tar -C "$work_directory/rootfs" -cf "$work_directory/rootfs.tar" .
"${podman_command[@]}" import \
  --change LABEL=com.example.image=runtime-matrix \
  "$work_directory/rootfs.tar" \
  "localhost/${resource_prefix}:1" \
  >/dev/null
"${podman_command[@]}" network create "${resource_prefix}-network" >/dev/null
"${podman_command[@]}" volume create "${resource_prefix}-data" >/dev/null
"${podman_command[@]}" pod create \
  --name "${resource_prefix}-pod" \
  --network "${resource_prefix}-network" \
  --publish 127.0.0.1:18080:8080 \
  >/dev/null
"${podman_command[@]}" create \
  --name "${resource_prefix}-web" \
  --pod "${resource_prefix}-pod" \
  --env BOXFERRY_MODE=matrix \
  --label com.example.boxferry=runtime-matrix \
  --user 1001:1002 \
  --workdir /srv/runtime \
  --read-only \
  --restart on-failure:4 \
  --entrypoint /bin/true \
  --health-cmd /bin/true \
  --health-interval 30s \
  --health-timeout 2s \
  --health-retries 4 \
  --health-start-period 5s \
  --volume "${resource_prefix}-data:/data:ro,Z" \
  "localhost/${resource_prefix}:1" \
  --serve \
  >/dev/null

read -r -a pod_member_ids <<< "$(
  "${podman_command[@]}" pod inspect "${resource_prefix}-pod" --format '{{range .Containers}}{{.Id}} {{end}}'
)"

"${podman_command[@]}" container inspect -- "${pod_member_ids[@]}" >"${output_directory}/containers.json"
"${podman_command[@]}" image inspect -- "localhost/${resource_prefix}:1" >"${output_directory}/images.json"
"${podman_command[@]}" network inspect -- "${resource_prefix}-network" >"${output_directory}/networks.json"
"${podman_command[@]}" volume inspect -- "${resource_prefix}-data" >"${output_directory}/volumes.json"
"${podman_command[@]}" pod inspect -- "${resource_prefix}-pod" >"${output_directory}/pods.json"
printf '%s\n' "$actual_version" >"${output_directory}/version.txt"
chmod 0644 "${output_directory}"/*
