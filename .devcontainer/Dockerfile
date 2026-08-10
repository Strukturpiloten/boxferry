# renovate: datasource=docker depName=docker.io/library/node
# release: 24.18.1-bookworm-slim
FROM docker.io/library/node@sha256:235600a8101ab264e117b1768e925532262668dc9b581ef1dd7d96ced463b8e7 AS node_toolchain

# renovate: datasource=docker depName=mcr.microsoft.com/devcontainers/rust
# release: 2.0.14-bookworm
FROM mcr.microsoft.com/devcontainers/rust@sha256:d599b1bc98c3fc53825462ebf1610f39ef9145aeab485deffb8aaef44f349fe1

COPY --from=node_toolchain /usr/local/ /usr/local/

# renovate: datasource=crate depName=cargo-deny
ARG CARGO_DENY_VERSION=0.20.2
# renovate: datasource=crate depName=cargo-semver-checks
ARG CARGO_SEMVER_CHECKS_VERSION=0.50.0
# renovate: datasource=crate depName=lychee
ARG LYCHEE_VERSION=0.24.2
# renovate: datasource=crate depName=zizmor
ARG ZIZMOR_VERSION=1.28.0
# renovate: datasource=npm depName=markdownlint-cli2
ARG MARKDOWNLINT_CLI2_VERSION=0.23.2
# renovate: datasource=github-releases depName=rhysd/actionlint
ARG ACTIONLINT_VERSION=1.7.12

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    cargo install --locked --version "${CARGO_DENY_VERSION}" cargo-deny \
    && cargo install --locked --version "${CARGO_SEMVER_CHECKS_VERSION}" cargo-semver-checks \
    && cargo install --locked --version "${LYCHEE_VERSION}" lychee \
    && cargo install --locked --version "${ZIZMOR_VERSION}" zizmor

RUN set -eux; \
    npm install --global "markdownlint-cli2@${MARKDOWNLINT_CLI2_VERSION}"; \
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
    cd "${temporary_directory}"; \
    grep -E "[[:space:]]${archive}$" checksums.txt | sha256sum --check -; \
    tar --extract --gzip --file "${archive}" actionlint; \
    install -m 0755 actionlint /usr/local/bin/actionlint; \
    cd /; \
    rm -r "${temporary_directory}"


LABEL org.opencontainers.image.authors="Martin Beckert <https://github.com/TheRealBecks>" \
      org.opencontainers.image.vendor="Strukturpiloten OHG" \
      org.opencontainers.image.url="https://www.strukturpiloten.de/"
