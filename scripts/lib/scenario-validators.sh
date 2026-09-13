#!/usr/bin/env bash
# Reusable native, neutral-projection, reimport, and runtime assertions.
# Sourcing defines functions only. The entry point owns isolation, deadline and
# progress helpers, runtime variables, artifact paths, and cleanup.

assert_successful_conversion() {
  local output=$1 selection=$2 output_directory=$3 report=$4
  local membership_contract=${5:-shared}
  jq --exit-status '
    .schema_version == 1 and .status == "success" and .exit_category == "success" and
    (.primary_diagnostic_code == null) and
    ([.diagnostics[]? | select(.severity == "error" or .code == "BFP0001")] | length == 0) and
    ((.output_artifacts | length) > 0)
  ' "${report}" > /dev/null || {
    printf 'Conversion report is not a successful BFP0001-free result: %s\n' "${report}" >&2
    return 1
  }
  [[ -d "${output_directory}" ]] && [[ -n "$(find "${output_directory}" -type f -print -quit)" ]] || {
    printf 'Conversion wrote no artifacts: %s\n' "${output_directory}" >&2
    return 1
  }
  local expected_count actual_count
  expected_count="$(jq '.output_artifacts | length' "${report}")"
  actual_count="$(find "${output_directory}" -maxdepth 1 -type f | wc -l)"
  [[ "${expected_count}" == "${actual_count}" ]] || {
    printf 'Report/artifact count mismatch for %s: report=%s files=%s\n' \
      "${output_directory}" "${expected_count}" "${actual_count}" >&2
    return 1
  }
  case "${output}" in
    compose) [[ -s "${output_directory}/compose.yaml" ]] ;;
    quadlet) find "${output_directory}" -maxdepth 1 -type f -name '*.container' -print -quit | grep --quiet . ;;
    podman)
      [[ -s "${output_directory}/podman.json" && -s "${output_directory}/podman-commands.sh" ]]
      sh -n "${output_directory}/podman-commands.sh"
      local operation_count command_count
      operation_count="$(jq '.operations | length' "${output_directory}/podman.json")"
      command_count="$(grep --count '^podman ' "${output_directory}/podman-commands.sh")"
      [[ "${operation_count}" == "${command_count}" ]] || {
        printf 'Podman operation/script count mismatch in %s: operations=%s commands=%s\n' \
          "${output_directory}" "${operation_count}" "${command_count}" >&2
        return 1
      }
      jq --exit-status '
        .schema_version == 1 and (.operations | length > 0) and
        all(.operations[]; .cli.program == "podman" and (.cli.argv | type == "array" and length > 0))
      ' "${output_directory}/podman.json" > /dev/null
      ;;
    *)
      printf 'Unknown output assertion: %s\n' "${output}" >&2
      return 1
      ;;
  esac
  case "${membership_contract}" in
    shared)
      assert_selection_membership "${selection}" "${output}" "${output_directory}"
      ;;
    scenario-specific) ;;
    *)
      printf 'Unknown selection-membership contract: %s\n' "${membership_contract}" >&2
      return 1
      ;;
  esac
}

assert_named_member() {
  local output=$1 directory=$2 name=$3 native=$4
  case "${output}" in
    compose)
      awk -v short="${name}" -v native="${native}" '
        $0 == "services:" { inside = 1; next }
        inside && /^[^ ]/ { inside = 0 }
        inside && ($0 == "  " short ":" || $0 == "  " native ":") { found = 1 }
        END { exit !found }
      ' "${directory}/compose.yaml"
      ;;
    quadlet)
      find "${directory}" -maxdepth 1 -type f \( -name "${name}.container" -o -name "${native}.container" \) \
        -print -quit | grep --quiet .
      ;;
    podman)
      jq --exit-status --arg short "${name}" --arg native "${native}" \
        'any(.operations[]?; .resource.kind == "container" and
          (.resource.name == $short or .resource.name == $native))' \
        "${directory}/podman.json" > /dev/null
      ;;
  esac
}

assert_named_absent() {
  local output=$1 directory=$2 name=$3 native=$4
  if assert_named_member "${output}" "${directory}" "${name}" "${native}"; then
    printf 'Selection unexpectedly retained %s in %s.\n' "${native}" "${directory}" >&2
    return 1
  fi
}

