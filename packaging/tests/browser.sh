#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tmp="$(mktemp -d)"
port=18091
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

cargo build -p kemuri --bin kemuri
"$root/target/debug/kemuri" serve --config "$tmp/kemuri.yaml" >"$tmp/server.log" 2>&1 &
for _ in $(seq 1 100); do
  if curl --fail --silent "http://127.0.0.1:$port/readyz" >/dev/null; then
    break
  fi
  sleep 0.1
done
curl --fail --silent "http://127.0.0.1:$port/readyz" >/dev/null

cd "$root/web"
KEMURI_URL="http://127.0.0.1:$port" npm run test:e2e
