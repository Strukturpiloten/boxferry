#!/usr/bin/env bash
# Shared read-only helpers for reviewed conformance scenario catalogues.
# The caller owns its progress counter, deadlines, isolation, and cleanup.

scenario_catalogue_validate() {
  local scenario_path=$1
  awk -F '\t' '
        NF && $1 !~ /^#/ {
            if (NF != 5 || $1 == "" || $2 == "" || $3 == "" || $4 == "" || $5 == "") {
                printf "Invalid scenario row at line %d: %s\\n", NR, $0 > "/dev/stderr"
                bad = 1
            }
            if (seen[$1]++) {
                printf "Duplicate scenario id at line %d: %s\\n", NR, $1 > "/dev/stderr"
                bad = 1
            }
        }
        END { exit bad }
    ' "${scenario_path}"
}

scenario_require() {
  local scenario_path=$1 expected=$2
  awk -F '\t' -v expected="${expected}" '
        $1 == expected { matches++ }
        END { exit matches == 1 ? 0 : 1 }
    ' "${scenario_path}"
}
