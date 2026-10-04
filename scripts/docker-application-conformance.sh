#!/usr/bin/env bash
set -Eeuo pipefail
umask 077

# A BoxFerry-owned, test-only disposable Docker boundary. This early scaffold
# proves catalogue/replay safety; it is not a Docker route or application gate.
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
contract="${script_dir}/lib/docker-application-contract.py"
profile='' lane='' lens_root='' lens_revision='' script_sha='' artifact='' artifact_sha256='' api_version=''
boxferry_root='' boxferry_binary='' boxferry_receipt=''

usage() {
  printf 'usage: %s --profile {catalogue|replay-probe|core-journey} --docker-lens-root PATH --docker-lens-revision SHA --native-script-sha256 SHA [--lane LANE --artifact PATH --api-version API] [--boxferry-root PATH --boxferry-binary PATH --boxferry-receipt PATH]\n' "$0" >&2
  exit 2
}

while (($#)); do
  (($# >= 2)) || usage
  case "$1" in
    --profile) profile=$2 ;;
    --lane) lane=$2 ;;
    --docker-lens-root) lens_root=$2 ;;
    --docker-lens-revision) lens_revision=$2 ;;
    --native-script-sha256) script_sha=$2 ;;
    --artifact) artifact=$2 ;;
    --api-version) api_version=$2 ;;
    --boxferry-root) boxferry_root=$2 ;;
    --boxferry-binary) boxferry_binary=$2 ;;
    --boxferry-receipt) boxferry_receipt=$2 ;;
    *) usage ;;
  esac
  shift 2
done
[[ -n $lens_root && -n $lens_revision && -n $script_sha ]] || usage
[[ $profile == catalogue || $profile == replay-probe || $profile == core-journey ]] || usage
for tool in python3 git; do command -v "$tool" > /dev/null || {
  printf 'missing required tool: %s\n' "$tool" >&2
  exit 1
}; done

# The caller names an exact, clean DockerLens checkout and script digest. No
# shell code from that checkout is sourced, and no image pin is copied here.
catalogue=$(python3 "$contract" catalogue --docker-lens-root "$lens_root" \
  --docker-lens-revision "$lens_revision" --native-script-sha256 "$script_sha")
if [[ $profile == catalogue ]]; then
  [[ -z $lane && -z $artifact && -z $api_version && -z $boxferry_root ]] || usage
  printf '%s\n' "$catalogue"
  exit 0
fi

[[ -n $lane && -n $api_version ]] || usage
if [[ $profile == core-journey ]]; then
  [[ -z $artifact && -n $boxferry_root && -n $boxferry_binary && -n $boxferry_receipt ]] || usage
  python3 "$contract" verify-candidate --boxferry-root "$boxferry_root" \
    --binary "$boxferry_binary" --receipt "$boxferry_receipt" \
    --docker-lens-root "$lens_root" --docker-lens-revision "$lens_revision" > /dev/null
else
  [[ -n $artifact && -z $boxferry_root && -z $boxferry_binary && -z $boxferry_receipt ]] || usage
fi
case "$lane" in
  debian11-rootful | debian11-rootless | upstream-rootful | upstream-rootless) ;;
  *) usage ;;
esac
for tool in podman curl jq timeout df du mktemp; do
  command -v "$tool" > /dev/null || {
    printf 'missing required tool: %s\n' "$tool" >&2
    exit 1
  }
