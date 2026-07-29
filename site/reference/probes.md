# Probe settings

All probe settings can appear on a profile. A check can override settings from
its profile.

## ICMP

| Field | Type or values | Default |
|---|---|---|
| `count` | positive integer | `3` |
| `address_family` | `auto`, `ipv4`, or `ipv6` | `auto` |
| `payload_size` | integer from 0 through 65507 | `56` |
| `source_address` | IP address | not set |

The source address must match the selected address family.

Linux needs ping socket permission or `CAP_NET_RAW`. See
[Install Kemuri](../guide/installation#prepare-icmp-permission).

Example:

```yaml
profiles:
  - kind: icmp
    id: ping-v4
    interval: 10s
    timeout: 2s
    count: 3
    address_family: ipv4
    payload_size: 56
```

## HTTP

| Field | Type or values |
|---|---|
| `url` | absolute HTTP or HTTPS URL |
| `method` | HTTP method |
| `headers` | map of header names and values |
| `body` | literal or secret value |
| `expected_status` | one integer or a range |
| `follow_redirects` | Boolean |
| `max_redirect_count` | nonnegative integer |
| `connection_mode` | `pooled`, `per_round`, or `fresh` |
| `measure_until` | `headers` or `body` |
| `user_agent` | string |
| `tls_validate` | Boolean |
| `root_certificates` | list of PEM file paths |

`expected_status` accepts one status:

```yaml
expected_status: 204
```

It also accepts a quoted inclusive range:

```yaml
expected_status: "200-399"
```

Set `measure_until: headers` to stop response latency at the response headers.
Set `measure_until: body` to include the complete response body.

`root_certificates` replaces the inherited list. Kemuri loads each file during
startup and reload.

Example:

```yaml
profiles:
  - kind: http
    id: api-health
    url: https://api.example.com/health
    method: GET
    headers:
      Accept: application/json
    expected_status: "200-299"
    follow_redirects: false
    connection_mode: per_round
    measure_until: body
    tls_validate: true
    interval: 30s
    timeout: 5s
```

## TCP

| Field | Type or values |
|---|---|
| `host` | host name or IP address |
| `port` | integer from 1 through 65535 |
| `address_family` | `auto`, `ipv4`, or `ipv6` |
| `source_address` | IP address |
| `tls.enabled` | Boolean |
| `tls.server_name` | TLS server name |
| `tls.tls_validate` | Boolean |
| `tls.root_certificates` | list of PEM file paths |

The source address must match the selected address family.

When TLS is active, a successful latency measurement includes the TLS
handshake. Kemuri reports TLS errors separately from TCP connection errors.

Example:

```yaml
profiles:
  - kind: tcp
    id: postgres-tls
    host: db.example.com
    port: 5432
    address_family: auto
    tls:
      enabled: true
      server_name: db.example.com
      tls_validate: true
    interval: 30s
    timeout: 5s
```

## DNS

| Field | Type or values |
|---|---|
| `name` | DNS name |
| `server` | DNS server address and optional port |
| `record_type` | DNS record type |
| `protocol` | `udp` or `tcp` |
| `expected_rcode` | DNS response code |
| `require_answer` | Boolean |

Supported response codes are:

- `noerror`
- `formerr`
- `servfail`
- `nxdomain`
- `notimp`
- `refused`

`domain` is an accepted alias for `name`. `resolver` is an accepted alias for
`server`. Use `name` and `server` in new configuration files.

Example:

```yaml
profiles:
  - kind: dns
    id: dns-a
    name: example.com
    server: 1.1.1.1:53
    record_type: A
    protocol: udp
    expected_rcode: noerror
    require_answer: true
    interval: 30s
    timeout: 5s
```

Use a TCP check with `protocol: tcp` when you must test DNS over TCP.

## Latency and failures

Kemuri keeps response latency separate from total elapsed time. A timeout or
network error does not enter a latency histogram.

An unexpected protocol response is a health failure. No usable response is
measurement loss.
