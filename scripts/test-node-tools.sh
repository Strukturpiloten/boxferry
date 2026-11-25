#!/usr/bin/env bash

# Actual npm tools, isolated consumer configuration and no runtime/compiler work.
set -Eeuo pipefail
repository_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
tool_root="${BOXFERRY_NODE_TOOLS_ROOT:-${repository_root}}"
runner=(bash "${repository_root}/scripts/run-node-tool.sh" "${tool_root}")
temporary="$(mktemp -d)"
trap 'rm -rf -- "${temporary}"' EXIT
mkdir -p "${temporary}/parent/consumer" "${temporary}/bin"
# Both parent configuration and PATH would break these checks if selected.
printf '{"tabWidth": 8}\n' > "${temporary}/parent/.prettierrc.json"
printf '[*]\nindent_style = tab\n' > "${temporary}/parent/.editorconfig"
printf '{"config":{"MD013":{"line_length":1}}}\n' > "${temporary}/parent/.markdownlint-cli2.jsonc"
for tool in prettier markdownlint-cli2; do
  printf '#!/usr/bin/env bash\nexit 97\n' > "${temporary}/bin/${tool}"
  chmod +x "${temporary}/bin/${tool}"
done
PATH="${temporary}/bin:${PATH}"
export PATH
cd -- "${temporary}/parent/consumer"
printf '{"tabWidth": 4}\n' > prettier.json
printf 'ignored.json\n' > prettier.ignore
printf '{"config":{"default":true,"MD013":false},"ignores":["ignored.md"]}\n' > markdown.json
printf '{\n"answer":42\n}\n' > input.json
printf '{broken\n' > ignored.json
printf '# Representative consumer\n\nA paragraph longer than one character.\n' > input.md
printf '#bad\n' > ignored.md

expect_status() {
  local expected=$1 status=0
  shift
  "$@" > "${temporary}/output" 2>&1 || status=$?
  if [[ "${status}" != "${expected}" ]]; then
    cat "${temporary}/output" >&2
    printf 'Expected status %s, got %s\n' "${expected}" "${status}" >&2
    exit 1
  fi
}

expect_status 1 "${runner[@]}" prettier --check --config prettier.json \
  --no-editorconfig --ignore-path prettier.ignore input.json ignored.json
"${runner[@]}" prettier --write --config prettier.json \
  --no-editorconfig --ignore-path prettier.ignore input.json ignored.json
grep -Fx '    "answer": 42' input.json
"${runner[@]}" prettier --check --config prettier.json \
  --no-editorconfig --ignore-path prettier.ignore input.json ignored.json
"${runner[@]}" markdownlint-cli2 --config markdown.json ':input.md' ':ignored.md'
printf '#bad\n' > input.md
expect_status 1 "${runner[@]}" markdownlint-cli2 --config markdown.json ':input.md'
printf '# Representative consumer\n\ntext   \n' > input.md
"${runner[@]}" markdownlint-cli2 --config markdown.json --fix ':input.md'
"${runner[@]}" markdownlint-cli2 --config markdown.json ':input.md'
expect_status 2 "${runner[@]}" unsupported
expect_status 2 "${runner[@]}" prettier --check --config prettier.json --no-editorconfig ignored.json
expect_status 2 bash "${repository_root}/scripts/run-node-tool.sh" "$PWD" prettier --version
mkdir "${temporary}/skewed-installation"
cp "${tool_root}/package.json" "${temporary}/skewed-installation/"
printf '{}\n' > "${temporary}/skewed-installation/package-lock.json"
ln -s "${tool_root}/node_modules" "${temporary}/skewed-installation/node_modules"
expect_status 2 bash "${repository_root}/scripts/run-node-tool.sh" \
  "${temporary}/skewed-installation" prettier --version
grep -F 'differs from the selected producer package-lock.json' "${temporary}/output"
printf '{"answer":42}\n' | "${runner[@]}" prettier --parser json --config prettier.json \
  --no-editorconfig > "${temporary}/stdin-output"
