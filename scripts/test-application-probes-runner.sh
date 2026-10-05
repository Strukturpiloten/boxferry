#!/usr/bin/env bash
# Prove the shared probe runner invokes every suite and stops on failure.
set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
test_root="$(mktemp -d)"
trap 'rm -r -- "${test_root}"' EXIT
cp -- "${script_directory}/test-application-probes.sh" "${test_root}/"
for application in runner export-privacy forgejo nextcloud paperless immich observability live-cleanup; do
  if [[ "${application}" == runner ]]; then
    stub="${test_root}/test-application-probes-runner.sh"
  elif [[ "${application}" == export-privacy ]]; then
    stub="${test_root}/test-application-export-privacy.sh"
  elif [[ "${application}" == live-cleanup ]]; then
    stub="${test_root}/test-podman-live-cleanup.sh"
  else
    stub="${test_root}/test-${application}-application-probes.sh"
  fi
  # shellcheck disable=SC2016 # These are literal lines in generated test stubs.
  printf '%s\n' '#!/usr/bin/env bash' \
    'name="${0##*/}"' \
    'printf "%s\n" "${name}" >> "${PROBE_TEST_LOG}"' \
    '[[ "${name}" != "${PROBE_FAIL:-}" ]] || exit 37' > "${stub}"
done

mkdir -- "${test_root}/bin"
# shellcheck disable=SC2016 # Literal stub commands prove Python invocation shape and environment.
printf '%s\n' '#!/usr/bin/env bash' \
  '[[ "${PYTHONDONTWRITEBYTECODE:-}" == 1 ]] || exit 41' \
  'if [[ "$#" -eq 1 && "$1" == "${PROBE_TEST_ROOT}/test-docker-application-expectations.py" ]]; then' \
  '  name=test-docker-application-expectations.py' \
  'elif [[ "$#" -eq 2 && "$1" == "${PROBE_TEST_ROOT}/lib/docker-application-expectations.py" && "$2" == check-sources ]]; then' \
  '  name="docker-application-expectations.py check-sources"' \
  'elif [[ "$#" -eq 1 && "$1" == "${PROBE_TEST_ROOT}/test-docker-application-schedule.py" ]]; then' \
  '  name=test-docker-application-schedule.py' \
  'elif [[ "$#" -eq 1 && "$1" == "${PROBE_TEST_ROOT}/test-docker-forgejo-authored-fields.py" ]]; then' \
  '  name=test-docker-forgejo-authored-fields.py' \
  'elif [[ "$#" -eq 1 && "$1" == "${PROBE_TEST_ROOT}/test-docker-core-artifact.py" ]]; then' \
  '  name=test-docker-core-artifact.py' \
  'elif [[ "$#" -eq 2 && "$1" == "${PROBE_TEST_ROOT}/lib/docker-core-artifact.py" && "$2" == check-sources ]]; then' \
  '  name="docker-core-artifact.py check-sources"' \
  'elif [[ "$#" -eq 1 && "$1" == "${PROBE_TEST_ROOT}/test-docker-application-contract.py" ]]; then' \
  '  name=test-docker-application-contract.py' \
  'elif [[ "$#" -eq 1 && "$1" == "${PROBE_TEST_ROOT}/test-native-presence.py" ]]; then' \
  '  name=test-native-presence.py' \
  'else' \
  '  exit 42' \
  'fi' \
  'printf "%s\n" "${name}" >> "${PROBE_TEST_LOG}"' \
  '[[ "${name}" != "${PROBE_FAIL:-}" ]] || exit 37' > "${test_root}/bin/python3"
chmod +x -- "${test_root}/bin/python3"