assert_resource_member() {
  local output=$1 directory=$2 kind=$3 name=$4
  if case "${output}" in
    compose)
      awk -v section="${kind}s" -v key="${name}" '
        $0 == section ":" { inside = 1; next }
        inside && /^[^ ]/ { inside = 0 }
        inside && ($0 == "  " key ":" || $0 == "  " key ": {}") { found = 1 }
        END { exit !found }
      ' "${directory}/compose.yaml"
      ;;
    quadlet)
      if [[ "${kind}" == network ]]; then
        grep --recursive --extended-regexp --quiet "^Network=${name}(\\.network)?$" "${directory}"
      else
        grep --recursive --extended-regexp --quiet "^Volume=${name}(\\.volume)?(:|$)" "${directory}"
      fi
      ;;
    podman)
      jq --exit-status --arg kind "${kind}" --arg name "${name}" \
        '[.external_preconditions[]?, .operations[]?.resource] |
         any(.kind == $kind and .name == $name)' "${directory}/podman.json" > /dev/null
      ;;
  esac then
    return 0
  fi

  printf 'Expected %s resource %s is absent from %s output in %s.\n' \
    "${kind}" "${name}" "${output}" "${directory}" >&2
  return 1
}

assert_resource_absent() {
  local output=$1 directory=$2 kind=$3 name=$4
  if assert_resource_member "${output}" "${directory}" "${kind}" "${name}" 2> /dev/null; then
    printf 'Selection unexpectedly retained %s %s in %s.\n' "${kind}" "${name}" "${directory}" >&2
    return 1
  fi
}

assert_network_boundary_semantics() {
  local output=$1 directory=$2
  local api="${current_prefix:?caller must supply current_prefix}-large-api"
  local private="${current_prefix:?caller must supply current_prefix}-large-private"
  local edge="${current_prefix:?caller must supply current_prefix}-large-edge"
  local cache="${current_prefix:?caller must supply current_prefix}-large-cache"
  local dual_attachment=true
  if [[ "${current_podman_major}" -lt 4 && "${current_podman_rootless}" == true ]]; then
    dual_attachment=false
  fi
  case "${output}" in
    compose)
      local service_block="${current_case:?caller must supply current_case}/api-service.compose.yaml"
      awk -v service="${api}" '
        $0 == "  " service ":" { inside = 1 }
        inside && $0 ~ /^  [^ ].*:$/ && $0 != "  " service ":" { exit }
        inside { print }
      ' "${directory}/compose.yaml" > "${service_block}"
      grep --fixed-strings --quiet "source: ${cache}" "${service_block}"
      grep --fixed-strings --quiet 'target: /cache' "${service_block}"
      grep --fixed-strings --quiet 'read_only: true' "${service_block}"
      grep --fixed-strings --quiet "${private}: {}" "${service_block}"
      if [[ "${dual_attachment}" == true ]]; then
        grep --fixed-strings --quiet "${edge}: {}" "${service_block}"
      else
        local proxy_block="${current_case:?caller must supply current_case}/proxy-service.compose.yaml"
        awk -v service="${current_prefix:?caller must supply current_prefix}-large-proxy" '
          $0 == "  " service ":" { inside = 1 }
          inside && $0 ~ /^  [^ ].*:$/ && $0 != "  " service ":" { exit }
          inside { print }
        ' "${directory}/compose.yaml" > "${proxy_block}"
        grep --fixed-strings --quiet "${edge}: {}" "${proxy_block}"
      fi
      ;;
    quadlet)
      local api_unit="${directory}/${api}.container"
      [[ -s "${api_unit}" ]]
      grep --extended-regexp --quiet "^Network=${private}(\\.network)?$" "${api_unit}" || {
        printf 'Quadlet API unit is missing private network %s: %s\n' \
          "${private}" "${api_unit}" >&2
        return 1
      }
      if [[ "${dual_attachment}" == true ]]; then
        grep --extended-regexp --quiet "^Network=${edge}(\\.network)?$" "${api_unit}" || {
          printf 'Quadlet API unit is missing edge network %s: %s\n' \
            "${edge}" "${api_unit}" >&2
          return 1
        }
      else
        local proxy_unit="${directory}/${current_prefix:?caller must supply current_prefix}-large-proxy.container"
        grep --extended-regexp --quiet "^Network=${edge}(\\.network)?$" "${proxy_unit}" || {
          printf 'Quadlet proxy unit is missing edge network %s: %s\n' \
            "${edge}" "${proxy_unit}" >&2
          return 1
        }
      fi
      grep --extended-regexp --quiet \
        "^Volume=${cache}(\\.volume)?:/cache:ro(,.*)?$" "${api_unit}" || {
        printf 'Quadlet API unit is missing read-only cache volume %s: %s\n' \
          "${cache}" "${api_unit}" >&2
        return 1
      }
      ;;
    podman)
      if [[ "${dual_attachment}" == true ]]; then
        jq --exit-status --arg api "${api}" --arg private "${private}" --arg edge "${edge}" --arg cache "${cache}" '
          any(.operations[]?;
            .resource.kind == "container" and .resource.name == $api and
            (.libpod.body.json.Networks[$private] != null) and
            (.libpod.body.json.Networks[$edge] != null) and
            any(.libpod.body.json.volumes[]?;
              .Name == $cache and .Dest == "/cache" and (.Options | index("ro") != null)))
        ' "${directory}/podman.json" > /dev/null
      else
        jq --exit-status --arg api "${api}" --arg proxy "${current_prefix:?caller must supply current_prefix}-large-proxy" \
          --arg private "${private}" --arg edge "${edge}" --arg cache "${cache}" '
          any(.operations[]?;
            .resource.kind == "container" and .resource.name == $api and
            (.libpod.body.json.Networks[$private] != null) and
            any(.libpod.body.json.volumes[]?;
              .Name == $cache and .Dest == "/cache" and (.Options | index("ro") != null))) and
          any(.operations[]?;
            .resource.kind == "container" and .resource.name == $proxy and
            (.libpod.body.json.Networks[$edge] != null))
        ' "${directory}/podman.json" > /dev/null
      fi
      ;;
  esac
}

