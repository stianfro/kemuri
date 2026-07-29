# Status model

Kemuri keeps one current state for each active check.

## Check states

| State | Condition |
|---|---|
| `healthy` | All attempted responses are healthy. |
| `degraded` | Results are mixed, or a protocol response is unhealthy. |
| `down` | All attempted samples have measurement loss. |
| `no_data` | Kemuri did not attempt a sample. |

Measurement loss means that Kemuri did not receive a usable protocol response.
Examples include a timeout, connection error, and DNS transport error.

A protocol-health failure means that Kemuri received a response that did not
meet the configured condition. Examples include an unexpected HTTP status or
DNS response code.

## Target state

The target state is the worst state of its active checks.

The order from best to worst is:

1. `healthy`
2. `degraded`
3. `down`
4. `no_data`

Disabled checks do not affect the target state or active check count.

## Series bucket states

The series API uses fixed time buckets.

| State | Meaning |
|---|---|
| `observed` | The bucket contains one or more stored rounds. |
| `skipped` | Kemuri recorded a no-data scheduling result for the bucket. |
| `missing` | The bucket has no stored observation. |

The graph puts each bucket at its actual time. It does not use the array
position as the time position.

## Alert data

Alert rules can use measurement loss, health failures, latency values, or
failure counts. A rule can require minimum round and latency sample counts
before it evaluates the state.
