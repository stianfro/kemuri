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
  "$root/packaging/kemuri.yaml" >"$tmp/kemuri.yaml"
cp "$tmp/kemuri.yaml" "$tmp/valid.yaml"

cargo build -p kemuri-cli --bin kemuri
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
test "$(curl --silent --output "$tmp/api-error" --write-out '%{http_code}' "http://127.0.0.1:$port/api/v1/missing")" = 404
jq -e '.request_id' "$tmp/api-error" >/dev/null
curl --fail --silent "http://127.0.0.1:$port/api/openapi.json" | jq -e '.openapi == "3.1.0"' >/dev/null

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

lsof -ti :$port -sTCP:LISTEN | xargs kill
wait "$server_pid"
"$binary" doctor --config "$tmp/kemuri.yaml"
"$binary" database backup --config "$tmp/kemuri.yaml" --output - >"$tmp/backup.sqlite"
test "$(sqlite3 "$tmp/backup.sqlite" 'PRAGMA integrity_check')" = ok
