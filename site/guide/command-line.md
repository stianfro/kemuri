# Command line

Kemuri has commands for service operation, validation, checks, notifications,
and database backups.

## Start the server

```sh
kemuri serve --config PATH
```

The process stays in the foreground. Use systemd or another process manager for
a long-running installation.

## Print version information

```sh
kemuri version
```

This command prints the package version, Git revision, build target, and build
profile.

## Validate configuration

```sh
kemuri config validate --config PATH
```

The command resolves profiles, checks, secrets, and certificate files. It does
not start the scheduler.

## Inspect dependencies

```sh
kemuri doctor --config PATH
```

Use `--test-notifiers` to test all configured notifiers:

```sh
kemuri doctor --config PATH --test-notifiers
```

This option can send real messages.

## Run one check

```sh
kemuri check TARGET_ID/CHECK_ID --config PATH
```

The command prints the check result. It returns a nonzero exit code for an
unhealthy result or an execution error.

## Test one notifier

```sh
kemuri notify test NOTIFIER_ID --config PATH
```

This command sends a real test notification.

## Back up the database

Write the backup to a file:

```sh
kemuri database backup --config PATH --output kemuri-backup.db
```

Write a complete SQLite image to standard output:

```sh
kemuri database backup --config PATH --output - > kemuri-backup.db
```

See [Backups and retention](../operations/backups) for verification steps.