done
[[ $EUID == 0 ]] || {
  printf 'outer Podman must be rootful; invoke this opt-in harness explicitly as root\n' >&2
  exit 1
}
bounded() {
  local status=0 duration=$1 now remaining limit
  shift
  if [[ ${guard_ready:-false} == true && ${cleanup_active:-false} == false ]]; then
    guard_alive || return 125
    now=$(cleanup_now) || return 125
    remaining=$((runtime_deadline - now - 8))
    ((remaining > 0)) || {
      printf 'Docker core execution deadline is exhausted\n' >&2
      return 124
    }
    [[ $duration =~ ^[0-9]+s$ ]] || return 125
    limit=${duration%s}
    ((limit <= remaining)) || duration="${remaining}s"
  fi
  timeout --signal=TERM --kill-after=5s "$duration" "$@" || status=$?
  if [[ ${guard_ready:-false} == true && ${cleanup_active:-false} == false ]]; then
    guard_alive || return 125
  fi
  return "$status"
}
outer_image=$(jq -er --arg lane "$lane" '.[$lane]' <<< "$catalogue")
fixture_image=$(jq -er '.fixture' <<< "$catalogue")
fixture_tag=${fixture_image%%@sha256:*}
fixture_version=${fixture_tag##*:}
core_alias="registry.invalid/boxferry-core/busybox:${fixture_version}"
outer_pull_attempted=false fixture_pull_attempted=false
report_host_cache() {
  printf 'host-image-cache: policy=retained-shared outer-pin=%s outer-pull-attempted=%s fixture-pin=%s fixture-pull-attempted=%s; no image removal or prune\n' \
    "$outer_image" "$outer_pull_attempted" "$fixture_image" "$fixture_pull_attempted" >&2
}
# Reject nonmatching or multi-operation artifacts before pulling or starting a daemon.
if [[ $profile == replay-probe ]]; then
  # Bind selected bytes once. This is identity only; independent intent/profile
  # checks remain mandatory, and subsequent stages cannot rebind changed bytes.
  artifact_sha256=$(python3 "$contract" artifact-identity --artifact "$artifact")
  if ! python3 "$contract" validate-artifact --artifact "$artifact" --image "$core_alias" \
    --artifact-sha256 "$artifact_sha256" \
    --api-version "$api_version" --lane "$lane" --catalogue-json "$catalogue" > /dev/null; then
    report_host_cache
    exit 1
  fi
fi

run_dir='' outer='' storage_volume='' watchdog_pid='' socket_path='' volume_path='' guard_pid='' guard_start=''
registered=false
main_pid=$$
main_start=$(python3 "$contract" parent-start --parent-pid "$main_pid")
signal_main() {
  python3 "$contract" signal-parent --parent-pid "$main_pid" \
    --parent-start "$main_start" > /dev/null 2>&1
}
guard_alive() {
  if ! python3 "$contract" parent-alive --parent-pid "$guard_pid" --parent-start "$guard_start" > /dev/null 2>&1; then
    printf 'independent deadline guard exited; refusing replay acceptance\n' >&2
    [[ -z $run_dir || ! -d $run_dir ]] || : > "$run_dir/guard-failure"
    return 1
  fi
}
cleanup_now() {
  local uptime rest
  read -r uptime rest < /proc/uptime || return 1
  [[ $uptime =~ ^[0-9]+\.[0-9]+$ ]] || return 1
  printf '%s\n' "${uptime%%.*}"
}
cleanup_bounded() {
  local maximum=${1%s} now remaining limit
  shift
  now=$(cleanup_now) || return 124
  remaining=$((cleanup_deadline - now - 7))
  ((remaining > 0)) || return 124
  limit=$maximum
  ((limit <= remaining)) || limit=$remaining
  timeout --signal=TERM --kill-after=5s "${limit}s" "$@"
}
cleanup_owned() {
  local failure=0 observed state inner_status now monitor_joined=true
  local outer_residual=none volume_residual=none directory_residual=none
  cleanup_active=true
  if [[ ${guard_ready:-false} == true ]]; then
    guard_alive || failure=1
  fi
  now=$(cleanup_now) || {
    now=0
    failure=1
  }
  cleanup_deadline=$((now + 95))
  if [[ -n $watchdog_pid ]]; then
    if [[ -n $run_dir && -d $run_dir ]] && : > "$run_dir/stop-watchdog"; then
      # Let bounded scans finish, but retain their paths if the monitor does
      # not exit within the aggregate cleanup budget.
      local watchdog_wait=0
      while kill -0 "$watchdog_pid" 2> /dev/null && ((watchdog_wait < 50)); do
        sleep 1
        watchdog_wait=$((watchdog_wait + 1))
      done
      if kill -0 "$watchdog_pid" 2> /dev/null; then
        failure=1
        monitor_joined=false
      else
        wait "$watchdog_pid" 2> /dev/null || failure=1
      fi
      [[ ! -e $run_dir/watchdog-failure ]] || failure=1
    else
      failure=1
      monitor_joined=false
    fi
    watchdog_pid=''
  fi
  if [[ $monitor_joined == false ]]; then
    # A still-running watchdog may be scanning either path. Keep every owned
    # resource and its stop marker so it can exit without racing deletion.
    [[ $registered != true || -z $outer ]] || outer_residual=$outer
    [[ $registered != true || -z $storage_volume ]] || volume_residual=$storage_volume
    [[ -z $run_dir ]] || directory_residual=$run_dir
    printf 'cleanup residuals: outer-container=%s storage-volume=%s private-directory=%s\n' \
      "$outer_residual" "$volume_residual" "$directory_residual" >&2
    return 1
  fi
  if [[ $registered == true && -n $outer ]]; then
    outer_residual=$outer
    state=0
    cleanup_bounded 15s podman container exists "$outer" || state=$?
    if ((state == 0)); then
      if observed=$(cleanup_bounded 15s podman inspect --format '{{index .Config.Labels "io.boxferry.docker-core-run"}}' "$outer") && [[ $observed == "$run_id" ]]; then
        if [[ -n $socket_path && -S $socket_path ]]; then
          # Only remove the fixed name if this run actually created it. A
          # failed removal is still failure even though outer storage goes.
          inner_status=$(cleanup_bounded 10s curl --silent --max-time 10 --max-filesize 65536 --output /dev/null --write-out '%{http_code}' \
            --unix-socket "$socket_path" "http://localhost/v${api_version}/containers/bf-docker-core/json") || failure=1
          if [[ $inner_status == 200 ]]; then
            cleanup_bounded 30s podman exec "$outer" docker -H unix:///boxferry-core/docker.sock \
              rm --force bf-docker-core > /dev/null 2>&1 || failure=1
            inner_status=$(cleanup_bounded 10s curl --silent --max-time 10 --max-filesize 65536 --output /dev/null --write-out '%{http_code}' \
              --unix-socket "$socket_path" "http://localhost/v${api_version}/containers/bf-docker-core/json") || failure=1
            [[ $inner_status == 404 ]] || failure=1
          elif [[ $inner_status != 404 ]]; then
            failure=1
          fi
        fi
        cleanup_bounded 60s podman rm --force --volumes "$outer" > /dev/null || failure=1
        state=0
        cleanup_bounded 15s podman container exists "$outer" > /dev/null 2>&1 || state=$?
        ((state == 1)) || failure=1
        if ((state == 1)); then outer_residual=none; fi
      else
        printf 'outer container ownership unverified; refusing removal\n' >&2
        failure=1
      fi
    elif ((state != 1)); then
      failure=1
    else
      outer_residual=none
    fi
  fi
  if [[ $registered == true && -n $storage_volume ]]; then
    volume_residual=$storage_volume
    state=0
    cleanup_bounded 15s podman volume exists "$storage_volume" || state=$?
    if ((state == 0)); then
      if observed=$(cleanup_bounded 15s podman volume inspect --format '{{index .Labels "io.boxferry.docker-core-run"}}' "$storage_volume") && [[ $observed == "$run_id" ]]; then
        cleanup_bounded 60s podman volume rm "$storage_volume" > /dev/null || failure=1
        state=0
        cleanup_bounded 15s podman volume exists "$storage_volume" > /dev/null 2>&1 || state=$?
        ((state == 1)) || failure=1
        if ((state == 1)); then volume_residual=none; fi
      else
        printf 'outer storage ownership unverified; refusing removal\n' >&2
        failure=1
      fi
    elif ((state != 1)); then
      failure=1
    else
      volume_residual=none
    fi
  fi
  if [[ -n $run_dir ]]; then
    # Only the exact mktemp-owned private directory is removed. It contains
    # synthetic request/state and image archive material for this run alone.
    directory_residual=$run_dir
    if [[ $run_dir == /tmp/boxferry-docker-core.* && -d $run_dir && ! -L $run_dir ]]; then
      if [[ $outer_residual == none ]]; then
        cleanup_bounded 30s rm -r -- "$run_dir" || failure=1
        [[ -e $run_dir ]] || directory_residual=none
      else
        # Keep the socket for precise recovery of an outer container, but
        # discard only run-owned temporary archive bytes independently.
        cleanup_bounded 10s rm -f -- "$run_dir/busybox.tar" || failure=1
      fi
    else
      failure=1
    fi
  fi
  if [[ ${guard_ready:-false} == true ]]; then
    guard_alive || failure=1
  fi
  if ((failure != 0)); then
    printf 'cleanup residuals: outer-container=%s storage-volume=%s private-directory=%s\n' \
      "$outer_residual" "$volume_residual" "$directory_residual" >&2
  fi
  return "$failure"
}
on_exit() {
  local status=$? cleanup_status=0
  trap - EXIT HUP INT TERM
  cleanup_owned || cleanup_status=$?
  ((cleanup_status == 0)) || status=1
  [[ ! -e ${run_dir:-/nonexistent}/guard-failure ]] || status=1
  report_host_cache
  if ((status != 0)); then
    ((cleanup_status == 0)) && printf 'cleanup residuals: outer-container=none storage-volume=none private-directory=none\n' >&2
    printf 'Docker core harness failed; no migration acceptance recorded\n' >&2
  fi
  exit "$status"
}
trap on_exit EXIT
trap 'exit 1' HUP INT TERM

run_dir=$(mktemp -d /tmp/boxferry-docker-core.XXXXXXXX)
chmod 0700 "$run_dir"
run_id=${run_dir##*.}
outer="bf-docker-core-${run_id}"
storage_volume="bf-docker-core-data-${run_id}"
socket_dir="$run_dir/socket"
socket_path="$socket_dir/docker.sock"
python3 "$contract" parent-alive --parent-pid "$main_pid" --parent-start "$main_start"
python3 "$contract" deadline-guard --parent-pid "$main_pid" --parent-start "$main_start" \
  --execution-seconds 780 --cleanup-seconds 120 --ready-file "$run_dir/deadline-guard-ready" &
guard_pid=$!
for _attempt in {1..50}; do
  [[ -e $run_dir/deadline-guard-ready ]] && break
  kill -0 "$guard_pid" 2> /dev/null || break
  sleep 0.1
done
[[ -f $run_dir/deadline-guard-ready ]] || {
  printf 'independent deadline guard did not acknowledge startup\n' >&2
  exit 1
}
guard_identity=$(< "$run_dir/deadline-guard-ready")
[[ $guard_identity == "$guard_pid:"* ]] || {
  printf 'independent deadline guard identity differs\n' >&2
  exit 1
}
guard_start=${guard_identity#"$guard_pid:"}
[[ $guard_start =~ ^[0-9]+$ ]] || exit 1
guard_alive || exit 1
guard_ready=true
runtime_deadline=$(cleanup_now)
runtime_deadline=$((runtime_deadline + 750))
if [[ $profile == core-journey ]]; then
  case "$lane" in
    debian11-rootful) docker_preset=debian11-20.10.5-rootful ;;
    debian11-rootless) docker_preset=debian11-20.10.5-rootless ;;
    upstream-rootful) docker_preset=upstream-29.8.1-rootful ;;
    upstream-rootless) docker_preset=upstream-29.8.1-rootless ;;
  esac
  python3 "$contract" render-core-source \
    --template "$script_dir/../fixtures/conformance/docker-application/core.compose.yaml" \
    --destination "$run_dir/core-source.yaml" --image "$core_alias"
  boxferry_snapshot="$run_dir/boxferry-candidate"
  bounded 45s python3 "$contract" capture-candidate --boxferry-root "$boxferry_root" \
    --binary "$boxferry_binary" --receipt "$boxferry_receipt" \
    --docker-lens-root "$lens_root" --docker-lens-revision "$lens_revision" \
    --destination "$boxferry_snapshot" > /dev/null
  (
    ulimit -f 128
    bounded 60s "$boxferry_snapshot" convert compose docker \
      --input-file "$run_dir/core-source.yaml" --project-directory "$run_dir" \
      --docker-target "$docker_preset" --output-directory "$run_dir/docker-output" \
      --console-format json > "$run_dir/plan-report.json" 2> /dev/null
  )
  python3 "$contract" check-core-output --kind plan-report --file "$run_dir/plan-report.json" \
    --lane "$lane" --catalogue-json "$catalogue"
  artifact="$run_dir/docker-output/docker-plan.json"
  python3 "$contract" check-core-output --kind plan-artifact --file "$artifact"
  artifact_sha256=$(python3 "$contract" artifact-identity --artifact "$artifact")
  python3 "$contract" validate-artifact --artifact "$artifact" --image "$core_alias" \
    --artifact-sha256 "$artifact_sha256" \
    --api-version "$api_version" --lane "$lane" --catalogue-json "$catalogue" > /dev/null
