#!/usr/bin/env bash

set -Eeuo pipefail

temporary_directory="$(mktemp -d)"
readonly temporary_directory
trap 'rm -rf -- "${temporary_directory}"' EXIT
changelog="${temporary_directory}/CHANGELOG.md"
readonly changelog

write_changelog() {
  printf '%s\n' "$@" > "${changelog}"
}

check_metadata() {
  local version=$1
  local latest_tag=$2
  shift 2
  env \
    BOXFERRY_RELEASE_METADATA_TEST_MODE=1 \
    BOXFERRY_TEST_CHANGELOG="${changelog}" \
    BOXFERRY_TEST_LATEST_TAG="${latest_tag}" \
    BOXFERRY_TEST_VERSION="${version}" \
    bash scripts/validate-release-metadata.sh "$@"
}

expect_failure() {
  local label=$1
  local version=$2
  local latest_tag=$3
  local diagnostic=$4
  local status=0
  shift 4
  check_metadata "${version}" "${latest_tag}" "$@" > "${temporary_directory}/output" 2>&1 || status=$?
  if [[ "${status}" == 0 ]] || ! grep -Fq -- "${diagnostic}" "${temporary_directory}/output"; then
    printf 'Release metadata negative case failed: %s (exit %s)\n' "${label}" "${status}" >&2
    cat "${temporary_directory}/output" >&2
    exit 1
  fi
}

expect_failure_both() {
  local mode
  for mode in --development --release; do
    expect_failure "$@" "${mode}"
  done
}

write_changelog \
  '# Changelog' '' '## [Unreleased]' '' '### Fixed' '' '- Pending Docker work.' '' \
  '## [0.10.0] - 2026-10-28' '' '### Added' '' '- Prepared release notes.'
# Development after version preparation must not require publishing the prepared version.
check_metadata 0.10.0 v0.9.0 --development > /dev/null
check_metadata 0.10.0 v0.10.0 --development > /dev/null
check_metadata 0.10.0 '' --development > /dev/null
for latest_tag in v0.9.0 v0.10.0 ''; do
  expect_failure 'non-empty Unreleased during release preparation' 0.10.0 "${latest_tag}" \
    'Unreleased must be empty' --release
  expect_failure 'default release mode rejects pending notes for any tag state' 0.10.0 "${latest_tag}" \
    'Unreleased must be empty'
done

# Invalid arguments fail before Cargo or changelog inspection.
for arguments in '--unknown' '--development --release' '--release --release' '--development extra'; do
  read -r -a argument_list <<< "${arguments}"
  status=0
  check_metadata 0.10.0 v0.9.0 "${argument_list[@]}" > "${temporary_directory}/output" 2>&1 || status=$?
  if [[ "${status}" != 2 ]] || ! grep -Fq 'Usage:' "${temporary_directory}/output"; then
    printf 'Invalid validator arguments were not rejected: %s\n' "${arguments}" >&2
    exit 1
  fi
done
status=0
check_metadata 0.10.0 v0.9.0 '' > "${temporary_directory}/output" 2>&1 || status=$?
if [[ "${status}" != 2 ]] || ! grep -Fq 'Usage:' "${temporary_directory}/output"; then
  printf 'Empty validator argument was not rejected.\n' >&2
  exit 1
fi

write_changelog \
  '# Changelog' '' '## [Unreleased]' '' \
  '## [0.10.0] - 2026-10-28' '' '### Added' '' '- Complete release notes.'
for latest_tag in v0.9.0 v0.10.0 ''; do
  check_metadata 0.10.0 "${latest_tag}" --development > /dev/null
  check_metadata 0.10.0 "${latest_tag}" --release > /dev/null
  check_metadata 0.10.0 "${latest_tag}" > /dev/null
done
expect_failure_both 'newest release differs from workspace version' 0.9.0 v0.9.0 \
  'must match workspace version'

write_changelog \
  '# Changelog' '' '## [Unreleased]' '' \
  '## [0.10.0] - 2026-10-28' '' '- First section.' '' \
  '## [0.10.0] - 2026-10-28' '' '- Duplicate section.'
