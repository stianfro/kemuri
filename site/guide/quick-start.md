# Quick start

This procedure starts one HTTP check and opens the web UI.

## 1. Create a configuration file

Create `kemuri.yaml`:

```yaml
version: 1

server:
  bind: 127.0.0.1
  port: 8080

storage:
  path: ./kemuri.db

profiles:
  - kind: http
    id: local-http
    url: http://127.0.0.1:8080/healthz
    interval: 30s
    timeout: 5s
    expected_status: 200

targets:
  - id: local-kemuri
    name: Local Kemuri
    address: 127.0.0.1
    checks:
      - id: health
        profile: local-http
```

## 2. Validate the configuration

```sh
kemuri config validate --config ./kemuri.yaml
```

Kemuri rejects unknown fields and invalid values. Fix each reported error
before you start the server.

## 3. Check local dependencies

```sh
kemuri doctor --config ./kemuri.yaml
```

Add `--test-notifiers` if the configuration contains notifiers:

```sh
kemuri doctor --config ./kemuri.yaml --test-notifiers
```

## 4. Start the server

```sh
kemuri serve --config ./kemuri.yaml
```

The default startup mode runs each enabled check immediately. Later rounds use
interval-aligned time slots.

## 5. Open the web UI

Open [http://127.0.0.1:8080](http://127.0.0.1:8080).

The readiness endpoint returns `200 OK` when required dependencies are ready:

```sh
curl --fail http://127.0.0.1:8080/readyz
```

## 6. Run one check from the command line

```sh
kemuri check local-kemuri/health --config ./kemuri.yaml
```

The command uses the same resolved probe settings as the server. It returns a
nonzero exit code when the result is not healthy.

## Add public DNS checks

The following configuration adds checks for three public DNS services:

```yaml
profiles:
  - kind: dns
    id: public-dns
    name: example.com
    record_type: A
    protocol: udp
    expected_rcode: noerror
    require_answer: true
    interval: 30s
    timeout: 5s

targets:
  - id: cloudflare-dns
    name: Cloudflare DNS
    address: 1.1.1.1
    group_path: public-dns
    checks:
      - id: resolve
        profile: public-dns
        server: 1.1.1.1:53

  - id: google-dns
    name: Google Public DNS
    address: 8.8.8.8
    group_path: public-dns
    checks:
      - id: resolve
        profile: public-dns
        server: 8.8.8.8:53

  - id: quad9-dns
    name: Quad9 DNS
    address: 9.9.9.9
    group_path: public-dns
    checks:
      - id: resolve
        profile: public-dns
        server: 9.9.9.9:53
```

Merge these entries with the existing `profiles` and `targets` lists. Do not
create a second key with the same name.
