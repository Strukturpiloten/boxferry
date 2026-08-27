# renovate: datasource=docker depName=docker.io/library/node
# release: 24.18.1-bookworm-slim
FROM docker.io/library/node@sha256:235600a8101ab264e117b1768e925532262668dc9b581ef1dd7d96ced463b8e7 AS node_toolchain

# renovate: datasource=docker depName=ghcr.io/astral-sh/uv
# release: 0.12.5
FROM ghcr.io/astral-sh/uv@sha256:e85be844203885286c60ffad8a858d48afb6c5a5c237ca0e67f12e74b8f174b1 AS uv_toolchain

# renovate: datasource=docker depName=mcr.microsoft.com/devcontainers/rust
# release: 2.0.14-bookworm
FROM mcr.microsoft.com/devcontainers/rust@sha256:d599b1bc98c3fc53825462ebf1610f39ef9145aeab485deffb8aaef44f349fe1

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

COPY --from=node_toolchain /usr/local/ /usr/local/
COPY --from=uv_toolchain /uv /uvx /usr/local/bin/

# renovate: datasource=crate depName=cargo-deny
ARG CARGO_DENY_VERSION=0.20.2
# renovate: datasource=crate depName=cargo-llvm-cov
ARG CARGO_LLVM_COV_VERSION=0.9.0
# renovate: datasource=crate depName=cargo-semver-checks
ARG CARGO_SEMVER_CHECKS_VERSION=0.50.0
# renovate: datasource=crate depName=lychee
ARG LYCHEE_VERSION=0.24.2
# renovate: datasource=crate depName=zizmor
ARG ZIZMOR_VERSION=1.28.0
# renovate: datasource=github-releases depName=rhysd/actionlint
ARG ACTIONLINT_VERSION=1.7.12

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    rustup component add llvm-tools-preview \
    && cargo install --locked --version "${CARGO_DENY_VERSION}" cargo-deny \
    && cargo install --locked --version "${CARGO_LLVM_COV_VERSION}" cargo-llvm-cov \
    && cargo install --locked --version "${CARGO_SEMVER_CHECKS_VERSION}" cargo-semver-checks \
    && cargo install --locked --version "${LYCHEE_VERSION}" lychee \
    && cargo install --locked --version "${ZIZMOR_VERSION}" zizmor

COPY scripts/install-file-tools.sh /tmp/install-file-tools.sh
COPY package.json package-lock.json /opt/boxferry-file-tools/

RUN set -eux; \
    npm ci --ignore-scripts --prefix /opt/boxferry-file-tools; \
    ln -s /opt/boxferry-file-tools/node_modules/.bin/markdownlint-cli2 \
      /usr/local/bin/markdownlint-cli2; \
    ln -s /opt/boxferry-file-tools/node_modules/.bin/prettier /usr/local/bin/prettier; \
    bash /tmp/install-file-tools.sh /usr/local/bin; \
    rm /tmp/install-file-tools.sh; \
    architecture="$(dpkg --print-architecture)"; \
    case "${architecture}" in amd64 | arm64) ;; *) exit 1 ;; esac; \
    archive="actionlint_${ACTIONLINT_VERSION}_linux_${architecture}.tar.gz"; \
    release_url="https://github.com/rhysd/actionlint/releases/download/v${ACTIONLINT_VERSION}"; \
    temporary_directory="$(mktemp -d)"; \
    curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error \
      --output "${temporary_directory}/checksums.txt" \
      "${release_url}/actionlint_${ACTIONLINT_VERSION}_checksums.txt"; \
    curl --proto '=https' --tlsv1.2 --fail --location --silent --show-error \
      --output "${temporary_directory}/${archive}" "${release_url}/${archive}"; \
    checksum="$(grep -F "  ${archive}" "${temporary_directory}/checksums.txt" \
      | cut --delimiter=' ' --fields=1)"; \
    test -n "${checksum}"; \
    printf '%s  %s\n' "${checksum}" "${temporary_directory}/${archive}" \
      | sha256sum --check -; \
    tar --extract --gzip --file "${temporary_directory}/${archive}" \
      --directory "${temporary_directory}" actionlint; \
    install -m 0755 "${temporary_directory}/actionlint" /usr/local/bin/actionlint; \
    rm -r "${temporary_directory}"

LABEL org.opencontainers.image.authors="Martin Beckert <https://github.com/TheRealBecks>" \
      org.opencontainers.image.vendor="Strukturpiloten OHG" \
      org.opencontainers.image.url="https://www.strukturpiloten.de/"
