#!/usr/bin/env sh
set -eu

if [ "$#" -lt 4 ]; then
  printf 'usage: %s EXPECTED_ENGINE_VERSION OUTPUT_DIRECTORY RESOURCE_PREFIX API_VERSION...\n' "$0" >&2
  exit 64
fi

expected_engine_version="$1"
output_directory="$2"
resource_prefix="$3"
shift 3
work_directory="$(mktemp -d)"
container_name="${resource_prefix}-web"
image_name="${resource_prefix}-image:1"
network_name="${resource_prefix}-network"
volume_name="${resource_prefix}-data"
dockerd_pid=""

cleanup() {
  docker container rm --force "$container_name" >/dev/null 2>&1 || true
  docker volume rm --force "$volume_name" >/dev/null 2>&1 || true
  docker network rm "$network_name" >/dev/null 2>&1 || true
  docker image rm --force "$image_name" >/dev/null 2>&1 || true
  if [ -n "$dockerd_pid" ]; then
    kill "$dockerd_pid" >/dev/null 2>&1 || true
    wait "$dockerd_pid" >/dev/null 2>&1 || true
  fi
  rm -rf -- "$work_directory"
}
trap cleanup EXIT INT TERM

export DOCKER_HOST="unix:///var/run/docker.sock"
export DOCKER_TLS_CERTDIR=""
dockerd-entrypoint.sh --host="$DOCKER_HOST" --tls=false >"$work_directory/dockerd.log" 2>&1 &
dockerd_pid="$!"

ready="false"
attempt=0
while [ "$attempt" -lt 60 ]; do
  if docker info >/dev/null 2>&1; then
    ready="true"
    break
  fi
  if ! kill -0 "$dockerd_pid" >/dev/null 2>&1; then
    break
  fi
  attempt=$((attempt + 1))
  sleep 1
done
if [ "$ready" != "true" ]; then
  printf 'nested Docker daemon did not become ready\n' >&2
  sed -n '1,200p' "$work_directory/dockerd.log" >&2 || true
  exit 65
fi

server_version="$(docker version --format '{{.Server.Version}}')"
server_api="$(docker version --format '{{.Server.APIVersion}}')"
server_minimum_api="$(docker version --format '{{.Server.MinAPIVersion}}')"
if [ "$server_version" != "$expected_engine_version" ]; then
  printf 'expected Docker Engine %s but image contains %s\n' "$expected_engine_version" "$server_version" >&2
  exit 66
fi

mkdir -p -- "$output_directory" "$work_directory/rootfs" "$work_directory/bind-source"
tar -C "$work_directory/rootfs" -cf "$work_directory/rootfs.tar" .
docker image import \
  --change 'ENV BASE=image' \
  --change 'ENTRYPOINT ["/fixture-entrypoint"]' \
  --change 'CMD ["--image-default"]' \
  --change 'USER 1000:1000' \
  --change 'WORKDIR /srv/image' \
  "$work_directory/rootfs.tar" \
  "$image_name" \
  >/dev/null
docker network create "$network_name" >/dev/null
docker volume create "$volume_name" >/dev/null
docker container create \
  --name "$container_name" \
  --network "$network_name" \
  --network-alias web \
  --env BOXFERRY_MODE=matrix \
  --user 1001:1002 \
  --workdir /srv/runtime \
  --read-only \
  --entrypoint /fixture-entrypoint \
  --volume "$volume_name:/data:ro,Z" \
  --volume "$work_directory/bind-source:/srv/fixture:ro" \
  "$image_name" \
  --serve \
  >/dev/null

for api_version in "$@"; do
  api_directory="$output_directory/api-${api_version}"
  mkdir -p -- "$api_directory"
  DOCKER_API_VERSION="$api_version" docker container inspect -- "$container_name" >"$api_directory/containers.json"
  DOCKER_API_VERSION="$api_version" docker image inspect -- "$image_name" >"$api_directory/images.json"
  DOCKER_API_VERSION="$api_version" docker network inspect -- "$network_name" >"$api_directory/networks.json"
  DOCKER_API_VERSION="$api_version" docker volume inspect -- "$volume_name" >"$api_directory/volumes.json"
done

printf '%s\n' "$server_version" >"$output_directory/engine-version.txt"
printf '%s\n' "$server_api" >"$output_directory/engine-api.txt"
printf '%s\n' "$server_minimum_api" >"$output_directory/engine-minimum-api.txt"
find "$output_directory" -type f -exec chmod 0644 {} \;
