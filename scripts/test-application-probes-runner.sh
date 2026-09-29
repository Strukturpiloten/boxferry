#!/usr/bin/env bash
# Prove the shared probe runner invokes every suite and stops on failure.
set -Eeuo pipefail

script_directory="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
test_root="$(mktemp -d)"
trap 'rm -r -- "${test_root}"' EXIT
cp -- "${script_directory}/test-application-probes.sh" "${test_root}/"
for application in runner export-privacy forgejo nextcloud paperless immich; do
  if [[ "${application}" == runner ]]; then
    stub="${test_root}/test-application-probes-runner.sh"
  elif [[ "${application}" == export-privacy ]]; then
    stub="${test_root}/test-application-export-privacy.sh"
  else
    stub="${test_root}/test-${application}-application-probes.sh"
  fi
  # shellcheck disable=SC2016 # These are literal lines in generated test stubs.
  printf '%s\n' '#!/usr/bin/env bash' \
    'name="${0##*/}"' \
    'printf "%s\n" "${name}" >> "${PROBE_TEST_LOG}"' \
    '[[ "${name}" != "${PROBE_FAIL:-}" ]] || exit 37' > "${stub}"
done

probe_log="${test_root}/probes.log"
PROBE_TEST_LOG="${probe_log}" bash "${test_root}/test-application-probes.sh"
[[ "$(< "${probe_log}")" == $'test-application-probes-runner.sh\ntest-application-export-privacy.sh\ntest-forgejo-application-probes.sh\ntest-nextcloud-application-probes.sh\ntest-paperless-application-probes.sh\ntest-immich-application-probes.sh' ]]
: > "${probe_log}"
status=0
PROBE_TEST_LOG="${probe_log}" PROBE_FAIL=test-paperless-application-probes.sh \
  bash "${test_root}/test-application-probes.sh" || status=$?
[[ "${status}" == 37 ]]
[[ "$(< "${probe_log}")" == $'test-application-probes-runner.sh\ntest-application-export-privacy.sh\ntest-forgejo-application-probes.sh\ntest-nextcloud-application-probes.sh\ntest-paperless-application-probes.sh' ]]
