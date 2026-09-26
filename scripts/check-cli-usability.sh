#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
cd -- "${script_dir}/.."

# Keep this focused command comfortable on smaller development machines.
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-2}"
exec cargo test --locked --package boxferry --all-features --test cli_usability --test cli_privacy -- --test-threads=1
