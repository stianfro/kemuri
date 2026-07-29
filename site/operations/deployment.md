# Deployment

Run one Kemuri process for one SQLite database. Put the database on a local
file system.

## Directory layout

A system installation can use these paths:

```text
/usr/local/bin/kemuri
/etc/kemuri/kemuri.yaml
/var/lib/kemuri/kemuri.db
```

Create a service account and data directory:

```sh
sudo useradd --system --home /var/lib/kemuri --shell /usr/sbin/nologin kemuri
sudo install -d -o kemuri -g kemuri -m 0750 /var/lib/kemuri
sudo install -d -o root -g kemuri -m 0750 /etc/kemuri
sudo install -o root -g kemuri -m 0640 kemuri.yaml /etc/kemuri/kemuri.yaml
```

## systemd

Release archives contain `kemuri.service`. Install and start it:

```sh
sudo install -m 0644 kemuri.service /etc/systemd/system/kemuri.service
sudo systemctl daemon-reload
sudo systemctl enable --now kemuri
```

Inspect the service:

```sh
systemctl status kemuri
journalctl -u kemuri
```

The provided unit runs as the `kemuri` user. Update its paths if your
installation uses other locations.

## Bind address

Kemuri binds to `127.0.0.1` by default. Use a private interface address when
other trusted hosts must connect directly.

```yaml
server:
  bind: 100.64.0.10
  port: 8080
```

Kemuri has no built-in login. Do not publish an unprotected listener to the
public internet.

## Reverse proxy

Use a trusted reverse proxy for TLS, access control, or a public host name.

Set the external URL:

```yaml
server:
  bind: 127.0.0.1
  port: 8080
  public_url: https://kemuri.example.com
```

Proxy these paths without a prefix change:

- `/`
- `/api/`
- `/healthz`
- `/readyz`
- `/metrics`, if Prometheus uses the proxy

Disable response buffering for `/api/v1/events`. Keep the connection open for
server-sent events.

## CORS

Keep CORS off when the UI and API use the same origin:

```yaml
server:
  cors: false
```

Set `cors: true` only when another origin must read API data. This setting
permits cross-origin `GET` and `HEAD`. It does not permit cross-origin reload.

## Readiness

Use `/healthz` for a liveness check. Use `/readyz` before you send normal
traffic.

```sh
curl --fail http://127.0.0.1:8080/healthz
curl --fail http://127.0.0.1:8080/readyz
```

## Shutdown

Send `SIGTERM` to stop Kemuri. The process stops new work, drains active
rounds, flushes storage, and exits.

Set a positive shutdown timeout:

```yaml
server:
  shutdown_timeout: 30s
```

Set the process manager stop timeout to a value that is greater than the Kemuri
timeout.
