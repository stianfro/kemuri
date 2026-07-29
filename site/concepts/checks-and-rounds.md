# Checks and rounds

Kemuri uses profiles to define reusable probe settings. A target contains one
or more checks. Each check refers to one profile.

## Profile

A profile has a probe kind, an ID, an interval, a timeout, and probe settings.
Checks can use one profile on many targets.

## Target

A target identifies the monitored system. It has an ID, an address, an
optional name, an optional group path, labels, and checks.

## Check

A check joins a target with a profile. A check can override profile settings.

Kemuri applies these inheritance rules:

- A scalar check value replaces the profile value.
- HTTP headers merge by header name.
- A list on a check replaces the profile list.

Kemuri validates the final resolved settings. A check cannot change the probe
kind of its profile.

## Scheduled slot

Each interval creates one aligned time slot. Stable jitter moves the dispatch
time inside the configured jitter range.

The default startup mode runs one immediate round. It then moves to aligned
slots. Set `scheduler.startup_mode` to `aligned` to omit the immediate round.

## Round

A round is one scheduled execution of one check. It records:

- scheduled, start, and finish times
- configured and attempted sample counts
- latency-bearing, healthy, unhealthy, and lost sample counts
- minimum, median, and maximum latency
- the execution result and stop reason
- the configuration generation and check revision

Latency histograms contain only valid response latency. Timeouts and network
errors do not enter the latency histogram.

## No-data round

Kemuri writes a no-data round when it cannot attempt a sample. Examples are an
overlap, queue backpressure, and a disk-pressure scheduling pause.

Kemuri records the slot and moves to the next slot. It does not delay the old
slot.

## Disabled entries

Set `enabled: false` on a target or check to stop new scheduling. Kemuri keeps
old rounds, revisions, and alert history.
