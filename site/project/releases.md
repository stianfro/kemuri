# Release process

Kemuri uses version tags and GitHub Actions.

## Version tag

A tag in the form `vMAJOR.MINOR.PATCH` starts the release workflows.

Example:

```text
v1.0.0
```

## Platform archives

The release workflow builds these targets:

- `x86_64-unknown-linux-musl`
- `aarch64-unknown-linux-musl`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`
- `x86_64-pc-windows-msvc`

It publishes compressed archives, SHA-256 checksums, shell and PowerShell
installers, a source archive, and a distribution manifest.

Each binary archive also contains:

- sample configuration
- systemd unit
- README
- license
- changelog

## Container image

The container workflow publishes one OCI image index for Linux AMD64 and
ARM64.

It publishes these tags for a stable release:

- full version, for example `1.0.0`
- major and minor version, for example `1.0`
- major version, for example `1`
- `latest`

The workflow publishes build provenance and an SBOM.

## Release checks

Run these tasks before you create a tag:

```sh
just ci
just test-usage
just test-container
just test-load
just audit
just release-check
```

Validate all YAML files with `just yaml`.

Create the tag only after the required checks pass and the Git status is clean.

## Published release

Verify these items after both workflows complete:

1. GitHub Release is not a draft.
2. Each platform archive exists.
3. Each checksum verifies.
4. The source archive exists.
5. The image index has AMD64 and ARM64 manifests.
6. The version command reports the tag version.
