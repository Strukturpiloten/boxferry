#!/usr/bin/env bash

set -euo pipefail
tooling_root="/opt/boxferry-tooling"

for persistent_directory_name in CARGO_HOME GH_CONFIG_DIR; do
  persistent_directory="${!persistent_directory_name:-}"
  if [[ -z "${persistent_directory}" ]]; then
    printf 'BoxFerry Dev Container is missing %s.\n' "${persistent_directory_name}" >&2
    exit 1
  fi
  if [[ ! -w "${persistent_directory}" ]]; then
    sudo chown -R "$(id -u):$(id -g)" "${persistent_directory}"
  fi
  if [[ ! -w "${persistent_directory}" ]]; then
    printf 'BoxFerry Dev Container cannot make %s writable: %s\n' \
      "${persistent_directory_name}" "${persistent_directory}" >&2
    exit 1
  fi
done

chmod 0700 "${GH_CONFIG_DIR}"

tools=(
  actionlint
  cargo
  cargo-clippy
  cargo-deny
  cargo-llvm-cov
  cargo-semver-checks
  curl
  gh
  git
  hadolint
  helm
  jq
  kind
  kubectl
  kustomize
  lychee
  markdownlint-cli2
  node
  npm
  prettier
  python3
  ruff
  rustc
  rustfmt
  rustup
  shellcheck
  shfmt
  tombi
  ty
  uv
  zizmor
)

for tool in "${tools[@]}"; do
  if ! command -v "${tool}" > /dev/null 2>&1; then
    printf 'BoxFerry Dev Container is missing required tool: %s\n' "${tool}" >&2
    exit 1
  fi
done

# Execute client-only commands as the development user; no runtime or credentials needed.
git --version > /dev/null
gh --version > /dev/null
kind version > /dev/null
kubectl version --client=true > /dev/null
helm version --short > /dev/null
kustomize version > /dev/null

installed_components="$(rustup component list --installed)"
if ! grep -q '^llvm-tools-' <<< "${installed_components}"; then
  printf 'BoxFerry Dev Container is missing Rust component: llvm-tools-preview\nActive toolchain: %s\n' \
    "$(rustup show active-toolchain)" >&2
  printf 'From the BoxFerry repository root, run: rustup component add llvm-tools-preview\nThen rerun: bash .devcontainer/verify-tools.sh\n' >&2
  exit 1
fi

repository_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
# Inspect the exact mounted checkout and index; missing metadata is never repaired.

# The image retains the selected authority for executable version checks; no bootstrap here.
python3 - "${repository_root}" "${tooling_root}" << 'PYTHON'
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys
import tomllib

try:
    repository, tooling = map(Path, sys.argv[1:])
    manifest = json.loads((repository / ".github/automation.json").read_text())
    if json.loads((tooling / ".github/automation.json").read_text()) != manifest:
        raise ValueError("retained shared-tool authority differs from the checkout; rebuild the image")
    inventory = json.loads((repository / "automation/tool-artifacts.json").read_text())
    sources = inventory.get("sources", [])
    if len(sources) != 1 or sources[0]["tool"] != "git":
        raise ValueError("one canonical Git source is required")
    source = sources[0]
    prefix = Path("/opt/boxferry-git")
    provenance = json.loads((prefix / "share/doc/git/boxferry-source.json").read_text())
    if provenance["source"] != {"version": source["currentVersion"], "url": source["url"],
                                "sha256": source["sha256"]}:
        raise ValueError("installed Git source differs from the checkout; rebuild the image")
    if (hashlib.sha256((prefix / "share/doc/git/COPYING").read_bytes()).hexdigest()
            != provenance["license"]["sha256"]):
        raise ValueError("Git COPYING differs from its installed source provenance")
    options = subprocess.run(["git", "--version", "--build-options"], check=True,
                             capture_output=True, text=True, timeout=30).stdout
    if "rust: disabled" not in options.splitlines():
        raise ValueError("Git C build profile differs from recorded capability omissions")
    for executable in ("git", "/usr/bin/git"):
        actual = subprocess.run([executable, "--version"], check=True, capture_output=True,
                                text=True, timeout=30).stdout.strip()
        if actual != "git version " + source["currentVersion"]:
            raise ValueError(f"{executable} has {actual}; expected Git {source['currentVersion']}; rebuild the image")
        helper = subprocess.run([executable, "--exec-path"], check=True, capture_output=True,
                                text=True, timeout=30).stdout.strip()
        if Path(helper) != prefix / "libexec/git-core":
            raise ValueError("Git helper prefix differs from the complete runtime")
    for name in ("libexec/git-core/git-remote-http", "libexec/git-core/git-remote-https",
                 "libexec/git-core/git-http-fetch", "libexec/git-core/git-http-push",
                 "libexec/git-core/git-send-email", "share/perl5/Git.pm",
                 "share/git-core/templates/info/exclude", "share/doc/git/COPYING"):
        if not (prefix / name).is_file():
            raise ValueError("Git runtime is incomplete: " + name)
    subprocess.run(["perl", "-I" + str(prefix / "share/perl5"), "-MGit", "-e", "exit 0"],
                   check=True, timeout=30)
    project = tooling / "target/boxferry-automation" / manifest["revision"] / "automation"
    metadata = tomllib.loads((project / "pyproject.toml").read_text())
    requirement = metadata["tool"]["uv"]["required-version"]
    if not requirement.startswith("=="):
        raise ValueError("selected uv requirement must be exact")
    versions = {"uv": requirement[2:]}
    for name in ("ruff", "ty"):
        declared = [item.split("==", 1)[1] for item in metadata["dependency-groups"]["dev"]
                    if item.startswith(name + "==")]
        if len(declared) != 1:
            raise ValueError(f"selected project must declare one exact {name} version")
        versions[name] = declared[0]
    image = (repository / ".devcontainer/Dockerfile").read_text()
    node = re.search(r"^FROM docker\.io/library/node:([0-9]+\.[0-9]+\.[0-9]+)-[^\s]+@sha256:[a-f0-9]{64}",
                     image, re.MULTILINE)
    if node is None:
        raise ValueError("Node image must retain its version and digest declaration")
    versions["node"] = node[1]
    for name, expected in versions.items():
        if re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", expected) is None:
            raise ValueError(f"selected {name} version must be exact")
        actual = subprocess.run([name, "--version"], check=True, capture_output=True,
                                text=True, timeout=30).stdout.strip()
        pattern = ("v" + re.escape(expected) if name == "node" else
                   re.escape(name + " " + expected) + r"(?: \([^\r\n]+\))?")
        if re.fullmatch(pattern, actual) is None:
            raise ValueError(f"{name} executable does not match selected version {expected}")
        subprocess.run([name, "--help"], check=True, stdout=subprocess.DEVNULL,
                       stderr=subprocess.PIPE, timeout=30)
    channel = tomllib.loads((repository / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    compiler = subprocess.run(["rustc", "--version"], check=True, capture_output=True,
                              text=True, timeout=30).stdout.strip()
    if not compiler.startswith("rustc " + channel + " "):
        raise ValueError("installed Rust compiler does not match rust-toolchain.toml")
except (KeyError, OSError, ValueError, subprocess.SubprocessError) as error:
    print(f"BoxFerry Dev Container tool verification failed: {error}", file=sys.stderr)
    raise SystemExit(1) from None
PYTHON

python3 "${repository_root}/scripts/git-worktree.py" inspect --source "${repository_root}"

printf 'BoxFerry Dev Container tooling is ready.\n'
