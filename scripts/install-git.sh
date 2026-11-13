#!/usr/bin/env bash
# Image provisioning wrapper for the admitted installer, with the candidate inventory.
set -euo pipefail
repository_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
if [[ "$#" -ne 1 ]]; then
  printf 'Usage: bash scripts/install-git.sh NEW_ABSOLUTE_PREFIX\n' >&2
  exit 2
fi
# Reverify immutable objects, including warm caches; no development override.
python3 "${repository_root}/scripts/shared-ci.py" --help > /dev/null
revision="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["revision"])' \
  "${repository_root}/.github/automation.json")"
project="${repository_root}/target/boxferry-automation/${revision}/automation"
uv run --locked --project "${project}" python -I - "$1" "${repository_root}/automation/tool-artifacts.json" << 'PYTHON'
import json
from pathlib import Path
import sys
import subprocess
from boxferry_ci import runner, tooling
from boxferry_ci.git_install import install, load_source

# Keep the admitted runner, commands, limits and refusal unchanged; retain its
# bounded command output so native prerequisite failures remain actionable.
def execute(*args, **kwargs):
    result = runner.run(*args, **kwargs)
    print(result.stdout, end="", flush=True)
    print(result.stderr, end="", file=sys.stderr, flush=True)
    return result

try:
    prefix = Path(sys.argv[1])
    receipt = install(prefix, load_source(Path(sys.argv[2])), execute=execute)
    options = subprocess.check_output([str(prefix / "bin/git"), "--version", "--build-options"],
                                      text=True, timeout=30)
    if "rust: disabled" not in options.splitlines():
        raise tooling.ToolError("Git C build profile differs from its recorded capability omissions")
    print(options, end="", flush=True)
except tooling.ToolError as error:
    print(f"Canonical Git image installation failed: {error}", file=sys.stderr)
    raise SystemExit(1) from None
print(json.dumps(receipt, sort_keys=True))
PYTHON