grep -Fx '{ "answer": 42 }' "${temporary}/stdin-output"
# Manifest and lock are the only version authority, including image installations.
expected="$(node -p 'require(process.argv[1]).devDependencies.prettier' "${tool_root}/package.json")"
[[ "$("${runner[@]}" prettier --version)" == "${expected}" ]]
expect_status 2 "${runner[@]}" markdownlint-cli2 --help
expected="$(node -p 'require(process.argv[1]).devDependencies["markdownlint-cli2"]' "${tool_root}/package.json")"
grep -F "markdownlint-cli2 v${expected} " "${temporary}/output"

# Model stale installed identities over current manifests without acquiring tools.
# Keep real executable bytes; change only isolated copies of package metadata.
stale_root="${temporary}/stale-installation"
mkdir "${stale_root}"
cp "${tool_root}/package.json" "${tool_root}/package-lock.json" "${stale_root}/"
cp -R "${tool_root}/node_modules" "${stale_root}/"
stale_runner=(bash "${repository_root}/scripts/run-node-tool.sh" "${stale_root}")
printf '{\n"answer":42\n}\n' > stale.json
printf '# Representative consumer\n\ntext   \n' > stale.md
cp stale.json "${temporary}/before.json"
cp stale.md "${temporary}/before.md"
for package in prettier markdownlint-cli2 markdownlint nested-markdownlint; do
  installed="${stale_root}/node_modules/${package}"
  command=markdownlint-cli2
  if [[ "${package}" == prettier ]]; then command=prettier; fi
  if [[ "${package}" == nested-markdownlint ]]; then
    installed="${stale_root}/node_modules/markdownlint-cli2/node_modules/markdownlint"
    mkdir -p "${installed%/*}"
    cp -R "${stale_root}/node_modules/markdownlint" "${installed}"
  else
    cp "${installed}/package.json" "${temporary}/current-package.json"
  fi
  node -e '
const fs = require("node:fs");
const filename = process.argv[1];
const metadata = JSON.parse(fs.readFileSync(filename));
const version = metadata.version.split(".").map(Number);
const part = version.findLastIndex((value) => value > 0);
if (part < 0) throw new Error("Fixture needs an earlier numeric package version");
version[part] -= 1;
version.fill(0, part + 1);
metadata.version = version.join(".");
fs.writeFileSync(filename, JSON.stringify(metadata));
' "${installed}/package.json"
  if [[ "${command}" == prettier ]]; then
    expect_status 2 "${stale_runner[@]}" prettier --check --config prettier.json --no-editorconfig stale.json
    expect_status 2 "${stale_runner[@]}" prettier --write --config prettier.json --no-editorconfig stale.json
  else
    expect_status 2 "${stale_runner[@]}" markdownlint-cli2 --config markdown.json ':stale.md'
    expect_status 2 "${stale_runner[@]}" markdownlint-cli2 --fix --config markdown.json ':stale.md'
  fi
  grep -F "Installed ${command} identity differs from the selected lock" "${temporary}/output"
  cmp stale.json "${temporary}/before.json"
  cmp stale.md "${temporary}/before.md"
  if [[ "${package}" == nested-markdownlint ]]; then
    rm -rf "${installed}"
  else
    mv "${temporary}/current-package.json" "${installed}/package.json"
  fi
done
# Restoring the locked metadata allows the same genuine executables to modify files.
"${stale_runner[@]}" prettier --write --config prettier.json --no-editorconfig stale.json
"${stale_runner[@]}" markdownlint-cli2 --fix --config markdown.json ':stale.md'
"${stale_runner[@]}" prettier --check --config prettier.json --no-editorconfig stale.json
"${stale_runner[@]}" markdownlint-cli2 --config markdown.json ':stale.md'
if cmp -s stale.json "${temporary}/before.json" || cmp -s stale.md "${temporary}/before.md"; then
  printf 'Restored real tools did not apply the expected fixes.\n' >&2
  exit 1
fi
printf 'Locked Node tool execution contracts passed.\n'
