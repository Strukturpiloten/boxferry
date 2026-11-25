#!/usr/bin/env bash

# The caller owns cwd, configuration, ignores, file selection and check/fix mode.
set -Eeuo pipefail

if (($# < 2)); then
  printf 'Usage: %s <locked npm installation root> <prettier|markdownlint-cli2> [arguments...]\n' "$0" >&2
  exit 2
fi
tool_root="$(cd -- "$1" && pwd -P)"
tool=$2
shift 2
case "${tool}" in
  prettier) entry='prettier/bin/prettier.cjs' ;;
  markdownlint-cli2) entry='markdownlint-cli2/markdownlint-cli2-bin.mjs' ;;
  *)
    printf 'Unsupported shared Node tool: %s\n' "${tool}" >&2
    exit 2
    ;;
esac
readonly tool_root tool entry
producer_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
readonly producer_root
for manifest in package.json package-lock.json; do
  if ! cmp -s -- "${producer_root}/${manifest}" "${tool_root}/${manifest}"; then
    printf 'Node tool installation differs from the selected producer %s; reinstall its locked project.\n' "${manifest}" >&2
    exit 2
  fi
done
if [[ ! -f "${tool_root}/package.json" || ! -f "${tool_root}/package-lock.json" ||
  ! -f "${tool_root}/node_modules/${entry}" ]]; then
  printf 'Missing locked Node tools in %s; run npm ci --ignore-scripts --prefix there.\n' "${tool_root}" >&2
  exit 2
fi
# Inspect metadata without loading installed package code or consuming stdin.
# Resolve markdownlint from the CLI, including npm's nested dependency layout.
node -e '
const fs = require("node:fs");
const path = require("node:path");
const { createRequire } = require("node:module");
const [producer, root, tool, entry] = process.argv.slice(1);
const read = (filename) => JSON.parse(fs.readFileSync(filename, "utf8"));
try {
  const lock = read(path.join(producer, "package-lock.json"));
  const checkPackage = (directory, name) => {
    const relative = path.relative(root, directory).split(path.sep).join("/");
    const expected = lock.packages[relative];
    const installed = read(path.join(directory, "package.json"));
    if (fs.realpathSync(directory) !== directory || !expected ||
        installed.name !== name || installed.version !== expected.version) {
      throw new Error("package identity mismatch");
    }
    return { installed, expected };
  };
  const directory = path.join(root, "node_modules", tool);
  const { installed, expected } = checkPackage(directory, tool);
  const executable = path.join(root, "node_modules", entry);
  const installedBin = typeof installed.bin === "string" ? installed.bin : installed.bin[tool];
  if (lock.packages[""].devDependencies[tool] !== expected.version ||
      path.resolve(directory, installedBin) !== executable ||
      path.resolve(directory, expected.bin[tool]) !== executable ||
      fs.realpathSync(executable) !== executable) {
    throw new Error("executable identity mismatch");
  }
  if (tool === "markdownlint-cli2") {
    const resolve = createRequire(executable).resolve;
    let dependency = path.dirname(resolve("markdownlint"));
    while (!fs.existsSync(path.join(dependency, "package.json"))) {
      const parent = path.dirname(dependency);
      if (parent === dependency) throw new Error("missing dependency metadata");
      dependency = parent;
    }
    const library = checkPackage(dependency, "markdownlint");
    if (expected.dependencies.markdownlint !== library.expected.version ||
        ["markdownlint/promise", "markdownlint/helpers"].some(
          (name) => !fs.realpathSync(resolve(name)).startsWith(dependency + path.sep))) {
      throw new Error("dependency identity mismatch");
    }
  }
} catch {
  console.error("Installed " + tool + " identity differs from the selected lock; run npm ci --ignore-scripts --prefix " + root + ".");
  process.exit(2);
}
' "${producer_root}" "${tool_root}" "${tool}" "${entry}"
# Absolute package entry points prevent unrelated PATH executables from winning.
exec node "${tool_root}/node_modules/${entry}" "$@"
