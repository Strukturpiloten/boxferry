#!/usr/bin/env bash

set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repository_root="$(cd -- "${script_directory}/.." && pwd -P)"
readonly repository_root

cd -- "${repository_root}"

mode="${1:---check}"
if [[ "${mode}" != "--check" && "${mode}" != "--fix" ]]; then
  printf 'Usage: %s [--check|--fix]\n' "$0" >&2
  exit 2
fi
readonly mode

required_tools=(git hadolint markdownlint-cli2 prettier shellcheck shfmt tombi)
missing_tools=()
for tool in "${required_tools[@]}"; do
  if ! command -v "${tool}" > /dev/null 2>&1; then
    missing_tools+=("${tool}")
  fi
done
if ((${#missing_tools[@]} != 0)); then
  printf -v missing_list ' %s' "${missing_tools[@]}"
  printf 'BoxFerry file checks are missing required tool(s):%s. Use the Dev Container.\n' \
    "${missing_list}" >&2
  exit 2
fi

list_existing_files() {
  while IFS= read -r -d '' file; do
    if [[ -f "${file}" ]]; then
      printf '%s\0' "${file}"
    fi
  done < <(git ls-files --cached --others --exclude-standard -z -- "$@")
}

mapfile -d '' markdown_files < <(
  list_existing_files '*.md'
)
mapfile -d '' structured_files < <(
  list_existing_files \
    '*.json' '*.jsonc' '*.yaml' '*.yml' '*.code-workspace'
)
mapfile -d '' toml_files < <(
  list_existing_files '*.toml'
)
mapfile -d '' shell_files < <(
  list_existing_files '*.sh'
)
mapfile -d '' dockerfiles < <(
  list_existing_files \
    ':(glob)Dockerfile' ':(glob)**/Dockerfile' ':(glob)**/Dockerfile.*'
)

if ((${#markdown_files[@]} == 0)); then
  printf 'BoxFerry contains no Markdown files to check.\n' >&2
  exit 2
fi

markdown_literals=()
for file in "${markdown_files[@]}"; do
  markdown_literals+=(":${file}")
done

run() {
  printf '  +'
  printf ' %q' "$@"
  printf '\n'
  "$@"
}

if [[ "${mode}" == "--fix" ]]; then
  printf '\nFormat Markdown\n'
  run prettier --write --ignore-unknown "${markdown_files[@]}"
  run markdownlint-cli2 --fix "${markdown_literals[@]}"

  if ((${#structured_files[@]} != 0)); then
    printf '\nFormat JSON and YAML\n'
    run prettier --write --ignore-unknown "${structured_files[@]}"
  fi

  if ((${#toml_files[@]} != 0)); then
    printf '\nFormat TOML\n'
    run tombi format --offline "${toml_files[@]}"
  fi

  if ((${#shell_files[@]} != 0)); then
    printf '\nFormat shell scripts\n'
    run shfmt -w -i 2 -ci -sr "${shell_files[@]}"
  fi
fi

printf '\nLint Markdown\n'
run markdownlint-cli2 "${markdown_literals[@]}"

printf '\nCheck Markdown formatting\n'
run prettier --check --ignore-unknown "${markdown_files[@]}"

if ((${#structured_files[@]} != 0)); then
  printf '\nCheck JSON and YAML formatting and syntax\n'
  run prettier --check --ignore-unknown "${structured_files[@]}"
fi

if ((${#toml_files[@]} != 0)); then
  printf '\nCheck TOML formatting and validity\n'
  run tombi format --check --offline "${toml_files[@]}"
  run tombi lint --error-on-warnings --offline "${toml_files[@]}"
fi

if ((${#shell_files[@]} != 0)); then
  printf '\nCheck shell formatting and lint\n'
  run shfmt -d -i 2 -ci -sr "${shell_files[@]}"
  run shellcheck -- "${shell_files[@]}"
fi

if ((${#dockerfiles[@]} != 0)); then
  printf '\nLint Dockerfiles\n'
  run hadolint "${dockerfiles[@]}"
fi

printf '\nBoxFerry non-Rust file checks passed in %s mode.\n' "${mode#--}"
