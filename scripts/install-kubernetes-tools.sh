#!/usr/bin/env bash

set -Eeuo pipefail

if [[ "$#" -gt 1 ]]; then
  printf 'Usage: %s [install-directory]\n' "$0" >&2
  exit 2
fi
readonly install_directory="${1:-/usr/local/bin}"

# These are development defaults, not the fixed Kubernetes 1.20 conformance profile.
# Each kind record is one exact GitHub tag and one platform asset digest.
# renovate: datasource=github-release-attachments depName=kubernetes-sigs/kind depType=kind-linux-amd64
readonly kind_amd64_release="v0.33.0" kind_amd64_checksum="aee6151561422756b764a4ae28e7f44cda5af5a9eead3cc9985112b1de8d8e0d"
# renovate: datasource=github-release-attachments depName=kubernetes-sigs/kind depType=kind-linux-arm64
readonly kind_arm64_release="v0.33.0" kind_arm64_checksum="20022bee6cfcd5086cb7234d218e3454e6090022f2a8f55d1fa7fcf42c3867a2"
# renovate: datasource=github-releases depName=kubernetes/kubernetes
readonly kubectl_version="1.37.2"
# renovate: datasource=github-releases depName=helm/helm
readonly helm_version="4.3.1"
# renovate: datasource=github-releases depName=kubernetes-sigs/kustomize
readonly kustomize_version="5.8.3"

if [[ "$(uname -s)" != Linux ]]; then
  printf 'Kubernetes development tools require Linux.\n' >&2
  exit 1
fi

# Integrity sources: kind/Kustomize GitHub release asset SHA-256 digests;
# dl.k8s.io/release/vVERSION/bin/linux/ARCH/kubectl.sha256;
# get.helm.sh/helm-vVERSION-linux-ARCH.tar.gz.sha256sum.
# Review both architectures with each version update; never fetch unpinned checksums here.
case "$(uname -m)" in
  x86_64)
    readonly architecture="amd64"
    readonly kind_release="${kind_amd64_release}"
    readonly kind_checksum="${kind_amd64_checksum}"
    readonly kubectl_checksum="c04487523e7aa7d1493f7808174c899db93d7a9cf57531420cd76b9bd415f8f4"
    readonly helm_checksum="0403a88ae20913d11b10a2467b50087721fea7581736da6b9d3992f5357fe54b"
    readonly kustomize_checksum="cb9e31198d3f63b44848bd0afc4c8efc56e6cfef9c93a4a39a211940e6decd61"
    ;;
  aarch64 | arm64)
    readonly architecture="arm64"
    readonly kind_release="${kind_arm64_release}"
    readonly kind_checksum="${kind_arm64_checksum}"
    readonly kubectl_checksum="fcc16f63186659d9722014b7e02d5fdd2d4e669cb8c0ab66365d7d3b8ba03509"
    readonly helm_checksum="8cf8f66ecbc252ac8f7f06cca68aba88508ddfde3a6a34feb1678ad1694b4f88"
    readonly kustomize_checksum="9867b76482cfc6d25b546abc1d2b5a32bd137505aba640649b4b378f64f6d68f"
    ;;
  *)
    printf 'Unsupported Kubernetes-tool architecture: %s\n' "$(uname -m)" >&2
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
    --retry 3 --retry-all-errors --connect-timeout 10 --max-time 300 \
    --output "${destination}" "${url}"
  printf '%s  %s\n' "${checksum}" "${destination}" | sha256sum --check --status
}

# Stage and verify everything before replacing any installed tool.
download \
  "https://github.com/kubernetes-sigs/kind/releases/download/${kind_release}/kind-linux-${architecture}" \
  "${temporary_directory}/kind" "${kind_checksum}"
download \
  "https://dl.k8s.io/release/v${kubectl_version}/bin/linux/${architecture}/kubectl" \
  "${temporary_directory}/kubectl" "${kubectl_checksum}"
download \
  "https://get.helm.sh/helm-v${helm_version}-linux-${architecture}.tar.gz" \
  "${temporary_directory}/helm.tar.gz" "${helm_checksum}"
download \
  "https://github.com/kubernetes-sigs/kustomize/releases/download/kustomize/v${kustomize_version}/kustomize_v${kustomize_version}_linux_${architecture}.tar.gz" \
  "${temporary_directory}/kustomize.tar.gz" "${kustomize_checksum}"

tar --extract --gzip --file "${temporary_directory}/helm.tar.gz" \
  --directory "${temporary_directory}" --strip-components=1 "linux-${architecture}/helm"
tar --extract --gzip --file "${temporary_directory}/kustomize.tar.gz" \
  --directory "${temporary_directory}" kustomize

install -d "${install_directory}"
for tool in kind kubectl helm kustomize; do
  install -m 0755 "${temporary_directory}/${tool}" "${install_directory}/${tool}"
done

printf 'Installed kind %s, kubectl %s, Helm %s, and Kustomize %s.\n' \
  "${kind_release#v}" "${kubectl_version}" "${helm_version}" "${kustomize_version}"
