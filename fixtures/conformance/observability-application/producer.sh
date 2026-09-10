#!/bin/sh
set -eu

mode=${1:?producer mode required}

case "${mode}" in
  metrics)
    root=${2:-/www}
    mkdir -p "${root}"
    cat > "${root}/metrics" << 'EOF'
# HELP boxferry_fixture_temperature_celsius Controlled acceptance fixture value.
# TYPE boxferry_fixture_temperature_celsius gauge
boxferry_fixture_temperature_celsius{source="controlled"} 42
EOF
    cat > /tmp/boxferry-nginx.conf << EOF
events {}
http {
  access_log off;
  server {
    listen 8080;
    root ${root};
  }
}
EOF
    exec nginx -c /tmp/boxferry-nginx.conf -g 'daemon off;'
    ;;
  logs)
    destination=${2:-/var/log/boxferry/telemetry.log}
    mkdir -p "${destination%/*}"
    if [ ! -s "${destination}" ]; then
      printf '%s\n' \
        'sequence=0001 level=info message=boxferry-observability-known-log source=controlled' \
        > "${destination}"
    fi
    exec sleep 86400
    ;;
  self-test)
    temporary=${2:?self-test directory required}
    mkdir -p "${temporary}"
    destination="${temporary}/telemetry.log"
    "$0" logs-once "${destination}"
    "$0" logs-once "${destination}"
    test "$(wc -l < "${destination}")" -eq 1
    grep -Fqx \
      'sequence=0001 level=info message=boxferry-observability-known-log source=controlled' \
      "${destination}"
    ;;
  logs-once)
    destination=${2:?log destination required}
    mkdir -p "${destination%/*}"
    if [ ! -s "${destination}" ]; then
      printf '%s\n' \
        'sequence=0001 level=info message=boxferry-observability-known-log source=controlled' \
        > "${destination}"
    fi
    ;;
  *)
    printf 'Unknown producer mode: %s\n' "${mode}" >&2
    exit 64
    ;;
esac
