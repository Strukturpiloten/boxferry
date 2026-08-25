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
  env \
    BOXFERRY_RELEASE_METADATA_TEST_MODE=1 \
    BOXFERRY_TEST_CHANGELOG="${changelog}" \
    BOXFERRY_TEST_LATEST_TAG="${latest_tag}" \
    BOXFERRY_TEST_VERSION="${version}" \
    bash scripts/validate-release-metadata.sh
}

expect_failure() {
  local label=$1
  local version=$2
  local latest_tag=$3
  if check_metadata "${version}" "${latest_tag}" > /dev/null 2>&1; then
    printf 'Release metadata negative case unexpectedly passed: %s\n' "${label}" >&2
    exit 1
  fi
}

write_changelog \
  '# Changelog' \
  '' \
  '## [Unreleased]' \
  '' \
  '### Fixed' \
  '' \
  '- Pending fix.' \
  '' \
  '## [0.7.1] - 2026-08-25' \
  '' \
  '### Fixed' \
  '' \
  '- Released fix.'
check_metadata 0.7.1 v0.7.1 > /dev/null

write_changelog \
  '# Changelog' \
  '' \
  '## [Unreleased]' \
  '' \
  '## [0.8.0] - 2026-08-26' \
  '' \
  '### Added' \
  '' \
  '- Complete release notes.'
check_metadata 0.8.0 v0.7.1 > /dev/null

write_changelog \
  '# Changelog' \
  '' \
  '## [Unreleased]' \
  '' \
  '### Added' \
  '' \
  '- Included code omitted from release notes.' \
  '' \
  '## [0.8.0] - 2026-08-26' \
  '' \
  '### Fixed' \
  '' \
  '- A different fix.'
expect_failure 'non-empty Unreleased during release preparation' 0.8.0 v0.7.1

write_changelog \
  '# Changelog' \
  '' \
  '## [Unreleased]' \
  '' \
  '## [0.8.0] - 2026-08-26' \
  '' \
  '### Added' \
  '' \
  '- Notes for the wrong workspace version.'
expect_failure 'newest release differs from workspace version' 0.7.1 v0.7.1

write_changelog \
  '# Changelog' \
  '' \
  '## [Unreleased]' \
  '' \
  '## [0.8.0] - 2026-08-26' \
  '' \
  '### Added' \
  '' \
  '- First section.' \
  '' \
  '## [0.8.0] - 2026-08-26' \
  '' \
  '### Fixed' \
  '' \
  '- Duplicate section.'
expect_failure 'duplicate current-version release section' 0.8.0 v0.7.1

write_changelog \
  '# Changelog' \
  '' \
  '## [Unreleased]' \
  '' \
  '## [Unreleased]' \
  '' \
  '## [0.7.1] - 2026-08-25' \
  '' \
  '### Fixed' \
  '' \
  '- Released fix.'
expect_failure 'duplicate Unreleased section' 0.7.1 v0.7.1

printf 'BoxFerry release metadata policy tests passed.\n'
