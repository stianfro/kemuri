# Use the web UI

The Kemuri binary contains the web UI. The UI uses the same HTTP listener as
the API.

## Overview

The overview shows these totals:

- targets
- healthy targets
- degraded targets
- down targets
- targets with no data
- active alerts

The page groups targets by their configured group path.

![Kemuri overview with healthy and down targets](/screenshots/overview.png)

## Target page

Select a target to see its active checks. The table shows the probe type,
current state, latency, and measurement loss.

The target state is the worst state of its active checks.

## Check page

Select a check to see its current measurements and history.

![Kemuri check page with a 24-hour intermittent ICMP smoke graph](/screenshots/check-detail.png)

The smoke graph contains these data layers:

- median latency
- p95 latency
- measurement-loss bands
- protocol-health-failure bands
- skipped and missing time buckets
- alert intervals
- configuration revision markers

Select a time range from one hour through 90 days. Kemuri uses rollups for
long ranges. It reads raw rounds when a rollup bucket is not complete.

The time control changes between browser local time and UTC. The browser keeps
the selected setting.

## Alerts page

The alerts page shows active and resolved alert states. It also shows the
associated rule, target, check, and event times.

## System page

The system page shows the version, build target, runtime state, database state,
and active configuration generation.

![Kemuri system status page](/screenshots/system.png)

## Live updates

The UI receives state events from `/api/v1/events`. This endpoint uses
server-sent events.

The event stream is not a durable log. The UI reads current data again after a
reconnect.

If you use a reverse proxy, disable response buffering for this endpoint.
