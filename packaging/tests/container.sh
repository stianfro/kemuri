#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
image=kemuri:test
name="kemuri-test-$$"
volume="kemuri-test-$$"
tmp="$(mktemp -d)"
port=18092
cleanup() {
  docker rm -f "$name" >/dev/null 2>&1 || true
  docker volume rm "$volume" >/dev/null 2>&1 || true
  rm -rf "$tmp"
}
trap cleanup EXIT

docker build -f "$root/packaging/container/Dockerfile" -t "$image" "$root"
test "$(docker image inspect "$image" --format '{{.Config.User}}')" = kemuri
docker volume create "$volume" >/dev/null

cat >"$tmp/kemuri.yaml" <<'YAML'
version: 1
server:
  bind: 0.0.0.0
  port: 8080
  shutdown_timeout: 3s
storage:
  path: /var/lib/kemuri/kemuri.db
  disk_pressure:
    warning_free: 0.5%
    critical_free: 0.1%
scheduler:
  startup_mode: immediate_then_aligned
  default_jitter: 0%
  max_concurrent: 4
profiles:
  - kind: http
    id: self
    url: http://127.0.0.1:8080/healthz
    interval: 30s
    timeout: 2s
targets:
  - id: kemuri
    address: 127.0.0.1
    checks:
      - id: health
        profile: self
YAML

docker run --detach --name "$name" \
  --publish "127.0.0.1:$port:8080" \
  --mount "type=bind,src=$tmp/kemuri.yaml,dst=/etc/kemuri/kemuri.yaml,readonly" \
  --mount "type=volume,src=$volume,dst=/var/lib/kemuri" \
  "$image" >/dev/null
for _ in $(seq 1 100); do
  if curl --fail --silent "http://127.0.0.1:$port/readyz" >/dev/null; then break; fi
  sleep 0.2
done
curl --fail --silent "http://127.0.0.1:$port/readyz" >/dev/null
test "$(docker exec "$name" id -u)" != 0
docker exec "$name" test -s /var/lib/kemuri/kemuri.db
for _ in $(seq 1 30); do
  health="$(docker inspect "$name" --format '{{if .State.Health}}{{.State.Health.Status}}{{end}}')"
  test "$health" = healthy && break
  sleep 1
done
test "$(docker inspect "$name" --format '{{.State.Health.Status}}')" = healthy
docker stop --time 5 "$name" >/dev/null
docker rm "$name" >/dev/null

cat >"$tmp/kemuri.yaml" <<'YAML'
version: 1
server:
  bind: 0.0.0.0
  port: 8080
storage:
  path: /var/lib/kemuri/kemuri.db
  disk_pressure:
    warning_free: 0.5%
    critical_free: 0.1%
scheduler:
  startup_mode: immediate_then_aligned
  default_jitter: 0%
profiles:
  - kind: icmp
    id: ping
    interval: 30s
    timeout: 2s
    count: 1
targets:
  - id: ipv4-loopback
    address: 127.0.0.1
    checks:
      - id: ping
        profile: ping
  - id: ipv6-loopback
    address: ::1
    checks:
      - id: ping
        profile: ping
        address_family: ipv6
YAML

docker run --detach --name "$name" --cap-add NET_RAW \
  --publish "127.0.0.1:$port:8080" \
  --mount "type=bind,src=$tmp/kemuri.yaml,dst=/etc/kemuri/kemuri.yaml,readonly" \
  --mount "type=volume,src=$volume,dst=/var/lib/kemuri" \
  "$image" >/dev/null
for _ in $(seq 1 100); do
  if curl --fail --silent "http://127.0.0.1:$port/readyz" >/dev/null; then break; fi
  sleep 0.2
done
curl --fail --silent "http://127.0.0.1:$port/readyz" >/dev/null
sleep 2
curl --fail --silent "http://127.0.0.1:$port/api/v1/targets/ipv4-loopback/checks/ping/rounds?limit=1" \
  | jq -e '.rounds | length == 1' >/dev/null
if docker exec "$name" test -r /proc/net/if_inet6; then
  curl --fail --silent "http://127.0.0.1:$port/api/v1/targets/ipv6-loopback/checks/ping/rounds?limit=1" \
    | jq -e '.rounds | length == 1' >/dev/null
fi
