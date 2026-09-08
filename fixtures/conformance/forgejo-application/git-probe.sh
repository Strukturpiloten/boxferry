#!/bin/sh

set -eu

mode=${1:?seed or verify mode required}
readonly mode
: "${BF_FORGEJO_USER:?Forgejo user required}"
: "${BF_FORGEJO_PASSWORD:?Forgejo password required}"
: "${BF_HTTP_PORT:?HTTP port required}"
: "${BF_SSH_PORT:?SSH port required}"

state=/fixture/probe-state
proof=/fixture/repository-proof.txt
http_root="http://127.0.0.1:${BF_HTTP_PORT}"
http_repository="${http_root}/${BF_FORGEJO_USER}/migration-baseline.git"
ssh_repository="ssh://git@127.0.0.1:${BF_SSH_PORT}/${BF_FORGEJO_USER}/migration-baseline.git"
askpass="${state}/askpass.sh"
private_key="${state}/id_ed25519"
ssh_command="ssh -F /dev/null -i ${private_key} -o IdentitiesOnly=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null"
work="$(mktemp -d /tmp/boxferry-forgejo-git.XXXXXX)"
trap 'rm -rf -- "${work}"' EXIT INT TERM

mkdir -p -- "${state}"
chmod 0700 "${state}"
# The generated helper expands these variables when Git invokes it.
# shellcheck disable=SC2016
printf '%s\n' '#!/bin/sh' \
  'case "$1" in' \
  '  *Username*) printf "%s\\n" "${BF_FORGEJO_USER}" ;;' \
  '  *) printf "%s\\n" "${BF_FORGEJO_PASSWORD}" ;;' \
  'esac' > "${askpass}"
chmod 0700 "${askpass}"

http_git() {
  GIT_ASKPASS="${askpass}" GIT_TERMINAL_PROMPT=0 git "$@"
}

ssh_git() {
  GIT_SSH_COMMAND="${ssh_command}" git "$@"
}

api_post() {
  path=$1
  payload=$2
  authorization="$(printf '%s' "${BF_FORGEJO_USER}:${BF_FORGEJO_PASSWORD}" | base64 | tr -d '\n')"
  wget --quiet --output-document=/dev/null \
    --header="Authorization: Basic ${authorization}" \
    --header='Content-Type: application/json' \
    --post-data="${payload}" "${http_root}${path}"
}

assert_checkout() {
  directory=$1
  expected_head="$(cat "${state}/expected-head")"
  test "$(git -C "${directory}" rev-parse HEAD)" = "${expected_head}"
  cmp "${proof}" "${directory}/repository-proof.txt"
  test "$(cat "${directory}/protocol-proof.txt")" = 'pushed-over-ssh'
}

clone_ssh_with_retry() {
  destination=$1
  attempts=0
  while ! ssh_git clone --quiet "${ssh_repository}" "${destination}"; do
    attempts=$((attempts + 1))
    if test "${attempts}" -ge 30; then
      printf '%s\n' 'SSH clone did not become ready after 60 seconds.' >&2
      return 1
    fi
    rm -rf -- "${destination}"
    sleep 2
  done
}

case "${mode}" in
  seed)
    ssh-keygen -q -t ed25519 -N '' -C boxferry-live -f "${private_key}"
    api_post /api/v1/user/repos '{"name":"migration-baseline","private":true}'
    public_key="$(cat "${private_key}.pub")"
    api_post /api/v1/user/keys "{\"title\":\"boxferry-live\",\"key\":\"${public_key}\"}"

    git init --quiet --initial-branch=main "${work}/seed"
    git -C "${work}/seed" config user.name 'BoxFerry Live'
    git -C "${work}/seed" config user.email 'boxferry@example.invalid'
    cp "${proof}" "${work}/seed/repository-proof.txt"
    git -C "${work}/seed" add repository-proof.txt
    GIT_AUTHOR_DATE='2001-01-01T00:00:00Z' GIT_COMMITTER_DATE='2001-01-01T00:00:00Z' \
      git -C "${work}/seed" commit --quiet --message 'Add deterministic migration proof'
    http_git -C "${work}/seed" push --quiet "${http_repository}" main

    clone_ssh_with_retry "${work}/ssh-clone"
    git -C "${work}/ssh-clone" config user.name 'BoxFerry Live'
    git -C "${work}/ssh-clone" config user.email 'boxferry@example.invalid'
    printf '%s\n' 'pushed-over-ssh' > "${work}/ssh-clone/protocol-proof.txt"
    git -C "${work}/ssh-clone" add protocol-proof.txt
    GIT_AUTHOR_DATE='2001-01-01T00:01:00Z' GIT_COMMITTER_DATE='2001-01-01T00:01:00Z' \
      git -C "${work}/ssh-clone" commit --quiet --message 'Prove SSH publication'
    ssh_git -C "${work}/ssh-clone" push --quiet origin main
    git -C "${work}/ssh-clone" rev-parse HEAD > "${state}/expected-head"

    http_git clone --quiet "${http_repository}" "${work}/http-clone"
    assert_checkout "${work}/http-clone"
    ;;
  verify)
    test -s "${private_key}"
    test -s "${state}/expected-head"
    http_git clone --quiet "${http_repository}" "${work}/http-clone"
    assert_checkout "${work}/http-clone"
    clone_ssh_with_retry "${work}/ssh-clone"
    assert_checkout "${work}/ssh-clone"
    ;;
  *)
    printf 'Unknown Forgejo Git probe mode: %s\n' "${mode}" >&2
    exit 2
    ;;
esac