probe_log="${test_root}/probes.log"
run_probe_test() {
  PROBE_TEST_LOG="${probe_log}" PROBE_TEST_ROOT="${test_root}" PROBE_FAIL="${1:-}" \
    PYTHONDONTWRITEBYTECODE=0 PATH="${test_root}/bin:${PATH}" bash "${test_root}/test-application-probes.sh"
}
readonly existing_order=$'test-application-probes-runner.sh\ntest-docker-application-expectations.py\ndocker-application-expectations.py check-sources\ntest-docker-application-schedule.py\ntest-docker-forgejo-authored-fields.py\ntest-application-export-privacy.sh\ntest-forgejo-application-probes.sh\ntest-nextcloud-application-probes.sh\ntest-paperless-application-probes.sh\ntest-immich-application-probes.sh\ntest-observability-application-probes.sh'
run_probe_test
[[ "$(< "${probe_log}")" == "${existing_order}"$'\ntest-docker-core-artifact.py\ndocker-core-artifact.py check-sources\ntest-docker-application-contract.py\ntest-native-presence.py\ntest-podman-live-cleanup.sh' ]]
: > "${probe_log}"
status=0
run_probe_test test-docker-application-expectations.py || status=$?
[[ "${status}" == 37 ]]
[[ "$(< "${probe_log}")" == $'test-application-probes-runner.sh\ntest-docker-application-expectations.py' ]]
: > "${probe_log}"
status=0
run_probe_test 'docker-application-expectations.py check-sources' || status=$?
[[ "${status}" == 37 ]]
[[ "$(< "${probe_log}")" == $'test-application-probes-runner.sh\ntest-docker-application-expectations.py\ndocker-application-expectations.py check-sources' ]]
: > "${probe_log}"
status=0
run_probe_test test-docker-application-schedule.py || status=$?
[[ "${status}" == 37 ]]
[[ "$(< "${probe_log}")" == $'test-application-probes-runner.sh\ntest-docker-application-expectations.py\ndocker-application-expectations.py check-sources\ntest-docker-application-schedule.py' ]]
: > "${probe_log}"
status=0
run_probe_test test-docker-forgejo-authored-fields.py || status=$?
[[ "${status}" == 37 ]]
[[ "$(< "${probe_log}")" == $'test-application-probes-runner.sh\ntest-docker-application-expectations.py\ndocker-application-expectations.py check-sources\ntest-docker-application-schedule.py\ntest-docker-forgejo-authored-fields.py' ]]
: > "${probe_log}"
status=0
run_probe_test test-paperless-application-probes.sh || status=$?
[[ "${status}" == 37 ]]
[[ "$(< "${probe_log}")" == $'test-application-probes-runner.sh\ntest-docker-application-expectations.py\ndocker-application-expectations.py check-sources\ntest-docker-application-schedule.py\ntest-docker-forgejo-authored-fields.py\ntest-application-export-privacy.sh\ntest-forgejo-application-probes.sh\ntest-nextcloud-application-probes.sh\ntest-paperless-application-probes.sh' ]]
: > "${probe_log}"
status=0
run_probe_test test-observability-application-probes.sh || status=$?
[[ "${status}" == 37 ]]
[[ "$(< "${probe_log}")" == $'test-application-probes-runner.sh\ntest-docker-application-expectations.py\ndocker-application-expectations.py check-sources\ntest-docker-application-schedule.py\ntest-docker-forgejo-authored-fields.py\ntest-application-export-privacy.sh\ntest-forgejo-application-probes.sh\ntest-nextcloud-application-probes.sh\ntest-paperless-application-probes.sh\ntest-immich-application-probes.sh\ntest-observability-application-probes.sh' ]]

: > "${probe_log}"
status=0
run_probe_test test-docker-core-artifact.py || status=$?
[[ "${status}" == 37 ]]
[[ "$(< "${probe_log}")" == "${existing_order}"$'\ntest-docker-core-artifact.py' ]]

: > "${probe_log}"
status=0
run_probe_test 'docker-core-artifact.py check-sources' || status=$?
[[ "${status}" == 37 ]]
[[ "$(< "${probe_log}")" == "${existing_order}"$'\ntest-docker-core-artifact.py\ndocker-core-artifact.py check-sources' ]]

: > "${probe_log}"
status=0
run_probe_test test-docker-application-contract.py || status=$?
[[ "${status}" == 37 ]]
[[ "$(< "${probe_log}")" == "${existing_order}"$'\ntest-docker-core-artifact.py\ndocker-core-artifact.py check-sources\ntest-docker-application-contract.py' ]]
: > "${probe_log}"
status=0
run_probe_test test-native-presence.py || status=$?
[[ "${status}" == 37 ]]
[[ "$(< "${probe_log}")" == "${existing_order}"$'\ntest-docker-core-artifact.py\ndocker-core-artifact.py check-sources\ntest-docker-application-contract.py\ntest-native-presence.py' ]]
: > "${probe_log}"
status=0
run_probe_test test-podman-live-cleanup.sh || status=$?
[[ "${status}" == 37 ]]
[[ "$(< "${probe_log}")" == "${existing_order}"$'\ntest-docker-core-artifact.py\ndocker-core-artifact.py check-sources\ntest-docker-application-contract.py\ntest-native-presence.py\ntest-podman-live-cleanup.sh' ]]