assert_selection_membership() {
  local selection=$1 output=$2 directory=$3 expected excluded name
  case "${selection}" in
    exact | exact-repeat)
      expected='web'
      excluded='api'
      ;;
    prefix)
      expected='api db worker proxy cache'
      excluded='web stopped'
      ;;
    label | all)
      expected='web api db worker stopped options'
      if [[ "${current_podman_major}" -ge 4 ]]; then
        expected+=' pod-member pod-secondary-member'
      fi
      excluded=''
      ;;
    network-boundary)
      expected='api db worker proxy cache options'
      excluded='web'
      ;;
    section-independent)
      expected='stopped'
      excluded='web api'
      ;;
    *) return 0 ;;
  esac
  for name in ${expected}; do
    assert_named_member "${output}" "${directory}" "${name}" "${current_prefix:?caller must supply current_prefix}-${name}" ||
      assert_named_member "${output}" "${directory}" "${name}" "${current_prefix:?caller must supply current_prefix}-small-${name}" ||
      assert_named_member "${output}" "${directory}" "${name}" "${current_prefix:?caller must supply current_prefix}-large-${name}" || {
      printf 'Selection %s omitted expected member %s from %s.\n' "${selection}" "${name}" "${directory}" >&2
      return 1
    }
  done
  for name in ${excluded}; do
    assert_named_absent "${output}" "${directory}" "${name}" "${current_prefix:?caller must supply current_prefix}-${name}"
    assert_named_absent "${output}" "${directory}" "${name}" "${current_prefix:?caller must supply current_prefix}-small-${name}"
    assert_named_absent "${output}" "${directory}" "${name}" "${current_prefix:?caller must supply current_prefix}-large-${name}"
  done
  if [[ "${selection}" == network-boundary ]]; then
    assert_resource_member "${output}" "${directory}" network "${current_prefix:?caller must supply current_prefix}-large-private"
    assert_resource_member "${output}" "${directory}" network "${current_prefix:?caller must supply current_prefix}-large-edge"
    assert_resource_member "${output}" "${directory}" volume "${current_prefix:?caller must supply current_prefix}-large-data"
    assert_resource_member "${output}" "${directory}" volume "${current_prefix:?caller must supply current_prefix}-large-cache"
    assert_resource_absent "${output}" "${directory}" network "${current_prefix:?caller must supply current_prefix}-small-net"
    assert_resource_absent "${output}" "${directory}" volume "${current_prefix:?caller must supply current_prefix}-small-data"
    assert_network_boundary_semantics "${output}" "${directory}"
  fi
}