expect_failure_both 'duplicate current-version release section' 0.10.0 v0.9.0 \
  'exactly one release section'

write_changelog \
  '# Changelog' '' '## [Unreleased]' '' '## [Unreleased]' '' \
  '## [0.10.0] - 2026-10-28' '' '- Released fix.'
expect_failure_both 'duplicate Unreleased section' 0.10.0 v0.9.0 \
  'exactly one Unreleased section'

write_changelog '# Changelog' '' '## [0.10.0] - 2026-10-28' '' '- Released fix.'
expect_failure_both 'missing Unreleased section' 0.10.0 v0.9.0 \
  'exactly one Unreleased section'

write_changelog '# Changelog' '' '## [Unreleased]'
expect_failure_both 'missing current-version release section' 0.10.0 v0.9.0 \
  'must match workspace version'

for heading in '## [0.10.0]' '## [0.10.0] - tomorrow'; do
  write_changelog '# Changelog' '' '## [Unreleased]' '' "${heading}" '' '- Released fix.'
  expect_failure_both 'missing or malformed release date' 0.10.0 v0.9.0 \
    'must include a YYYY-MM-DD date'
done

write_changelog '# Changelog' '' '## [Unreleased]' '' '## [0.10.0] - 2026-10-28' ''
expect_failure_both 'empty numbered release notes' 0.10.0 v0.9.0 'is empty'
write_changelog '# Changelog' '' '## [Unreleased]' '' '## [0.10.0] - 2026-10-28' '' '---'
expect_failure_both 'unusable numbered release notes' 0.10.0 v0.9.0 'contains no usable notes'
rm -- "${changelog}"
expect_failure_both 'missing changelog' 0.10.0 v0.9.0 'Changelog does not exist'

# Exercise the production Cargo metadata path in an isolated repository fixture.
metadata_repository="${temporary_directory}/repository"
mkdir -p -- "${metadata_repository}/scripts" "${temporary_directory}/bin"
cp -- scripts/validate-release-metadata.sh scripts/extract-release-notes.sh "${metadata_repository}/scripts/"
cat > "${temporary_directory}/bin/cargo" << 'CARGO'
#!/usr/bin/env bash
set -Eeuo pipefail
[[ "$*" == 'metadata --locked --no-deps --format-version 1' ]]
printf '%s\n' "${BOXFERRY_TEST_CARGO_METADATA:?}"
CARGO
chmod +x "${temporary_directory}/bin/cargo"
write_changelog \
  '# Changelog' '' '## [Unreleased]' '' \
  '## [0.10.0] - 2026-10-28' '' '- Complete release notes.'
cp -- "${changelog}" "${metadata_repository}/CHANGELOG.md"
metadata="$(jq -n '{packages: (["boxferry-model", "boxferry-engine", "boxferry-compose", "boxferry-docker", "boxferry-podman", "boxferry-quadlet", "boxferry"] | map({name: ., version: "0.10.0", publish: null}))}')"
for mode in --development --release; do
  env PATH="${temporary_directory}/bin:${PATH}" BOXFERRY_TEST_CARGO_METADATA="${metadata}" \
    bash "${metadata_repository}/scripts/validate-release-metadata.sh" "${mode}" > /dev/null
  for mutation in '.packages[0].version = "0.9.0"' 'del(.packages[0])' '.packages[0].publish = []'; do
    invalid_metadata="$(jq "${mutation}" <<< "${metadata}")"
    status=0
    env PATH="${temporary_directory}/bin:${PATH}" BOXFERRY_TEST_CARGO_METADATA="${invalid_metadata}" \
      bash "${metadata_repository}/scripts/validate-release-metadata.sh" "${mode}" \
      > "${temporary_directory}/output" 2>&1 || status=$?
    if [[ "${status}" != 1 ]] || ! grep -Eq 'one common workspace package version|expected packages|packages must be publishable' "${temporary_directory}/output"; then
      printf 'Invalid package metadata was not rejected in %s: %s\n' "${mode}" "${mutation}" >&2
      cat "${temporary_directory}/output" >&2
      exit 1
    fi
  done
done

printf 'BoxFerry release metadata policy tests passed.\n'
