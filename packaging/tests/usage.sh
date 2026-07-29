#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
tmp="$(mktemp -d)"
port=18090
cleanup() {
  lsof -ti :$port -sTCP:LISTEN | xargs -r kill || true
  test -n "${fixture_pid:-}" && kill "$fixture_pid" 2>/dev/null || true
  test -n "${tls_pid:-}" && kill "$tls_pid" 2>/dev/null || true
  rm -rf "$tmp"
}
trap cleanup EXIT

: >"$tmp/webhooks"
python3 "$root/packaging/tests/fixtures/services.py" "$tmp/fail" "$tmp/webhooks" &
fixture_pid=$!
openssl req -x509 -newkey rsa:2048 -nodes -days 1 -subj '/CN=localhost' \
  -addext 'basicConstraints=critical,CA:FALSE' -addext 'subjectAltName=DNS:localhost' \
  -keyout "$tmp/key.pem" -out "$tmp/cert.pem" >/dev/null 2>&1
openssl s_server -accept 18101 -cert "$tmp/cert.pem" -key "$tmp/key.pem" -quiet -www \
  >"$tmp/tls.log" 2>&1 &
tls_pid=$!
for _ in $(seq 1 100); do
  if curl --fail --silent http://127.0.0.1:18104/health >/dev/null; then break; fi
  sleep 0.1
done
curl --fail --silent http://127.0.0.1:18104/health >/dev/null

cat >"$tmp/kemuri.yaml" <<YAML
version: 1
server:
  bind: 127.0.0.1
  port: $port
  public_url: http://127.0.0.1:$port
  shutdown_timeout: 3s
storage:
  path: $tmp/kemuri.db
  disk_pressure:
    warning_free: 0.5%
    critical_free: 0.1%
scheduler:
  tick_interval: 100ms
  startup_mode: immediate_then_aligned
  default_jitter: 0%
  max_concurrent: 16
profiles:
  - kind: http
    id: fixture-http
    url: http://127.0.0.1:18104/health
    interval: 1s
    timeout: 2s
    expected_status: 200
  - kind: tcp
    id: fixture-tcp
    host: 127.0.0.1
    port: 18100
    interval: 2s
    timeout: 2s
  - kind: tcp
    id: fixture-tls
    host: 127.0.0.1
    port: 18101
    interval: 2s
    timeout: 2s
    tls:
      enabled: true
      server_name: localhost
      tls_validate: false
  - kind: tcp
    id: fixture-tls-ca
    host: 127.0.0.1
    port: 18101
    interval: 2s
    timeout: 2s
    tls:
      enabled: true
      server_name: localhost
      tls_validate: true
      root_certificates:
        - $tmp/cert.pem
  - kind: dns
    id: fixture-dns-udp
    name: fixture.test
    server: 127.0.0.1:18102
    record_type: A
    protocol: udp
    expected_rcode: noerror
    require_answer: true
    interval: 2s
    timeout: 2s
  - kind: dns
    id: fixture-dns-tcp
    name: fixture.test
    server: 127.0.0.1:18102
    record_type: A
    protocol: tcp
    expected_rcode: noerror
    require_answer: true
    interval: 2s
    timeout: 2s
notifiers:
  - kind: webhook
    id: local-webhook
    url: http://127.0.0.1:18104/webhook
    timeout: 2s
rules:
  - id: fixture-down
    profile: fixture-http
    metric: consecutive_unhealthy_rounds
    operator: gte
    threshold: "1"
    window: 1m
    notifier: local-webhook
    minimum_rounds: 1
targets:
  - id: fixtures
    address: 127.0.0.1
    group_path: local
    checks:
      - id: http
        profile: fixture-http
      - id: tcp
        profile: fixture-tcp
      - id: tls
        profile: fixture-tls
      - id: tls-ca
        profile: fixture-tls-ca
      - id: dns-udp
        profile: fixture-dns-udp
      - id: dns-tcp
        profile: fixture-dns-tcp
YAML
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
"$binary" check --config "$tmp/kemuri.yaml" fixtures/http
"$binary" check --config "$tmp/kemuri.yaml" fixtures/tcp
"$binary" check --config "$tmp/kemuri.yaml" fixtures/tls
"$binary" check --config "$tmp/kemuri.yaml" fixtures/tls-ca
"$binary" check --config "$tmp/kemuri.yaml" fixtures/dns-udp
"$binary" check --config "$tmp/kemuri.yaml" fixtures/dns-tcp

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


touch "$tmp/fail"
for _ in $(seq 1 100); do
  if curl --fail --silent "http://127.0.0.1:$port/api/v1/alerts?rule_id=fixture-down" \
    | jq -e '.alerts[0].state == "firing"' >/dev/null; then break; fi
  sleep 0.2
done
curl --fail --silent "http://127.0.0.1:$port/api/v1/alerts?rule_id=fixture-down" \
  | jq -e '.alerts[0].state == "firing"' >/dev/null
rm "$tmp/fail"
for _ in $(seq 1 100); do
  if curl --fail --silent "http://127.0.0.1:$port/api/v1/alerts?rule_id=fixture-down" \
    | jq -e '.alerts[0].state == "normal"' >/dev/null; then break; fi
  sleep 0.2
done
curl --fail --silent "http://127.0.0.1:$port/api/v1/alerts?rule_id=fixture-down" \
  | jq -e '.alerts[0].state == "normal"' >/dev/null
for _ in $(seq 1 150); do
  test "$(wc -l <"$tmp/webhooks" 2>/dev/null || echo 0)" -ge 2 && break
  sleep 0.2
done
test "$(wc -l <"$tmp/webhooks")" -ge 2
jq -s -e 'map(.event_type) | index("firing") != null and index("resolved") != null' \
  "$tmp/webhooks" >/dev/null

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