assert_deterministic_exact_compose() {
  local socket=$1
  run_convert compose "${socket}" exact-repeat --podman-resource "container=${current_prefix:?caller must supply current_prefix}-small-web"
  diff --recursive --unified "${current_case:?caller must supply current_case}/outputs/exact-compose" "${current_case:?caller must supply current_case}/outputs/exact-repeat-compose"
}

assert_neutral_projection_equivalent() {
  local left=$1 right=$2 assertion=$3 side input output report
  local projection_root="${current_case:?caller must supply current_case}/neutral-projections/${assertion}"
  mkdir -p -- "${projection_root}"
  for side in left right; do
    if [[ "${side}" == left ]]; then input="${left}"; else input="${right}"; fi
    output="${projection_root}/${side}"
    report="${projection_root}/${side}.report.json"
    boxferry_operation "BoxFerry ${assertion} ${side} neutral projection" \
      convert compose compose --input-file "${input}" --loss-policy partial \
      --output-directory "${output}" --console-format json > "${report}"
    jq --exit-status '
      .schema_version == 1 and .status == "success" and .exit_category == "success"
    ' "${report}" > /dev/null
    [[ -s "${output}/compose.yaml" ]]
    canonicalize_compose_semantics "${output}/compose.yaml" "${projection_root}/${side}.semantic.yaml"
  done
  diff --unified "${projection_root}/left.semantic.yaml" "${projection_root}/right.semantic.yaml"
}

canonicalize_compose_semantics() {
  local input=$1 output=$2
  python3 "${repository_root:?caller must supply repository_root}/fixtures/conformance/podman-live/canonicalize_compose.py" "${input}" "${output}"
}

assert_strict_policy_blocks() {
  local socket=$1 report="${current_case:?caller must supply current_case}/strict-policy.report.json" error="${current_case:?caller must supply current_case}/strict-policy.stderr"
  if ! expected_failure_operation 90s 'BoxFerry strict-policy validation' 2 \
    "${boxferry_bin:?caller must supply boxferry_bin}" validate podman compose --podman-socket "${socket}" \
    --application-name "${current_prefix:?caller must supply current_prefix}" --podman-resource "container=${current_prefix:?caller must supply current_prefix}-small-web" \
    --loss-policy exact --console-format json > "${report}" 2> "${error}"; then
    printf '%s\n' 'Strict loss policy did not fail with the expected command status.' >&2
    return 1
  fi
  [[ ! -s "${error}" ]] || {
    printf 'Strict policy produced presentation stderr instead of structured JSON: %s\n' "${error}" >&2
    return 1
  }
  jq --exit-status '
    .schema_version == 1 and (.status == "blocked" or .status == "failure") and
    (.primary_diagnostic_code == "BFP0002" or .primary_diagnostic_code == "BFP0003") and
    any(.diagnostics[]?; .code == "BFP0002" or .code == "BFP0003") and
    (.fix_first.code == .primary_diagnostic_code)
  ' "${report}" > /dev/null
}

run_reimports() {
  local version=${1:?caller must supply declared Podman version}
  local selection input output reimport_directory
  mkdir -p -- "${current_case:?caller must supply current_case}/reimports"
  for selection in exact prefix label all network-boundary; do
    for input in compose quadlet; do
      require_scenario "${input}-reimports"
      reimport_directory="${current_case:?caller must supply current_case}/outputs/${selection}-${input}"
      for output in compose quadlet podman; do
        local -a command=("${boxferry_bin:?caller must supply boxferry_bin}" convert "${input}" "${output}" --loss-policy partial)
        if [[ "${input}" == compose ]]; then
          command+=(--input-file "${reimport_directory}/compose.yaml")
        else
          while IFS= read -r -d '' file; do command+=(--input-file "${file}"); done < <(find "${reimport_directory}" -maxdepth 1 -type f -print0 | sort -z)
          command+=(--application-name "${current_prefix:?caller must supply current_prefix}")
        fi
        if [[ "${output}" == podman ]]; then command+=(--podman-target-context rootful); fi
        local result="${current_case:?caller must supply current_case}/reimports/${selection}-${input}-to-${output}"
        local report="${result}.report.json"
        command+=(--output-directory "${result}" --console-format json)
        timed_operation 90s "BoxFerry ${selection} ${input}-to-${output} reimport" \
          "${command[@]}" > "${report}"
        assert_successful_conversion "${output}" "${selection}" "${result}" "${report}"
      done
    done
    require_scenario neutral-projection-equivalence
    assert_neutral_projection_equivalent \
      "${current_case:?caller must supply current_case}/outputs/${selection}-compose/compose.yaml" \
      "${current_case:?caller must supply current_case}/reimports/${selection}-compose-to-compose/compose.yaml" \
      "${selection}-compose-reimport"
    if [[ "${selection}" != all ]]; then
      assert_neutral_projection_equivalent \
        "${current_case:?caller must supply current_case}/outputs/${selection}-compose/compose.yaml" \
        "${current_case:?caller must supply current_case}/reimports/${selection}-quadlet-to-compose/compose.yaml" \
        "${selection}-quadlet-reimport"
    else
      assert_default_podman_network_evidence \
        "${current_case:?caller must supply current_case}/outputs/all-quadlet.report.json" \
        "${current_case:?caller must supply current_case}/feature-gates.txt" \
        "${version}" "${current_podman_rootless}" "${current_default_podman_network_present}" || return 1
    fi
  done
}

