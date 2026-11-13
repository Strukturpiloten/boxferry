# renovate: datasource=docker depName=docker.io/library/node
# release: 24.22.1-bookworm-slim
FROM docker.io/library/node:24.22.1-bookworm-slim@sha256:55fb5f5a527ea2c5c4a95f963e48ef90aa38c7b346054fdc8ee9957a2b55d205 AS node_toolchain

# renovate: datasource=docker depName=mcr.microsoft.com/devcontainers/rust
# release: 2.0.14-bookworm
FROM mcr.microsoft.com/devcontainers/rust:2.0.14-bookworm@sha256:d599b1bc98c3fc53825462ebf1610f39ef9145aeab485deffb8aaef44f349fe1

SHELL ["/bin/bash", "-o", "pipefail", "-c"]

COPY --from=node_toolchain /usr/local/ /usr/local/
COPY rust-toolchain.toml /opt/boxferry-toolchain/rust-toolchain.toml

# renovate: datasource=crate depName=cargo-deny
ARG CARGO_DENY_VERSION=0.20.2
# renovate: datasource=crate depName=cargo-llvm-cov
ARG CARGO_LLVM_COV_VERSION=0.9.0
# renovate: datasource=crate depName=cargo-semver-checks
ARG CARGO_SEMVER_CHECKS_VERSION=0.50.0
# renovate: datasource=crate depName=zizmor
ARG ZIZMOR_VERSION=1.28.0

RUN --mount=type=cache,target=/usr/local/cargo/registry \
    set -eux; \
    rust_channel="$(python3 -c 'import pathlib, tomllib; print(tomllib.loads(pathlib.Path("/opt/boxferry-toolchain/rust-toolchain.toml").read_text())["toolchain"]["channel"])')"; \
    rustup toolchain install "${rust_channel}" --profile minimal --component clippy --component rustfmt --component llvm-tools-preview; \
    rustup default "${rust_channel}"; \
    cargo "+${rust_channel}" install --locked --version "${CARGO_DENY_VERSION}" cargo-deny; \
    cargo "+${rust_channel}" install --locked --version "${CARGO_LLVM_COV_VERSION}" cargo-llvm-cov; \
    cargo "+${rust_channel}" install --locked --version "${CARGO_SEMVER_CHECKS_VERSION}" cargo-semver-checks; \
    cargo "+${rust_channel}" install --locked --version "${ZIZMOR_VERSION}" zizmor

COPY scripts/install-file-tools.sh /opt/boxferry-tooling/scripts/
COPY automation/src/boxferry_ci/tooling.py automation/src/boxferry_ci/git_source.py /opt/boxferry-tooling/automation/src/boxferry_ci/
COPY automation/tool-artifacts.json /opt/boxferry-tooling/automation/
COPY scripts/shared-ci.py scripts/install-git.sh /opt/boxferry-tooling/scripts/
COPY .github/automation.json /opt/boxferry-tooling/.github/
COPY package.json package-lock.json /opt/boxferry-file-tools/
ENV UV_CACHE_DIR=/opt/boxferry-tooling/uv-cache \
    UV_PYTHON_INSTALL_DIR=/opt/boxferry-tooling/python

RUN set -eux; \
    npm ci --ignore-scripts --prefix /opt/boxferry-file-tools; \
    ln -s /opt/boxferry-file-tools/node_modules/.bin/markdownlint-cli2 \
      /usr/local/bin/markdownlint-cli2; \
    ln -s /opt/boxferry-file-tools/node_modules/.bin/prettier /usr/local/bin/prettier; \
    python3 --version; \
    bash /opt/boxferry-tooling/scripts/install-file-tools.sh /usr/local/bin; \
    python3 /opt/boxferry-tooling/scripts/shared-ci.py tools \
      --destination /usr/local/bin --tools actionlint lychee --python-tools; \
    bash /opt/boxferry-tooling/scripts/install-git.sh /opt/boxferry-git; \
    for executable in /opt/boxferry-git/bin/*; do \
      ln -sf "${executable}" "/usr/local/bin/${executable##*/}"; \
      ln -sf "${executable}" "/usr/bin/${executable##*/}"; \
    done; \
    chown -R vscode:vscode /opt/boxferry-tooling/target /opt/boxferry-tooling/uv-cache /opt/boxferry-tooling/python; \
    chmod -R a+rX /opt/boxferry-tooling

# Git uses the canonical source inventory and complete prefix; gh is a locked feature.
COPY scripts/install-kubernetes-tools.sh /tmp/install-kubernetes-tools.sh
RUN git --version \
    && bash /tmp/install-kubernetes-tools.sh /usr/local/bin \
    && kind version \
    && kubectl version --client=true \
    && helm version --short \
    && kustomize version \
    && rm /tmp/install-kubernetes-tools.sh

LABEL org.opencontainers.image.authors="Martin Beckert <https://github.com/TheRealBecks>" \
      org.opencontainers.image.vendor="Strukturpiloten OHG" \
      org.opencontainers.image.url="https://www.strukturpiloten.de/"