fi
[[ $(bounded 15s podman info --format '{{.Host.Security.Rootless}}') == false ]] || {
  printf 'outer Podman is not rootful\n' >&2
  exit 1
}
mkdir -m 0777 "$socket_dir"
container_state=0
bounded 15s podman container exists "$outer" || container_state=$?
((container_state == 1)) || {
  printf 'outer container name is not demonstrably free\n' >&2
  exit 1
}
volume_state=0
bounded 15s podman volume exists "$storage_volume" || volume_state=$?
((volume_state == 1)) || {
  printf 'outer storage name is not demonstrably free\n' >&2
  exit 1
}
registered=true
graph_root=$(bounded 15s podman info --format '{{.Store.GraphRoot}}')
[[ $graph_root == /* && $graph_root != / ]] || {
  printf 'outer Podman graph root is invalid\n' >&2
  exit 1
}
baseline_free=$(bounded 10s df -Pk "$graph_root" | awk 'END {print $4}')
if [[ ! $baseline_free =~ ^[0-9]+$ ]] || ((baseline_free < 8 * 1024 * 1024)); then
  printf 'core probe requires 8 GiB free storage\n' >&2
  exit 1
fi
available_memory_kib=$(awk '/^MemAvailable:/ {print $2}' /proc/meminfo)
if [[ ! $available_memory_kib =~ ^[0-9]+$ ]] || ((available_memory_kib < 6 * 1024 * 1024)); then
  printf 'core probe requires 6 GiB available memory\n' >&2
  exit 1
fi
bounded 30s podman volume create --label "io.boxferry.docker-core-run=$run_id" "$storage_volume" > /dev/null
volume_path=$(bounded 15s podman volume inspect --format '{{.Mountpoint}}' "$storage_volume")
[[ $volume_path == /* && $volume_path != / && -d $volume_path && ! -L $volume_path ]] || {
  printf 'run-owned outer volume mountpoint is invalid\n' >&2
  exit 1
}

watchdog_abort() {
  local state
  : > "$run_dir/watchdog-failure" || true
  while [[ ! -e $run_dir/stop-watchdog ]]; do
    state=0
    signal_main || state=$?
    if ((state == 0 || state == 3)); then return 1; fi
    sleep 1
  done
  return 1
}
watchdog_parent_alive() {
  local state=0
  python3 "$contract" parent-alive --parent-pid "$main_pid" --parent-start "$main_start" \
    > /dev/null 2>&1 || state=$?
  if ((state == 3)); then return 3; fi
  if ((state != 0)); then
    printf 'Docker core parent monitor failed; probe cannot pass\n' >&2
    : > "$run_dir/watchdog-failure" || true
  fi
  return 0
}
watchdog() {
  local used temporary_used free temporary_free minimum_free=$baseline_free growth sample
  while :; do
    [[ ! -e $run_dir/stop-watchdog ]] || return 0
    watchdog_parent_alive || return 0
    sleep 5
    [[ ! -e $run_dir/stop-watchdog ]] || return 0
    watchdog_parent_alive || return 0
    sample=$(bounded 50s python3 "$contract" sample-owned-storage --volume "$volume_path" \
      --run-dir "$run_dir" --graph-root "$graph_root" --baseline-free "$baseline_free") || {
      watchdog_abort
      return 1
    }
    used=$(jq -er '.used' <<< "$sample") || {
      watchdog_abort
      return 1
    }
    temporary_used=$(jq -er '.temporary_used' <<< "$sample") || {
      watchdog_abort
      return 1
    }
    free=$(jq -er '.minimum_free' <<< "$sample") || {
      watchdog_abort
      return 1
    }
    temporary_free=$(jq -er '.temporary_free' <<< "$sample") || {
      watchdog_abort
      return 1
    }
    if [[ ! $used =~ ^[0-9]+$ || ! $temporary_used =~ ^[0-9]+$ ||
      ! $free =~ ^[0-9]+$ || ! $temporary_free =~ ^[0-9]+$ ]]; then
      printf 'Docker core storage monitor returned a nonnumeric value\n' >&2
      watchdog_abort
      return 1
    fi
    ((free < minimum_free)) && minimum_free=$free
    printf '%s\n' "$minimum_free" > "$run_dir/minimum-free-kib" || {
      watchdog_abort
      return 1
    }
    growth=$((baseline_free - minimum_free))
    if ((used > 4 * 1024 * 1024 || temporary_used > 512 * 1024 || growth > 6 * 1024 * 1024 || free < 2 * 1024 * 1024 || temporary_free < 1024 * 1024)); then
      printf 'Docker core storage, disk growth, or free-space budget exceeded\n' >&2
      watchdog_abort
      return 1
    fi
  done
}
watchdog &
watchdog_pid=$!

# No host Docker socket or checkout is mounted. The two exact digest pulls are
# host-side prerequisites; the nested Engine is never asked to pull an image.
outer_pull_attempted=true
bounded 180s podman pull "$outer_image" > /dev/null
fixture_pull_attempted=true
bounded 180s podman pull "$fixture_image" > /dev/null
host_image_id=$(bounded 15s podman image inspect --format '{{.Id}}' "$fixture_image")
host_image_id=${host_image_id#sha256:}
[[ $host_image_id =~ ^[0-9a-f]{64}$ ]] || {
  printf 'canonical fixture image ID unavailable\n' >&2
  exit 1
}
[[ $(bounded 15s podman image inspect --format '{{.Os}}/{{.Architecture}}' "$fixture_image") == linux/amd64 ]] || {
  printf 'host fixture image platform has no reviewed environment baseline\n' >&2
  exit 1
}
archive="$run_dir/busybox.tar"
bounded 120s podman save --format docker-archive --output "$archive" "$fixture_image"
storage_target=/var/lib/docker
[[ $lane == *-rootless ]] && storage_target=/home/docker/.local/share/docker
storage_mount="$storage_volume:$storage_target:U"
nesting_flags=()
if [[ $lane == debian11-rootless ]]; then
  storage_mount+=,suid,dev
  # These reviewed nesting exceptions apply only to the historical native lane.
  nesting_flags+=(--oom-score-adj=0 --security-opt apparmor=unconfined)
fi
bounded 120s podman run --pull=never --detach --name "$outer" \
  --label "io.boxferry.docker-core-run=$run_id" \
  --privileged --pids-limit=512 --memory=4g --cpus=2 --image-volume=ignore \
  "${nesting_flags[@]}" --volume "$storage_mount" --volume "$socket_dir:/boxferry-core" \
  "$outer_image" /usr/local/bin/start-dockerd --host=unix:///boxferry-core/docker.sock > /dev/null
[[ $(bounded 15s podman inspect --format '{{.HostConfig.Privileged}}' "$outer") == true ]] || {
  printf 'reviewed outer nesting privilege was not applied\n' >&2
  exit 1
}
bounded 15s podman inspect --format '{{json .Mounts}}' "$outer" |
  python3 "$contract" verify-outer-mounts --volume "$storage_volume" \
    --destination "$storage_target" --socket-dir "$socket_dir" > /dev/null || {
  printf 'outer Docker daemon has an unexpected mount\n' >&2
  exit 1
}
if [[ $lane == debian11-rootless ]]; then
  # Mountinfo is data from the isolated container; never execute checkout code as root.
  bounded 15s podman exec "$outer" cat /proc/self/mountinfo 2> /dev/null |
    python3 "$contract" verify-storage-mount --destination "$storage_target" > /dev/null || {
    printf 'historical rootless data-root flags are not verified\n' >&2
    exit 1
  }
fi
deadline=$((SECONDS + 180))
until [[ -S $socket_path ]] && curl --fail --silent --max-time 5 --max-filesize 65536 --unix-socket "$socket_path" http://localhost/_ping > /dev/null; do
  ((SECONDS < deadline)) || {
    printf 'nested Docker daemon did not become ready\n' >&2
    exit 1
  }
  [[ $(bounded 15s podman inspect --format '{{.State.Running}}' "$outer") == true ]] || {
    printf 'outer Docker daemon exited before readiness\n' >&2
    exit 1
  }
  sleep 2
done
chmod 0666 "$socket_path"
version_json=$(curl --fail --silent --max-time 10 --max-filesize 65536 --unix-socket "$socket_path" http://localhost/version)
info_json=$(curl --fail --silent --max-time 10 --max-filesize 65536 --unix-socket "$socket_path" http://localhost/info)
observed_release=$(jq -er '.Version' <<< "$version_json")
observed_api=$(jq -er '.ApiVersion' <<< "$version_json")
observed_rootless=$(jq -r '(.Rootless == true) or any(.SecurityOptions[]?; . == "name=rootless" or startswith("name=rootless,"))' <<< "$info_json")
observed_docker_root=$(jq -er '.DockerRootDir' <<< "$info_json")
python3 "$contract" verify-docker-root --lane "$lane" --observed "$observed_docker_root" > /dev/null
# These variables expand only in the isolated guest shell, never on the host.
# shellcheck disable=SC2016
uid_report=$(bounded 15s podman exec "$outer" sh -ec '
  count=0; effective=
  for status in /proc/[0-9]*/status; do
    [ -f "$status" ] || continue
    IFS= read -r comm < "${status%/status}/comm" || continue
    [ "$comm" = dockerd ] || continue
    count=$((count + 1))
    while read -r key real uid saved filesystem; do
      if [ "$key" = Uid: ]; then effective=$uid; break; fi
    done < "$status"
  done
  printf "%s:%s\n" "$count" "$effective"
' 2> /dev/null)
python3 "$contract" verify-mode --lane "$lane" --rootless-marker "$observed_rootless" \
  --uid-report "$uid_report" > /dev/null
installed_package=''
if [[ $lane == debian11-* ]]; then
  # dpkg-query expands its literal package-version template, not either shell.
  # shellcheck disable=SC2016
  installed_package=$(bounded 15s podman exec "$outer" dpkg-query -W '-f=${Version}' docker.io 2> /dev/null)
fi
python3 "$contract" validate-artifact --artifact "$artifact" --image "$core_alias" \
  --artifact-sha256 "$artifact_sha256" \
  --api-version "$api_version" --lane "$lane" \
  --observed-release "$observed_release" --observed-api "$observed_api" \
  --observed-package "$installed_package" --catalogue-json "$catalogue" > /dev/null
bounded 45s podman cp "$archive" "$outer:/tmp/bf-core-busybox.tar"
bounded 45s podman exec "$outer" docker -H unix:///boxferry-core/docker.sock \
  load --input /tmp/bf-core-busybox.tar > /dev/null
inner_image_id=$(bounded 15s podman exec "$outer" docker -H unix:///boxferry-core/docker.sock \
  image inspect --format '{{.Id}}' "$fixture_tag")
inner_image_id=${inner_image_id#sha256:}
[[ $inner_image_id == "$host_image_id" ]] || {
  printf 'loaded inner image ID differs from canonical pinned image\n' >&2
  exit 1
}
[[ $(bounded 15s podman exec "$outer" docker -H unix:///boxferry-core/docker.sock \
  image inspect --format '{{.Os}}/{{.Architecture}}' "$fixture_tag") == linux/amd64 ]] || {
  printf 'inner fixture image platform has no reviewed environment baseline\n' >&2
  exit 1
}
bounded 15s podman exec "$outer" docker -H unix:///boxferry-core/docker.sock tag "$fixture_tag" "$core_alias"
alias_image_id=$(bounded 15s podman exec "$outer" docker -H unix:///boxferry-core/docker.sock \
  image inspect --format '{{.Id}}' "$core_alias")
alias_image_id=${alias_image_id#sha256:}
[[ $alias_image_id == "$host_image_id" ]] || {
  printf 'isolated image alias does not resolve to the reviewed image ID\n' >&2
  exit 1
}

bounded 45s python3 "$contract" replay-test-only --allow-isolated-apply \
  --artifact-sha256 "$artifact_sha256" \
  --artifact "$artifact" --image "$core_alias" --api-version "$api_version" --lane "$lane" \
  --observed-release "$observed_release" --observed-api "$observed_api" \
  --observed-package "$installed_package" --catalogue-json "$catalogue" \
  --socket "$socket_path" --state "$run_dir/state.json" > /dev/null
if [[ $profile == core-journey ]]; then
  container_id=$(python3 "$contract" check-core-output --kind state-id \
    --file "$run_dir/state.json" --image "$core_alias")
  (
    ulimit -f 128
    bounded 30s podman exec "$outer" docker -H unix:///boxferry-core/docker.sock \
      inspect --format '{{json .}}' "$container_id" > "$run_dir/native-inspect.json" 2> /dev/null
  )
  network_id=$(python3 "$contract" check-core-output --kind inspect \
    --file "$run_dir/native-inspect.json" --container-id "$container_id" \
    --image "$core_alias" --fixture-image "$fixture_image")
  (
    ulimit -f 128
    bounded 30s podman exec "$outer" docker -H unix:///boxferry-core/docker.sock \
      network inspect --format '{{json .}}' "$network_id" > "$run_dir/native-network-inspect.json" 2> /dev/null
  )
  python3 "$contract" check-core-output --kind network-inspect \
    --file "$run_dir/native-network-inspect.json" \
    --container-file "$run_dir/native-inspect.json" \
    --container-id "$container_id" --network-id "$network_id" \
    --image "$core_alias" --fixture-image "$fixture_image"
  bounded 30s python3 "$contract" verify-candidate --boxferry-root "$boxferry_root" \
    --binary "$boxferry_binary" --receipt "$boxferry_receipt" \
    --docker-lens-root "$lens_root" --docker-lens-revision "$lens_revision" > /dev/null
  (
    ulimit -f 128
    bounded 60s "$boxferry_snapshot" convert docker compose \
      --docker-socket "$socket_path" --docker-container-id "$container_id" \
      --application-name bf-docker-core \
      --docker-import-policy portable \
      --promote-docker-protected-environment-values \
      --environment-values include \
      --docker-resource-decision "network:${network_id}=external:bridge" \
      --loss-policy partial --output-directory "$run_dir/compose-output" \
      --console-format json > "$run_dir/compose-report.json" 2> "$run_dir/compose-console.txt"
  )
  python3 "$contract" check-core-output --kind compose-console --file "$run_dir/compose-console.txt" \
    --container-file "$run_dir/native-inspect.json" \
    --network-file "$run_dir/native-network-inspect.json" \
    --container-id "$container_id" --network-id "$network_id" \
    --image "$core_alias" --fixture-image "$fixture_image" --socket-path "$socket_path"
  python3 "$contract" check-core-output --kind compose-report --file "$run_dir/compose-report.json" \
    --lane "$lane" --catalogue-json "$catalogue" \
    --container-file "$run_dir/native-inspect.json" \
    --network-file "$run_dir/native-network-inspect.json" \
    --container-id "$container_id" --network-id "$network_id" \
    --image "$core_alias" --fixture-image "$fixture_image" --socket-path "$socket_path"
  python3 "$contract" check-core-output --kind compose \
    --file "$run_dir/compose-output/compose.yaml" --image "$core_alias" \
    --fixture-image "$fixture_image" --container-id "$container_id" \
    --container-file "$run_dir/native-inspect.json" \
    --network-id "$network_id" --network-file "$run_dir/native-network-inspect.json"
fi
marker=$(bounded 30s podman exec "$outer" docker -H unix:///boxferry-core/docker.sock \
  exec bf-docker-core cat /tmp/boxferry-core-marker)
[[ $marker == boxferry-core-ready ]] || {
  printf 'core marker behavior differs\n' >&2
  exit 1
}
minimum_free=$(cat "$run_dir/minimum-free-kib" 2> /dev/null || printf '%s' "$baseline_free")
[[ $minimum_free =~ ^[0-9]+$ ]] || {
  printf 'Docker core storage monitor report is invalid\n' >&2
  exit 1
}
disk_growth_kib=$((baseline_free - minimum_free))
((disk_growth_kib >= 0)) || disk_growth_kib=0
guard_alive || exit 1
cleanup_interrupted=false
trap 'cleanup_interrupted=true' HUP INT TERM
cleanup_owned || {
  trap - EXIT HUP INT TERM
  report_host_cache
  exit 1
}
if [[ $cleanup_interrupted == true ]]; then
  trap - EXIT HUP INT TERM
  report_host_cache
  printf 'cleanup residuals: outer-container=none storage-volume=none private-directory=none\n' >&2
  printf 'Docker core harness interrupted during cleanup; no migration acceptance recorded\n' >&2
  exit 1
fi
trap - EXIT HUP INT TERM
report_host_cache
if [[ $profile == core-journey ]]; then
  printf 'CORE-JOURNEY-SCAFFOLD CHECKS-PASSED: lane=%s docker-release=%s advertised-api=%s docker-lens-revision=%s candidate-receipt=operator-attested loaded-image-id=%s peak-disk-growth-kib=%s cleanup=verified acceptance=pending-reviewed-native-evidence; one core service only, no six-application acceptance\n' \
    "$lane" "$observed_release" "$observed_api" "$lens_revision" \
    "$inner_image_id" "$disk_growth_kib"
  exit 0
fi
printf 'TRANSPORT-ONLY PASS: lane=%s docker-release=%s advertised-api=%s rootless=%s docker-lens-revision=%s native-script-sha256=%s fixture=%s loaded-image-id=%s alias=%s peak-disk-growth-kib=%s cleanup=verified\n' \
  "$lane" "$observed_release" "$observed_api" "$observed_rootless" \
  "$lens_revision" "$script_sha" "$fixture_image" "$inner_image_id" "$core_alias" "$disk_growth_kib"
