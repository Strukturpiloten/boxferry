#!/usr/bin/env bash
# Extract a direct user-mode verifier; never run the tool image or register binfmt.
set -euo pipefail
# renovate: datasource=docker depName=docker.io/tonistiigi/binfmt
readonly release_emulator_image='docker.io/tonistiigi/binfmt:qemu-v10.2.3-68@sha256:400a4873b838d1b89194d982c45e5fb3cda4593fbfd7e08a02e76b03b21166f0'

if [[ $# != 3 ]]; then
  echo 'usage: release-emulator.sh docker|podman TARGET NEW_DIRECTORY' >&2
  exit 2
fi
runtime="$1"
case "${runtime}" in
  docker | podman) ;;
  *)
    echo 'An explicit docker or podman backend is required.' >&2
    exit 2
    ;;
esac
case "$2" in
  i686-unknown-linux-musl) executable=qemu-i386 ;;
  armv7-unknown-linux-musleabihf) executable=qemu-arm ;;
  *)
    echo 'No reviewed direct emulator for this target.' >&2
    exit 2
    ;;
esac
output_directory="$3"
mkdir -- "${output_directory}"
container_id=''
creation_attempted=false
recovery_required=false
ownership_token="$(python3 -c 'import secrets; print(secrets.token_hex(32))')"
container_name="boxferry-release-emulator-${ownership_token}"
ownership_label=dev.boxferry.release-emulator
recovery_record="${output_directory}/container-recovery.json"
python3 - "${recovery_record}" "${runtime}" "${container_name}" "${ownership_label}" "${ownership_token}" "${release_emulator_image}" << 'PY'
import json
import sys
with open(sys.argv[1], "x", encoding="utf-8") as stream:
    json.dump(dict(zip(("runtime", "name", "label", "owner", "image"), sys.argv[2:], strict=True)), stream)
    stream.write("\n")
PY

inspect_owned_id() {
  local inspection
  inspection="${output_directory}/container-inspection.json"
  if ! timeout --signal=TERM --kill-after=5s 30s "${runtime}" container inspect "${container_name}" > "${inspection}"; then
    return 1
  fi
  python3 - "${inspection}" "${recovery_record}" "${container_id}" << 'PY'
import json
import pathlib
import re
import sys
inspection = pathlib.Path(sys.argv[1])
if inspection.stat().st_size > 65536:
    raise SystemExit("container inspection exceeds the verification bound")
record = json.loads(pathlib.Path(sys.argv[2]).read_text(encoding="utf-8"))
objects = json.loads(inspection.read_text(encoding="utf-8"))
if not isinstance(objects, list) or len(objects) != 1 or not isinstance(objects[0], dict):
    raise SystemExit("container inspection must contain one object")
item = objects[0]
identifier = item.get("Id")
config = item.get("Config")
state = item.get("State")
if not isinstance(identifier, str) or not re.fullmatch("[a-f0-9]{64}", identifier):
    raise SystemExit("container inspection must contain a full immutable identifier")
if not isinstance(config, dict) or not isinstance(state, dict):
    raise SystemExit("container inspection lacks configuration or state")
if item.get("Name") not in (record["name"], "/" + record["name"]):
    raise SystemExit("container name does not match the recorded extraction task")
tagged_repository, digest = record["image"].rsplit("@", 1)
repository = tagged_repository.rsplit(":", 1)[0]
allowed_references = {record["image"], repository + "@" + digest}
references = []
if "Image" in config:
    references.append(config["Image"])
if "ImageName" in item:
    references.append(item["ImageName"])
if not references or any(not isinstance(reference, str) or reference not in allowed_references for reference in references):
    raise SystemExit("container image references do not match the recorded repository and immutable digest")
if "ImageDigest" in item and item["ImageDigest"] != digest:
    raise SystemExit("container image digest conflicts with the recorded immutable digest")
labels = config.get("Labels")
if not isinstance(labels, dict) or labels.get(record["label"]) != record["owner"]:
    raise SystemExit("container ownership label does not match the recorded extraction task")
if sys.argv[3] and identifier != sys.argv[3]:
    raise SystemExit("container identifier changed after creation")
if state.get("Running") is not False:
    raise SystemExit("extraction container is unexpectedly running or has unknown state")
print(identifier)
PY
}
cleanup() {
  local status=$?
  local owned_id=''
  trap - EXIT INT TERM
  if [[ "${creation_attempted}" == true ]]; then
    if owned_id="$(inspect_owned_id)" && [[ "${owned_id}" =~ ^[a-f0-9]{64}$ ]]; then
      if ! timeout --signal=TERM --kill-after=5s 30s "${runtime}" rm -- "${owned_id}" > /dev/null; then
        echo 'Failed to remove the verified stopped extraction container.' >&2
        recovery_required=true
        status=1
      fi
    else
      echo 'Extraction container identity could not be verified; no container was removed.' >&2
      recovery_required=true
      status=1
    fi
  fi
  rm -f -- "${output_directory}/container-inspection.json"
  if [[ "${recovery_required}" == true ]]; then
    printf 'Recovery record retained: %s\n' "${recovery_record}" >&2
  else
    rm -f -- "${recovery_record}"
  fi
  if [[ ${status} != 0 ]]; then
    rm -f -- "${output_directory}/${executable}"
    if [[ "${recovery_required}" != true ]]; then
      rmdir -- "${output_directory}" || true
    fi
  fi
  exit "${status}"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
creation_attempted=true
if ! container_id="$(timeout --signal=TERM --kill-after=5s 180s "${runtime}" create \
  --name "${container_name}" --label "${ownership_label}=${ownership_token}" \
  --platform linux/amd64 "${release_emulator_image}")"; then
  container_id=''
  echo 'Container creation did not complete; verifying recorded ownership before recovery.' >&2
  exit 1
fi
if [[ ! "${container_id}" =~ ^[a-f0-9]{64}$ ]]; then
  container_id=''
  echo 'Creation did not return a full immutable container identifier.' >&2
  exit 1
fi
verified_id="$(inspect_owned_id)"
[[ "${verified_id}" == "${container_id}" ]] || {
  echo 'Extraction container identity mismatch.' >&2
  exit 1
}
timeout --signal=TERM --kill-after=5s 60s "${runtime}" cp "${container_id}:/usr/bin/${executable}" "${output_directory}/${executable}"
python3 - "${output_directory}/${executable}" << 'PY'
import pathlib
import stat
import struct
import sys
path = pathlib.Path(sys.argv[1])
info = path.lstat()
if not stat.S_ISREG(info.st_mode) or not 64 <= info.st_size <= 134217728:
    raise SystemExit("emulator must be a bounded regular file")
with path.open("rb") as stream:
    header = stream.read(64)
if header[:6] != b"\x7fELF\x02\x01" or struct.unpack_from("<H", header, 18)[0] != 62:
    raise SystemExit("emulator must be an AMD64 ELF executable")
PY
chmod 0755 "${output_directory}/${executable}"
# Derive the expected executable version from the same reviewed image pin.
image_version="${release_emulator_image#*:qemu-v}"
image_version="${image_version%%-*}"
effective_version="$(timeout --signal=TERM --kill-after=5s 10s "${output_directory}/${executable}" --version)"
[[ "${effective_version}" == "qemu-${executable#qemu-} version ${image_version} "* ]] || {
  echo 'Extracted QEMU version does not match its reviewed image pin.' >&2
  exit 1
}
