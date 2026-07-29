# Install Kemuri

Kemuri release archives contain one binary, a sample configuration, a systemd
unit, the license, and project documents.

## Supported systems

The server runtime supports Linux. Release archives also provide the CLI for
macOS and Windows.

| System | Architecture | Archive |
|---|---|---|
| Linux | x86-64 | `kemuri-x86_64-unknown-linux-musl.tar.xz` |
| Linux | ARM64 | `kemuri-aarch64-unknown-linux-musl.tar.xz` |
| macOS | x86-64 | `kemuri-x86_64-apple-darwin.tar.xz` |
| macOS | ARM64 | `kemuri-aarch64-apple-darwin.tar.xz` |
| Windows | x86-64 | `kemuri-x86_64-pc-windows-msvc.zip` |

## Install with the shell installer

Use this command on Linux or macOS:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/stianfro/kemuri/releases/latest/download/kemuri-installer.sh | sh
```

The installer selects the correct archive. It verifies the SHA-256 checksum
before it installs the binary.

## Install with PowerShell

Use this command in PowerShell:

```powershell
irm https://github.com/stianfro/kemuri/releases/latest/download/kemuri-installer.ps1 | iex
```

## Install an archive

1. Download an archive and its `.sha256` file from the
   [release page](https://github.com/stianfro/kemuri/releases).
2. Verify the checksum.
3. Extract the archive.
4. Copy `kemuri` to a directory in `PATH`.

This example is for Linux x86-64:

```sh
sha256sum --check kemuri-x86_64-unknown-linux-musl.tar.xz.sha256
tar -xf kemuri-x86_64-unknown-linux-musl.tar.xz
sudo install -m 0755 \
  kemuri-x86_64-unknown-linux-musl/kemuri \
  /usr/local/bin/kemuri
```

## Verify the installation

```sh
kemuri --version
kemuri version
```

## Prepare ICMP permission

ICMP checks need Linux ping socket permission or the `CAP_NET_RAW` capability.
Do not add this capability if you do not use ICMP checks.

Add the capability to the installed binary:

```sh
sudo setcap cap_net_raw=+ep /usr/local/bin/kemuri
```

An operating system update or a new binary can remove this capability. Check
the capability after each update:

```sh
getcap /usr/local/bin/kemuri
```

## Next step

Continue with the [quick start](./quick-start).
