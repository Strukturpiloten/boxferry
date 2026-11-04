# renovate: datasource=docker depName=docker.io/library/node
# release: 24.21.0-bookworm-slim
FROM docker.io/library/node:24.21.0-bookworm-slim@sha256:2fe369e969550cde8e867afc3fe370b260140cab4a23d467074295b42163d553 AS node_toolchain

# renovate: datasource=docker depName=ghcr.io/astral-sh/uv
# release: 0.12.22
FROM ghcr.io/astral-sh/uv:0.12.22@sha256:f513a91fc62fe7c17567eee97230dd198e43edb8a9fbecca843714a4358fe1bc AS uv_toolchain

# renovate: datasource=docker depName=mcr.microsoft.com/devcontainers/rust
# release: 2.0.14-bookworm
FROM mcr.microsoft.com/devcontainers/rust:2.0.14-bookworm@sha256:d599b1bc98c3fc53825462ebf1610f39ef9145aeab485deffb8aaef44f349fe1

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

COPY --from=node_toolchain /usr/local/ /usr/local/
COPY --from=uv_toolchain /uv /uvx /usr/local/bin/

# renovate: datasource=crate depName=cargo-deny
ARG CARGO_DENY_VERSION=0.20.2
# renovate: datasource=crate depName=cargo-llvm-cov
ARG CARGO_LLVM_COV_VERSION=0.9.0
# renovate: datasource=crate depName=cargo-semver-checks
ARG CARGO_SEMVER_CHECKS_VERSION=0.50.0
# renovate: datasource=crate depName=zizmor
ARG ZIZMOR_VERSION=1.28.0

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    rustup component add llvm-tools-preview \
    && cargo install --locked --version "${CARGO_DENY_VERSION}" cargo-deny \
    && cargo install --locked --version "${CARGO_LLVM_COV_VERSION}" cargo-llvm-cov \
    && cargo install --locked --version "${CARGO_SEMVER_CHECKS_VERSION}" cargo-semver-checks \
    && cargo install --locked --version "${ZIZMOR_VERSION}" zizmor

COPY scripts/install-file-tools.sh /tmp/boxferry-tooling/scripts/
COPY automation/src/boxferry_ci/tooling.py /tmp/boxferry-tooling/automation/src/boxferry_ci/
COPY automation/tool-artifacts.json /tmp/boxferry-tooling/automation/
COPY package.json package-lock.json /opt/boxferry-file-tools/

RUN set -eux; \
    npm ci --ignore-scripts --prefix /opt/boxferry-file-tools; \
    ln -s /opt/boxferry-file-tools/node_modules/.bin/markdownlint-cli2 \
      /usr/local/bin/markdownlint-cli2; \
    ln -s /opt/boxferry-file-tools/node_modules/.bin/prettier /usr/local/bin/prettier; \
    python3 --version; \
    bash /tmp/boxferry-tooling/scripts/install-file-tools.sh /usr/local/bin; \
    python3 /tmp/boxferry-tooling/automation/src/boxferry_ci/tooling.py \
      --destination /usr/local/bin --tools actionlint lychee; \
    rm -r /tmp/boxferry-tooling

# Git is provided by the digest-pinned Rust base; gh is a locked Dev Container feature.
COPY scripts/install-kubernetes-tools.sh /tmp/install-kubernetes-tools.sh
RUN git --version \
    && bash /tmp/install-kubernetes-tools.sh /usr/local/bin \
    && kind version \
    && kubectl version --client=true \
    && helm version --short \
    && kustomize version \
    && rm /tmp/install-kubernetes-tools.sh

# Install the workspace toolchain as well as the image default. Components belong
# to individual toolchains; a Renovate update must not lose coverage tooling.
COPY rust-toolchain.toml /opt/boxferry-toolchain/rust-toolchain.toml
WORKDIR /opt/boxferry-toolchain
RUN rustup show active-toolchain
WORKDIR /

LABEL org.opencontainers.image.authors="Martin Beckert <https://github.com/TheRealBecks>" \
      org.opencontainers.image.vendor="Strukturpiloten OHG" \
      org.opencontainers.image.url="https://www.strukturpiloten.de/"
