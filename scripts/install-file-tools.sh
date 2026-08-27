#!/usr/bin/env bash

set -Eeuo pipefail

readonly install_directory="${1:-/usr/local/bin}"

# renovate: datasource=github-releases depName=tombi-toml/tombi
readonly tombi_version="1.4.1"
# renovate: datasource=github-releases depName=mvdan/sh
readonly shfmt_version="3.13.1"
# renovate: datasource=github-releases depName=koalaman/shellcheck
readonly shellcheck_version="0.11.0"
# renovate: datasource=github-releases depName=hadolint/hadolint
readonly hadolint_version="2.15.1"

case "$(uname -m)" in
  x86_64)
    readonly release_architecture="x86_64"
    readonly shfmt_architecture="amd64"
    readonly tombi_architecture="x86_64-unknown-linux-musl"
    readonly tombi_checksum="e2dc1190e0590b1fd0581fa8f85017215ed9b05a4d85c1cf18e0979b6cf24fe1"
    readonly shfmt_checksum="fb096c5d1ac6beabbdbaa2874d025badb03ee07929f0c9ff67563ce8c75398b1"
    readonly shellcheck_checksum="8c3be12b05d5c177a04c29e3c78ce89ac86f1595681cab149b65b97c4e227198"
    readonly hadolint_checksum="c7187db94eeeeca956519a6af171adc31453941a1e777961f6e680f697c8c507"
    ;;
  aarch64 | arm64)
    readonly release_architecture="aarch64"
    readonly shfmt_architecture="arm64"
    readonly tombi_architecture="aarch64-unknown-linux-musl"
    readonly tombi_checksum="b70750d462954294c354a014bb90399273876b953c21e3a136aafd3d1d41f5e8"
    readonly shfmt_checksum="32d92acaa5cd8abb29fc49dac123dc412442d5713967819d8af2c29f1b3857c7"
    readonly shellcheck_checksum="12b331c1d2db6b9eb13cfca64306b1b157a86eb69db83023e261eaa7e7c14588"
    readonly hadolint_checksum="f6198ef8090f404dbb771abfee086eb8c48ac177f30da7fd3510aca35b344b5d"
    ;;
  *)
    printf 'Unsupported file-tool architecture: %s\n' "$(uname -m)" >&2
    exit 1
    ;;
esac

temporary_directory="$(mktemp -d)"
readonly temporary_directory
trap 'rm -r -- "${temporary_directory}"' EXIT

download() {
  local url=$1
  local destination=$2
  local checksum=$3

  printf 'Downloading %s\n' "${url##*/}"
  curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error \
    --output "${destination}" "${url}"
  printf '%s  %s\n' "${checksum}" "${destination}" | sha256sum --check --status
}

install -d "${install_directory}"

tombi_archive="${temporary_directory}/tombi.tar.gz"
download \
  "https://github.com/tombi-toml/tombi/releases/download/v${tombi_version}/tombi-cli-${tombi_version}-${tombi_architecture}.tar.gz" \
  "${tombi_archive}" "${tombi_checksum}"
tar --extract --gzip --file "${tombi_archive}" --directory "${temporary_directory}"
install -m 0755 \
  "${temporary_directory}/tombi-cli-${tombi_version}-${tombi_architecture}/tombi" \
  "${install_directory}/tombi"

shfmt_binary="${temporary_directory}/shfmt"
download \
  "https://github.com/mvdan/sh/releases/download/v${shfmt_version}/shfmt_v${shfmt_version}_linux_${shfmt_architecture}" \
  "${shfmt_binary}" "${shfmt_checksum}"
install -m 0755 "${shfmt_binary}" "${install_directory}/shfmt"

shellcheck_archive="${temporary_directory}/shellcheck.tar.xz"
download \
  "https://github.com/koalaman/shellcheck/releases/download/v${shellcheck_version}/shellcheck-v${shellcheck_version}.linux.${release_architecture}.tar.xz" \
  "${shellcheck_archive}" "${shellcheck_checksum}"
tar --extract --xz --file "${shellcheck_archive}" --directory "${temporary_directory}"
install -m 0755 \
  "${temporary_directory}/shellcheck-v${shellcheck_version}/shellcheck" \
  "${install_directory}/shellcheck"

hadolint_binary="${temporary_directory}/hadolint"
download \
  "https://github.com/hadolint/hadolint/releases/download/v${hadolint_version}/hadolint-linux-${release_architecture}" \
  "${hadolint_binary}" "${hadolint_checksum}"
install -m 0755 "${hadolint_binary}" "${install_directory}/hadolint"

printf 'Installed Tombi %s, shfmt %s, ShellCheck %s, and Hadolint %s.\n' \
  "${tombi_version}" "${shfmt_version}" "${shellcheck_version}" "${hadolint_version}"
