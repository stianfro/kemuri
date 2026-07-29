# Backups and retention

Back up the SQLite database while Kemuri runs. Do not copy an active database
file with a normal file-copy command.

## Create a backup

Write a backup to a file:

```sh
kemuri database backup \
  --config /etc/kemuri/kemuri.yaml \
  --output /srv/backups/kemuri.db
```

Write a complete SQLite image to standard output:

```sh
kemuri database backup \
  --config /etc/kemuri/kemuri.yaml \
  --output - > /srv/backups/kemuri.db
```

Store backups outside the active data directory.

## Verify a backup

```sh
sqlite3 /srv/backups/kemuri.db 'PRAGMA integrity_check;'
```

The expected result is `ok`.

Test restore procedures at fixed intervals. Keep the test separate from the
active service.

## Retention

Configure retention under `storage.retention`:

```yaml
storage:
  retention:
    raw_rounds: 7d
    rollup_5m: 90d
    rollup_1h: forever
    alert_events: 30d
    notification_records: 30d
```

A value is a positive duration or `forever`.

Kemuri deletes data in bounded batches. It deletes a raw round only when the
exact matching rollup bucket exists.

Notification records are deleted before their referenced alert events.

## Disk pressure

Configure two free-space limits:

```yaml
storage:
  disk_pressure:
    warning_free: "10%"
    critical_free: "5%"
```

Kemuri pauses scheduling at or below the critical limit. It resumes scheduling
only above the warning limit.

This hysteresis prevents repeated pause and resume changes near one value.

During a disk-pressure pause, Kemuri keeps these functions active:

- web UI
- health and readiness
- metrics
- retention
- configuration reload

Readiness reports the disk state.