legacy_default_network_api_for_observation() {
  local version=${1:?caller must supply declared Podman version}
  local rootless=${2:?caller must supply rootless state}

  # These are reviewed observations, not a version-family rule.  The listed
  # version/mode combinations expose the default CNI network only through CniConfig, so
  # PodmanLens must retain the bounded untyped-field finding rather than
  # pretending the resource is portable Quadlet intent.
  case "${version}:${rootless}" in
    3.0.1:false) printf '%s\n' 3.0.0 ;;
    3.4.4:false | 3.4.4:true) printf '%s\n' 3.4.4 ;;
    *) return 1 ;;
  esac
}

assert_legacy_default_network_evidence() {
  local report=${1:?caller must supply all-resource Quadlet report}
  local version=${2:?caller must supply declared Podman version}
  local api_version=${3:?caller must supply declared Podman API version}
  local root_mode=${4:?caller must supply root mode}

  jq --exit-status --arg version "${version}" --arg api_version "${api_version}" '
    .status == "success"
    and any(
      .diagnostics[]?;
      .code == "BFP0002"
      and .source_code == "PLN0023"
      and any(.fields[]?; .name == "subject" and .value == "network:podman")
      and any(
        .fields[]?;
        .name == "reason"
        and .value == "PodmanLens found native response fields without typed portable mappings; path descriptors were retained without values"
      )
      and any(.fields[]?; .name == "decision" and .value == "omitted")
      and any(.fields[]?; .name == "source_engine" and .value == $version)
      and any(.fields[]?; .name == "source_api" and .value == $api_version)
      and any(.fields[]?; .name == "native_path" and .value == "$.CniConfig")
      and any(
        .fields[]?;
        .name == "native_value_policy"
        and .value == "field paths retained; native values not retained"
      )
    )
    and (any(
      .diagnostics[]?;
      .code == "BFQ0007"
      and any(.fields[]?; .name == "subject" and .value == "networks.podman")
    ) | not)
  ' "${report}" > /dev/null || {
    printf '%s\n' \
      "Podman ${version} ${root_mode} default CNI network did not retain its exact untyped acquisition evidence." >&2
    return 1
  }
}

assert_default_podman_network_evidence() {
  local report=${1:?caller must supply all-resource Quadlet report}
  local feature_gates=${2:?caller must supply feature-gates path}
  local version=${3:?caller must supply declared Podman version}
  local rootless=${4:?caller must supply rootless state}
  local present=${5:?caller must supply default-network inventory state}
  local legacy_api_version=
  local root_mode=rootful

  if [[ "${rootless}" == true ]]; then
    root_mode=rootless
  fi

  legacy_api_version=$(legacy_default_network_api_for_observation "${version}" "${rootless}") || true
  if [[ "${present}" == true && -n "${legacy_api_version}" ]]; then
    assert_legacy_default_network_evidence \
      "${report}" "${version}" "${legacy_api_version}" "${root_mode}" || return 1
    printf '%s\n' \
      "Podman ${version} ${root_mode} default CNI network remains omitted with exact retained acquisition evidence" \
      >> "${feature_gates}"
  elif [[ "${present}" == true ]]; then
    jq --exit-status '
      .status == "success"
      and any(
        .diagnostics[]?;
        .code == "BFQ0007"
        and any(.fields[]?; .name == "subject" and .value == "networks.podman")
        and any(
          .fields[]?;
          .name == "reason"
          and .value == "network lifecycle ownership is uncertain; no managed .network unit was generated"
        )
      )
    ' "${report}" > /dev/null || {
      printf '%s\n' \
        'All-resource Quadlet export did not report its uncertain unreferenced default network.' >&2
      return 1
    }
    printf '%s\n' \
      'all-resource Quadlet export diagnoses the uncertain default Podman network before reimport' \
      >> "${feature_gates}"
  else
    printf '%s\n' \
      'default Podman network absent from live inventory; default-network ownership diagnostic is inapplicable' \
      >> "${feature_gates}"
  fi
}

