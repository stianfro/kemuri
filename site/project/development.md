# Development

Kemuri is a Rust workspace with a React web UI.

## Repository layout

```text
crates/kemuri-core       Shared domain types
crates/kemuri-config     YAML parsing and resolved configuration
crates/kemuri-probes     ICMP, HTTP, TCP, TLS, and DNS probes
crates/kemuri-storage    SQLite schema, queries, and migrations
crates/kemuri-server     Runtime workers, API, and embedded UI
crates/kemuri-cli        Command-line program
web                      React web UI
openapi                  Tracked OpenAPI document
packaging                Samples, service files, and acceptance tests
site                     This documentation site
```

## Required tools

Install these tools:

- Rust stable
- Node.js 24
- npm
- `just`
- `yq`
- `jq`
- SQLite command-line tools

Container and browser tests also need Docker and Chromium dependencies.

## Use `just`

List tasks:

```sh
just --list
```

Build and test Rust:

```sh
just build
just test
just lint
```

Build and test the web UI:

```sh
just test-web
just test-browser
```

Build the documentation:

```sh
just docs-build
```

Run the change-scoped gate before a commit:

```sh
just ci-diff
```

Run the complete gate when the change affects more than one module:

```sh
just ci
```

## Generated files

The OpenAPI document and frontend types are generated files.

```sh
just api-generate
just api-check
```

The production web bundle is also tracked. `just web-build` fails when the
generated bundle is stale.

## Tests

Use these acceptance tasks when the change affects the related path:

```sh
just test-api
just test-usage
just test-container
just test-load
just test-scale
```

The usage test uses local HTTP, TCP, TLS, DNS, webhook, and browser fixtures.
It does not use public internet services.

The scale tests use bounded local fixtures. See
[Load testing](/project/load-testing) for the test matrix and the opt-in
commands.

## Contributions

Read
[CONTRIBUTING.md](https://github.com/stianfro/kemuri/blob/main/CONTRIBUTING.md)
before you open a pull request.

Use a conventional commit message. Add tests for changed behavior. Keep
generated files current.
