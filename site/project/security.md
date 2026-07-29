# Security

Kemuri is for a trusted host or a trusted reverse proxy.

## No built-in login

Kemuri does not authenticate users. A user who can reach the service can read
monitoring data.

Keep the listener on localhost, a private network, or an access-controlled
reverse proxy.

## Configuration reload

The HTTP reload endpoint requires a same-origin JSON request. Kemuri rejects
cross-origin reload.

Disable CORS unless another origin must read API data. CORS never permits a
cross-origin reload.

## Secrets

Use environment or file references for passwords, webhook URLs, HTTP bodies,
and sensitive headers.

Kemuri hashes effective secret values for revision identity. It does not store
or log those values.

Protect these resources:

- configuration file
- secret files
- process environment
- SQLite database
- database backups

## TLS

Keep certificate validation active for HTTP and TCP TLS checks. Add a private
root certificate file when a target uses a private certificate authority.

Do not set `tls_validate: false` as a general fix for certificate errors.

## ICMP capability

Grant `CAP_NET_RAW` only when ICMP checks need it. Do not run the service as
root only to use ICMP.

For containers, add only `NET_RAW`. Do not use a privileged container.

## Reports

Follow the private reporting procedure in
[SECURITY.md](https://github.com/stianfro/kemuri/blob/main/SECURITY.md).

Do not put a security report in a public issue.
