# ICMP with systemd

The Kemuri ICMP probe uses `SOCK_DGRAM` + `IPPROTO_ICMP` on Linux for
unprivileged ICMP Echo. This requires the process to be in a group allowed by
the `net.ipv4.ping_group_range` sysctl.

## Default sysctl

On most Linux distributions, the default `ping_group_range` allows ICMP for
GID 0 (root) only:

```
net.ipv4.ping_group_range = 1 0
```

This means a non-root Kemuri process cannot create ICMP sockets.

## Option 1: Set the sysctl globally

Edit `/etc/sysctl.d/99-kemuri.conf`:

```
net.ipv4.ping_group_range = 0 2147483647
```

Apply with:

```sh
sysctl -p /etc/sysctl.d/99-kemuri.conf
```

Restrict the range to the Kemuri process GID for production use:

```
net.ipv4.ping_group_range = 1000 1000
```

## Option 2: Use systemd AmbientCapabilities

Add `CAP_NET_RAW` to the service unit. This allows the process to create raw
ICMP sockets:

```ini
[Service]
AmbientCapabilities=CAP_NET_RAW
```

## Option 3: Run as root

Running Kemuri as root is the simplest option but is not recommended for
production.

## systemd service example

```ini
[Unit]
Description=Kemuri Network Monitor
After=network.target

[Service]
Type=simple
ExecStart=/usr/local/bin/kemuri serve --config /etc/kemuri/config.yaml
User=kemuri
Group=kemuri
AmbientCapabilities=CAP_NET_RAW
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

## Verification

Check the current `ping_group_range`:

```sh
cat /proc/sys/net/ipv4/ping_group_range
```

Test ICMP as the Kemuri user:

```sh
sudo -u kemuri ping -c 1 127.0.0.1
```
