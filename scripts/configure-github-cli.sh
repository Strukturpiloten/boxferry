#!/usr/bin/env bash

set -euo pipefail

readonly github_host="github.com"
readonly expected_config_directory="/workspaces/.boxferry-gh"

if [[ "${GH_CONFIG_DIR:-}" != "${expected_config_directory}" ]]; then
  printf 'Run this command inside the BoxFerry Dev Container after rebuilding it.\n' >&2
  printf 'Expected GH_CONFIG_DIR=%s, found %s.\n' \
    "${expected_config_directory}" "${GH_CONFIG_DIR:-<unset>}" >&2
  exit 1
fi

if ! command -v gh > /dev/null 2>&1; then
  printf 'GitHub CLI is not installed in this Dev Container. Rebuild the container.\n' >&2
  exit 1
fi

if [[ ! -t 0 ]]; then
  printf 'Run this command in an interactive terminal so the token can be entered securely.\n' >&2
  exit 1
fi

if [[ -n "${GH_TOKEN:-}" || -n "${GITHUB_TOKEN:-}" ]]; then
  printf 'Unset GH_TOKEN and GITHUB_TOKEN before configuring persistent authentication.\n' >&2
  exit 1
fi

mkdir -p "${GH_CONFIG_DIR}"
chmod 0700 "${GH_CONFIG_DIR}"
umask 077

if gh auth status --hostname "${github_host}" > /dev/null 2>&1; then
  current_user="$(gh api user --jq '.login')"
  printf 'GitHub CLI is already authenticated as %s in this Dev Container.\n' "${current_user}"
  read -r -p 'Replace the stored authentication? [y/N] ' replace_authentication
  case "${replace_authentication}" in
    y | Y | yes | YES)
      gh auth logout --hostname "${github_host}" --user "${current_user}"
      ;;
    *)
      printf 'Existing GitHub CLI authentication was left unchanged.\n'
      exit 0
      ;;
  esac
fi

printf '%s\n' 'Required fine-grained token permissions for the three Strukturpiloten repositories:'
printf '%s\n' '  Contents: Read and write'
printf '%s\n' '  Issues: Read and write'
printf '%s\n' '  Pull requests: Read and write'
printf '%s\n' '  Workflows: Read and write'
printf '%s\n' 'Paste the token below. Input is hidden and is never passed as a command-line argument.'

github_token=""
trap 'unset github_token' EXIT INT TERM
IFS= read -r -s -p 'GitHub token: ' github_token
printf '\n'

if [[ -z "${github_token}" ]]; then
  printf 'No token was entered.\n' >&2
  exit 1
fi

printf '%s\n' "${github_token}" |
  gh auth login --hostname "${github_host}" --with-token --insecure-storage
unset github_token

readonly auth_file="${GH_CONFIG_DIR}/hosts.yml"
if [[ ! -f "${auth_file}" ]]; then
  printf 'GitHub CLI did not create the expected authentication file: %s\n' "${auth_file}" >&2
  exit 1
fi
chmod 0600 "${auth_file}"

gh auth status --hostname "${github_host}"
printf 'GitHub CLI authentication is stored only in %s.\n' "${auth_file}"
printf '%s\n' 'The existing Git credential helpers were not changed.'
