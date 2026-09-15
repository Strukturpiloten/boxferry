#!/usr/bin/env bash
# Shared Docker Compose provider metadata and checksum-verified installer.

boxferry_load_compose_provider_metadata() {
  # renovate: datasource=github-releases depName=docker/compose
  local -r expected_version="5.5.0"
  local -r expected_sha256="c57ab918abd5b05ca7e7d0f275875dd1330a695074f309dc9eab1b49efafcd4b"
  local -r expected_url="https://github.com/docker/compose/releases/download/v${expected_version}/docker-compose-linux-x86_64"

  local declaration metadata_declared=false readonly_declaration variable
  for variable in BOXFERRY_COMPOSE_PROVIDER_VERSION BOXFERRY_COMPOSE_PROVIDER_SHA256 BOXFERRY_COMPOSE_PROVIDER_URL; do
    if declare -p "${variable}" > /dev/null 2>&1; then
      metadata_declared=true
      break
    fi
  done
  if [[ ${metadata_declared} != true ]]; then
    while IFS= read -r readonly_declaration; do
      case ${readonly_declaration} in
        declare\ -*\ BOXFERRY_COMPOSE_PROVIDER_VERSION | declare\ -*\ BOXFERRY_COMPOSE_PROVIDER_VERSION=*)
          metadata_declared=true
          break
          ;;
        declare\ -*\ BOXFERRY_COMPOSE_PROVIDER_SHA256 | declare\ -*\ BOXFERRY_COMPOSE_PROVIDER_SHA256=*)
          metadata_declared=true
          break
          ;;
        declare\ -*\ BOXFERRY_COMPOSE_PROVIDER_URL | declare\ -*\ BOXFERRY_COMPOSE_PROVIDER_URL=*)
          metadata_declared=true
          break
          ;;
      esac
    done < <(readonly -p)
  fi

  if [[ ${metadata_declared} == true ]]; then
    for variable in BOXFERRY_COMPOSE_PROVIDER_VERSION BOXFERRY_COMPOSE_PROVIDER_SHA256 BOXFERRY_COMPOSE_PROVIDER_URL; do
      if ! declaration=$(declare -p "${variable}" 2> /dev/null); then
        printf 'Docker Compose provider metadata must only come from this library: %s\n' "${variable}" >&2
        return 1
      fi
      if [[ ${declaration} != "declare -r "* && ${declaration} != "declare -rx "* ]]; then
        printf 'Docker Compose provider metadata must only come from this library: %s\n' "${variable}" >&2
        return 1
      fi
    done
    if [[ ${BOXFERRY_COMPOSE_PROVIDER_VERSION-} != "${expected_version}" ||
      ${BOXFERRY_COMPOSE_PROVIDER_SHA256-} != "${expected_sha256}" ||
      ${BOXFERRY_COMPOSE_PROVIDER_URL-} != "${expected_url}" ]]; then
      printf 'Docker Compose provider metadata differs from the canonical library values\n' >&2
      return 1
    fi
    return 0
  fi

  readonly BOXFERRY_COMPOSE_PROVIDER_VERSION="${expected_version}"
  readonly BOXFERRY_COMPOSE_PROVIDER_SHA256="${expected_sha256}"
  readonly BOXFERRY_COMPOSE_PROVIDER_URL="${expected_url}"
}

if ! boxferry_load_compose_provider_metadata; then
  unset -f boxferry_load_compose_provider_metadata
  return 1
fi
unset -f boxferry_load_compose_provider_metadata

boxferry_install_compose_provider() {
  local destination=${1:?destination path required}

  mkdir -p "$(dirname -- "${destination}")"
  curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error \
    --retry 3 --retry-all-errors --connect-timeout 10 --max-time 300 \
    --output "${destination}" "${BOXFERRY_COMPOSE_PROVIDER_URL}"
  printf '%s  %s\n' "${BOXFERRY_COMPOSE_PROVIDER_SHA256}" "${destination}" |
    sha256sum --check --strict
  chmod 0755 "${destination}"
}
