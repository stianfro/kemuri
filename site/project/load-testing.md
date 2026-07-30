# Load testing

Kemuri has local test tools for scheduler load, API load, configuration
changes, historical data, alerts, restarts, and resource limits. The tools do
not send load to public services.

## Safety rules

The default tests have limits for the check count, duration, memory, storage,
API response time, and generated row count. Keep these limits for a shared
development host.

Use an isolated host for a larger test. Set CPU, memory, storage, file
descriptor, packet-rate, and duration limits before the test. Stop the test if
Kemuri or an unrelated service becomes unhealthy.

Do not commit generated databases, logs, or result files.

## Short admission test

Run the complete bounded test set:

```sh
just test-scale
```

This task tests these conditions:

- Each scheduled slot has one stored result.
- No check has a duplicate scheduled slot.
- SQLite integrity and foreign key checks pass.
- API latency stays within the configured limit.
- Cursor pages do not lose or repeat records.
- Concurrent SSE clients receive round events and can reconnect.
- Valid and invalid reload operations do not stop the HTTP server.
- Alert storms deliver fast and slow webhooks.
- Failed webhooks enter the retry state.
- SIGTERM drains active work, and the same database opens after restart.
- Critical disk pressure pauses scheduling.
- Scheduling resumes after disk pressure clears.
- The web UI has bounded DOM and SVG sizes at desktop and mobile widths.

## Focused commands

Use a focused command when you change one test area:

```sh
just load-test-api
just load-test-reload
just load-test-resilience
just load-test-history --months 12 --verify --exercise
```

The resilience test uses process limits of 128 open files, 30 CPU seconds, and
2 GiB of virtual address space. It also changes the disk pressure thresholds
in its temporary configuration. It does not change host limits or fill a
disk.

## Historical data

The history generator creates deterministic databases for 1, 6, or 12
months. It includes completed and partial raw rounds. It also includes
complete and partial 5-minute and 1-hour rollups.

The default 12-month profile has 34,560 raw rounds and 69,120 rollup rows. It
uses approximately 18 MiB in the current test environment. This is a query
and retention fixture. It is not a storage growth estimate for a production
configuration.

The generator checks:

- migration checksums
- SQLite integrity
- SQLite backup integrity
- raw retention only when a matching 5-minute rollup exists
- target and round cursor pagination
- raw and rollup series selection
- command-line database backup

## Larger local run

Use `load-test-local` to select the workload:

```sh
just load-test-local \
  --checks 500 \
  --duration 600 \
  --interval 5s \
  --probe mixed \
  --concurrency 128 \
  --api-readers 4
```

The tool writes a machine-readable result outside the repository by default.
Use `just load-result-check PATH` to confirm that a result passed.

## Long run

The soak command has a 24-hour duration and local fixtures:

```sh
just load-test-soak
```

The command does not bypass its memory, storage, or API latency stop limits.
Run it only after a short admission test passes. A 24-hour or 7-day result is
valid only when the process runs for the complete period.

## Current bounded results

The current 500-check ICMP reference test uses 20 samples, a 300-second
interval, a per-probe concurrency limit of 48, and a 20 percent jitter window.
All 500 checks ran in the aligned slot. The test recorded 10,960 healthy
samples, no lost samples, no duplicate slots, and no missing checks.

A 500-check mixed-probe run completed 2,000 accounted rounds with no unhealthy
or lost samples. Its peak resident memory was 62.6 MiB. The API p95 response
time was 47.9 ms in that test environment.

These results describe the tested revision and test host. They are not a
general capacity guarantee. Use the same commands on the intended host before
you select a production check count.
