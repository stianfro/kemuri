# ICMP in Containers

The Kemuri ICMP probe uses `SOCK_DGRAM` + `IPPROTO_ICMP` on Linux for
unprivileged ICMP Echo. This requires the container process to be in a group
that is allowed by the `net.ipv4.ping_group_range` sysctl.

## Docker

Add the `sysctls` option to your `docker-compose.yml` or `docker run` command:

```yaml
services:
  kemuri:
    image: kemuri:latest
    sysctls:
      - net.ipv4.ping_group_range=0 2147483647
```

Or with `docker run`:

```sh
docker run --sysctl net.ipv4.ping_group_range=0\ 2147483647 kemuri:latest
```

The range `0 2147483647` allows all groups. Restrict this to a specific GID
for production use, for example `1000 1000` to allow only GID 1000.

## Podman

Podman uses the same `--sysctl` flag:

```sh
podman run --sysctl net.ipv4.ping_group_range=0\ 2147483647 kemuri:latest
```

## Kubernetes

Set the sysctl in the pod spec:

```yaml
apiVersion: v1
kind: Pod
metadata:
  name: kemuri
spec:
  securityContext:
    sysctls:
      - name: net.ipv4.ping_group_range
        value: "0 2147483647"
  containers:
    - name: kemuri
      image: kemuri:latest
```

Note: `net.ipv4.ping_group_range` is a non-namespaced sysctl. Your cluster
must allow it via the `--allowed-unsafe-sysctls` kubelet flag.

## Verification

Run the capability check inside the container:

```sh
kemuri check-icmp
```

Or manually test with:

```sh
ping -c 1 127.0.0.1
```

If `ping` works without root, ICMP sockets are available.