scenario_podman_socket() {
  local socket=${1:?scenario validator must supply socket}
  local subcommand=${2:?scenario validator must supply Podman command}
  shift 2
  podman_socket "${socket}" "validate runtime scenario via Podman ${subcommand}" \
    "${subcommand}" "$@"
}

assert_runtime_scenarios() {
  local socket=$1 version=$2 major
  major="${version%%.*}"
  current_podman_major="${major}"
  current_podman_rootless="$(scenario_podman_socket "${socket}" info --format '{{.Host.Security.Rootless}}')"
  current_default_podman_network_present="$(
    scenario_podman_socket "${socket}" network ls --format json | jq --raw-output \
      'any(.[]?; .Name == "podman")'
  )"
  [[ "${current_default_podman_network_present}" == true || "${current_default_podman_network_present}" == false ]]
  printf 'Live scenario: stopped-and-running\n'
  require_scenario stopped-and-running
  [[ "$(scenario_podman_socket "${socket}" inspect --format '{{.State.Status}}' "${current_prefix:?caller must supply current_prefix}-running")" == running ]]
  local stopped_status
  stopped_status="$(scenario_podman_socket "${socket}" inspect --format '{{.State.Status}}' "${current_prefix:?caller must supply current_prefix}-stopped")"
  [[ "${stopped_status}" == created || "${stopped_status}" == configured ]]
  printf 'Live scenario: pod-members-and-standalone\n'
  require_scenario pod-members-and-standalone
  if ((major >= 4)); then
    [[ -n "$(scenario_podman_socket "${socket}" inspect --format '{{.Pod}}' "${current_prefix:?caller must supply current_prefix}-pod-member")" ]]
    [[ -n "$(scenario_podman_socket "${socket}" inspect --format '{{.Pod}}' "${current_prefix:?caller must supply current_prefix}-pod-secondary-member")" ]]
    [[ "$(scenario_podman_socket "${socket}" inspect --format '{{.Pod}}' "${current_prefix:?caller must supply current_prefix}-pod-member")" != "$(scenario_podman_socket "${socket}" inspect --format '{{.Pod}}' "${current_prefix:?caller must supply current_prefix}-pod-secondary-member")" ]]
    [[ -z "$(scenario_podman_socket "${socket}" inspect --format '{{.Pod}}' "${current_prefix:?caller must supply current_prefix}-small-web")" ]]
  else
    printf 'pod membership skipped by explicit Podman %s nested-runtime feature gate\n' "${version}" >> "${current_case:?caller must supply current_case}/feature-gates.txt"
  fi
  printf 'Live scenario: image-identities\n'
  require_scenario image-identities
  scenario_podman_socket "${socket}" inspect "${current_prefix:?caller must supply current_prefix}-small-web" | jq --exit-status '
    (((.[0].Config.Image // "") | startswith("registry.example.invalid/boxferry/")) or
      ((.[0].ImageName // "") | startswith("registry.example.invalid/boxferry/"))) and
    (.[0].Image | type == "string" and length > 0)
  ' > /dev/null
  scenario_podman_socket "${socket}" inspect "${current_prefix:?caller must supply current_prefix}-large-db" | jq --exit-status '
    ((((.[0].Config.Image // "") | startswith("registry.example.invalid/boxferry/")) or
      ((.[0].ImageName // "") | startswith("registry.example.invalid/boxferry/"))) and
    (.[0].Image | type == "string" and length > 0))
  ' > /dev/null
  scenario_podman_socket "${socket}" inspect "${current_prefix:?caller must supply current_prefix}-stopped" | jq --exit-status '
    (((.[0].Config.Image // "") | startswith("registry.example.invalid/boxferry/")) or
      ((.[0].ImageName // "") | startswith("registry.example.invalid/boxferry/"))) and
    (.[0].Image | type == "string" and length > 0)
  ' > /dev/null
  printf 'Live scenario: network-boundaries\n'
  require_scenario network-boundaries
  scenario_podman_socket "${socket}" inspect "${current_prefix:?caller must supply current_prefix}-large-api" > "${current_case:?caller must supply current_case}/network-boundaries.inspect.json"
  if ((major >= 4)); then
    jq --exit-status --arg private "${current_prefix:?caller must supply current_prefix}-large-private" --arg edge "${current_prefix:?caller must supply current_prefix}-large-edge" '
      (.[0].NetworkSettings.Networks[$private].Aliases | index("api")) and
      (.[0].NetworkSettings.Networks[$edge].Aliases | index("public-api"))
    ' "${current_case:?caller must supply current_case}/network-boundaries.inspect.json" > /dev/null
  elif [[ "${current_podman_rootless}" == true ]]; then
    scenario_podman_socket "${socket}" inspect "${current_prefix:?caller must supply current_prefix}-large-proxy" > "${current_case:?caller must supply current_case}/network-boundaries-proxy.inspect.json"
    jq --exit-status --arg private "${current_prefix:?caller must supply current_prefix}-large-private" '
      .[0].NetworkSettings.Networks | has($private)
    ' "${current_case:?caller must supply current_case}/network-boundaries.inspect.json" > /dev/null
    jq --exit-status --arg edge "${current_prefix:?caller must supply current_prefix}-large-edge" '
      .[0].NetworkSettings.Networks | has($edge)
    ' "${current_case:?caller must supply current_case}/network-boundaries-proxy.inspect.json" > /dev/null
    printf 'dual rootless CNI attachment is unavailable in Podman %s; private API and edge proxy prove the boundary\n' \
      "${version}" >> "${current_case:?caller must supply current_case}/feature-gates.txt"
  else
    jq --exit-status --arg private "${current_prefix:?caller must supply current_prefix}-large-private" --arg edge "${current_prefix:?caller must supply current_prefix}-large-edge" '
      (.[0].NetworkSettings.Networks | has($private)) and
      (.[0].NetworkSettings.Networks | has($edge))
    ' "${current_case:?caller must supply current_case}/network-boundaries.inspect.json" > /dev/null
    printf 'network aliases are unavailable in Podman %s inspect evidence\n' "${version}" \
      >> "${current_case:?caller must supply current_case}/feature-gates.txt"
  fi
  printf 'Live scenario: mount-matrix\n'
  require_scenario mount-matrix
  scenario_podman_socket "${socket}" inspect "${current_prefix:?caller must supply current_prefix}-large-db" | jq --exit-status '
    [.[0].Mounts[]? | select(.Type == "volume" and .RW == true)] | length == 1
  ' > /dev/null
  scenario_podman_socket "${socket}" inspect "${current_prefix:?caller must supply current_prefix}-large-api" | jq --exit-status '
    [.[0].Mounts[]? | select(.Type == "volume" and .RW == false and .Destination == "/cache")] | length == 1
  ' > /dev/null
  scenario_podman_socket "${socket}" inspect "${current_prefix:?caller must supply current_prefix}-options" | jq --exit-status '
    ([.[0].Mounts[]? | select(.Type == "bind" and .RW == false and .Propagation == "rprivate")] | length == 1) and
    (([.[0].Mounts[]? | select(.Type == "tmpfs")] | length == 1) or
      (.[0].HostConfig.Tmpfs["/scratch"] != null))
  ' > /dev/null
  printf 'Live scenario: environment-matrix\n'
  require_scenario environment-matrix
  scenario_podman_socket "${socket}" inspect "${current_prefix:?caller must supply current_prefix}-options" | jq --exit-status '
    ([.[0].Config.Env[]? | select(. == "BOXFERRY_ENV_FILE=present" or . == "BOXFERRY_ENV=inline")] | length == 2) and
    ([.[0].Config.Env[]? | select(. == "BOXFERRY_PROTECTED_TOKEN=not-a-secret-test-value")] | length == 1)
  ' > /dev/null
  printf 'Live scenario: runtime-policy-matrix\n'
  require_scenario runtime-policy-matrix
  scenario_podman_socket "${socket}" inspect "${current_prefix:?caller must supply current_prefix}-options" > "${current_case:?caller must supply current_case}/options.inspect.json"
  if ((major >= 4)); then
    jq --exit-status '
      .[0].HostConfig.Memory > 0 and .[0].HostConfig.PidsLimit == 64 and
      (.[0].HostConfig.CapDrop | length > 0) and
      .[0].HostConfig.RestartPolicy.Name == "on-failure" and
      .[0].HostConfig.RestartPolicy.MaximumRetryCount == 3 and
      .[0].HostConfig.LogConfig.Type == "k8s-file" and
      (.[0].HostConfig.IpcMode == "private" or .[0].HostConfig.IpcMode == "shareable") and
      .[0].HostConfig.PidMode == "private" and
      .[0].HostConfig.UTSMode == "private" and
      ((.[0].HostConfig.Annotations["io.boxferry.live"] //
        .[0].Config.Annotations["io.boxferry.live"]) == "present")
    ' "${current_case:?caller must supply current_case}/options.inspect.json" > /dev/null
  else
    jq --exit-status '
      .[0].HostConfig.CapDrop | length > 0
    ' "${current_case:?caller must supply current_case}/options.inspect.json" > /dev/null
    printf 'resource controls skipped by explicit Podman %s feature gate\n' "${version}" >> "${current_case:?caller must supply current_case}/feature-gates.txt"
  fi
  printf 'Live scenario: healthy-and-unhealthy\n'
  require_scenario healthy-and-unhealthy
  if ((major >= 4)); then
    local healthy_status=starting unhealthy_status=starting
    for _ in {1..30}; do
      healthy_status="$(scenario_podman_socket "${socket}" inspect --format '{{.State.Health.Status}}' "${current_prefix:?caller must supply current_prefix}-healthy")"
      unhealthy_status="$(scenario_podman_socket "${socket}" inspect --format '{{.State.Health.Status}}' "${current_prefix:?caller must supply current_prefix}-unhealthy")"
      [[ "${healthy_status}" == healthy && "${unhealthy_status}" == unhealthy ]] && break
      sleep 1
    done
    printf 'healthy=%s\nunhealthy=%s\n' "${healthy_status}" "${unhealthy_status}" > "${current_case:?caller must supply current_case}/health-status.txt"
    [[ "${healthy_status}" == healthy && "${unhealthy_status}" == unhealthy ]] || {
      printf 'Unexpected health states: healthy=%s unhealthy=%s\n' "${healthy_status}" "${unhealthy_status}" >&2
      return 1
    }
  else
    printf 'health states skipped by explicit Podman %s feature gate\n' "${version}" >> "${current_case:?caller must supply current_case}/feature-gates.txt"
  fi
  printf 'Live scenario: secret-conditional\n'
  require_scenario secret-conditional
  if ((major >= 4)) &&
    scenario_podman_socket "${socket}" secret inspect "${current_prefix:?caller must supply current_prefix}-conditional" > /dev/null 2>&1; then
    scenario_podman_socket "${socket}" inspect "${current_prefix:?caller must supply current_prefix}-secret" | jq --exit-status \
      '[.. | strings | select(contains("conditional"))] | length > 0' > /dev/null
    printf 'secret endpoint supported\n' >> "${current_case:?caller must supply current_case}/feature-gates.txt"
  else
    printf 'secret endpoint unavailable\n' >> "${current_case:?caller must supply current_case}/feature-gates.txt"
  fi
}

assert_smoke_runtime_baseline() {
  local baseline="${socket_directory:?caller must supply socket_directory}/smoke-baseline.json"
  require_scenario stopped-and-running
  jq --exit-status --arg selected "${current_prefix:?caller must supply current_prefix}-small-web" \
    --arg running "${current_prefix:?caller must supply current_prefix}-running" --arg stopped "${current_prefix:?caller must supply current_prefix}-stopped" \
    --arg network "${current_prefix:?caller must supply current_prefix}-small-net" --arg volume "${current_prefix:?caller must supply current_prefix}-small-data" '
      any(.[]; .Name == $running and .State.Status == "running") and
      any(.[]; .Name == $stopped and
        (.State.Status == "created" or .State.Status == "configured")) and
      any(.[]; .Name == $selected and
        ((((.Config.Image // "") | startswith("registry.example.invalid/boxferry/")) or
          ((.ImageName // "") | startswith("registry.example.invalid/boxferry/"))) and
        (.NetworkSettings.Networks[$network] != null) and
        any(.Mounts[]?; .Name == $volume and .Destination == "/var/lib/boxferry" and .RW == true) and
        any(.Config.Env[]?; . == "BOXFERRY_LIVE_MODE=small")))
    ' "${baseline}" > /dev/null
}
