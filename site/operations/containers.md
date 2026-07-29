# Containers

The release workflow publishes one OCI image index for AMD64 and ARM64:

```text
ghcr.io/stianfro/kemuri
```

Use a version tag for a stable deployment:

```sh
docker pull ghcr.io/stianfro/kemuri:1.0.0
```

The image runs as a non-root `kemuri` user. It stores data in
`/var/lib/kemuri`.

## Start a container

Create a directory for data and a configuration file:

```sh
mkdir -p ./kemuri-data
```

Use a configuration with these container paths:

```yaml
version: 1

server:
  bind: 0.0.0.0
  port: 8080

storage:
  path: /var/lib/kemuri/kemuri.db
```

Start the container:

```sh
docker run --name kemuri \
  --restart unless-stopped \
  --publish 127.0.0.1:8080:8080 \
  --volume "$PWD/kemuri.yaml:/etc/kemuri/kemuri.yaml:ro" \
  --volume "$PWD/kemuri-data:/var/lib/kemuri" \
  ghcr.io/stianfro/kemuri:1.0.0
```

The image has a health check that uses the Kemuri liveness endpoint.

## File ownership

The container user must be able to write to the mounted data directory. Set
the directory owner to the user ID in the image when your container runtime
does not map ownership.

Inspect the image user:

```sh
docker image inspect ghcr.io/stianfro/kemuri:1.0.0 \
  --format '{{.Config.User}}'
```

## ICMP checks

Add `NET_RAW` only when the configuration has ICMP checks:

```sh
docker run --cap-add NET_RAW ...
```

Do not use `--privileged`.

Test IPv4 and IPv6 loopback checks on the target host before you use external
addresses. IPv6 availability depends on the host and container network.

## Image metadata

The image includes the package version and source Git revision. The release
workflow also publishes build provenance and an SBOM.

Inspect the image index:

```sh
docker buildx imagetools inspect ghcr.io/stianfro/kemuri:1.0.0
```
