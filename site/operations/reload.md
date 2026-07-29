# Reload configuration

Kemuri can reload its configuration without an HTTP server restart.

## Validate first

```sh
kemuri config validate --config /etc/kemuri/kemuri.yaml
```

Validation checks resolved profile and check values. It also loads referenced
secret and certificate files.

## Reload with a signal

```sh
sudo systemctl reload kemuri
```

If the systemd unit does not define reload, send `SIGHUP`:

```sh
sudo systemctl kill --signal HUP kemuri
```

## Reload with the API

Send a same-origin JSON request:

```sh
curl --fail \
  -X POST \
  -H 'Content-Type: application/json' \
  --data '{}' \
  http://127.0.0.1:8080/api/v1/config/reload
```

The endpoint rejects cross-origin requests.

## Transaction behavior

Kemuri serializes reload requests. It performs these steps:

1. Read the full file.
2. Parse and validate all values.
3. Resolve profiles, checks, and secrets.
4. Load certificate files.
5. Initialize notifier clients.
6. Reconcile SQLite configuration state.
7. Replace the active runtime state.

If a step fails, Kemuri keeps the prior runtime state. The HTTP server stays
active.

## Check revisions

Kemuri creates a new check revision when effective settings change. New rounds
refer to the new revision. Old rounds keep their prior revision.

The series API and graph include revision markers.

## Disable and enable a check

Set `enabled: false` on a target or check, and then reload. Kemuri stops new
rounds for that entry. It keeps prior history.

Set `enabled: true`, and then reload again. Kemuri resolves the entry and
resumes scheduling.
