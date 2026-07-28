# Security Policy

## Reporting a Vulnerability

Report security vulnerabilities by opening a private security advisory on GitHub, or by emailing the maintainers directly.

Do not report security vulnerabilities through public GitHub issues.

## Secrets

Kemuri configuration supports secret references via `from_env` and `from_file` to avoid storing credentials in YAML files. Literal secret values in configuration trigger warnings.

Internal error details are never returned to API clients. All errors are logged server-side with request IDs for correlation.
