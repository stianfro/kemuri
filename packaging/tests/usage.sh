#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tmp="$(mktemp -d)"
port=18090
cleanup() {
  lsof -ti :$port -sTCP:LISTEN | xargs -r kill || true
  rm -rf "$tmp"
}
trap cleanup EXIT

sed \
  -e "s#port: 8080#port: $port\\n  shutdown_timeout: 3s#" \
  -e "s#127.0.0.1:8080#127.0.0.1:$port#" \
  -e "s#/var/lib/kemuri/kemuri.db#$tmp/kemuri.db#" \
  -e "s#warning_free: 10%#warning_free: 0.5%#" \
  -e "s#critical_free: 5%#critical_free: 0.1%#" \
  "$root/packaging/kemuri.yaml" >"$tmp/kemuri.yaml"
cp "$tmp/kemuri.yaml" "$tmp/valid.yaml"

cargo build -p kemuri --bin kemuri
binary="$root/target/debug/kemuri"
"$binary" config validate --config "$tmp/kemuri.yaml"
"$binary" version

"$binary" serve --config "$tmp/kemuri.yaml" >"$tmp/server.log" 2>&1 &
server_pid=$!
for _ in $(seq 1 100); do
  if curl --fail --silent "http://127.0.0.1:$port/readyz" >/dev/null; then break; fi
  sleep 0.1
done
curl --fail --silent "http://127.0.0.1:$port/readyz" >/dev/null
"$binary" check --config "$tmp/kemuri.yaml" kemuri/health

index="$(curl --fail --silent "http://127.0.0.1:$port/")"
asset="$(grep -o '/assets/[^\" ]*\.js' <<<"$index")"
curl --fail --silent "http://127.0.0.1:$port$asset" >/dev/null
test "$(curl --silent --output /dev/null --write-out '%{http_code}' "http://127.0.0.1:$port/assets/missing.js")" = 404
test "$(curl --silent --dump-header "$tmp/api-headers" --output "$tmp/api-error" --write-out '%{http_code}' "http://127.0.0.1:$port/api/v1/missing")" = 404
request_id="$(jq -r '.request_id' "$tmp/api-error")"
test -n "$request_id"
test "$(awk 'tolower($1) == "x-request-id:" { gsub("\r", "", $2); print $2 }' "$tmp/api-headers")" = "$request_id"
curl --fail --silent "http://127.0.0.1:$port/api/openapi.json" | jq -e '.openapi == "3.1.0"' >/dev/null
curl --fail --silent "http://127.0.0.1:$port/api/v1/targets?limit=1" | jq -e '.targets | length <= 1' >/dev/null
test "$(curl --silent --output "$tmp/limit-error" --write-out '%{http_code}' "http://127.0.0.1:$port/api/v1/targets?limit=0")" = 400
jq -e '.code == "bad_request" and .request_id' "$tmp/limit-error" >/dev/null
curl --fail --silent "http://127.0.0.1:$port/api/v1/alerts?limit=1" | jq -e '.alerts | type == "array"' >/dev/null
curl --fail --silent "http://127.0.0.1:$port/api/v1/alert-events?limit=1" | jq -e '.events | type == "array"' >/dev/null
curl --silent --dump-header "$tmp/cors-headers" --output /dev/null \
  -H 'Origin: https://example.invalid' "http://127.0.0.1:$port/api/v1/targets"
! grep -qi '^access-control-allow-origin:' "$tmp/cors-headers"
test "$(curl --silent --output "$tmp/cross-origin" --write-out '%{http_code}' -X POST \
  -H 'Content-Type: application/json' -H 'Origin: https://example.invalid' -d '{}' \
  "http://127.0.0.1:$port/api/v1/config/reload")" = 400

for _ in 1 2; do
  curl --fail --silent -X POST -H 'Content-Type: application/json' -d '{}' \
    "http://127.0.0.1:$port/api/v1/config/reload" >/dev/null
  sleep 0.2
done
printf 'version: 2\n' >"$tmp/kemuri.yaml"
curl --fail --silent -X POST -H 'Content-Type: application/json' -d '{}' \
  "http://127.0.0.1:$port/api/v1/config/reload" >/dev/null
sleep 0.2
curl --fail --silent "http://127.0.0.1:$port/readyz" >/dev/null
cp "$tmp/valid.yaml" "$tmp/kemuri.yaml"

curl --no-buffer --silent "http://127.0.0.1:$port/api/v1/events" >"$tmp/events" &
events_pid=$!
sleep 0.2
shutdown_started=$SECONDS
lsof -ti :$port -sTCP:LISTEN | xargs kill
wait "$server_pid"
wait "$events_pid"
test "$((SECONDS - shutdown_started))" -le 5
"$binary" doctor --config "$tmp/kemuri.yaml"
"$binary" database backup --config "$tmp/kemuri.yaml" --output - >"$tmp/backup.sqlite"
test "$(sqlite3 "$tmp/backup.sqlite" 'PRAGMA integrity_check')" = ok
