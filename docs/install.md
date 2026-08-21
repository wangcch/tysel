# Install

Tysel's developer toolchain consists of three executables that must be installed
in the same directory:

- `tysel` provides the CLI.
- `tysel-service` is the native stub used by `tysel build`.
- `tysel-worker` runs applications that use the `isolated` profile.

Add that directory to `PATH`. Tysel discovers the service stub and worker next
to the CLI, so an application does not need to configure their paths.

## Build from source

Building from source is currently the portable developer installation path and
requires the Rust toolchain documented in the repository README:

```bash
git clone https://github.com/wangcch/tysel.git
cd tysel
cargo build --locked --release \
  -p tysel-cli --bin tysel \
  -p tysel-runtime --bin tysel-service \
  -p tysel-isolate --bin tysel-worker

mkdir -p "$HOME/.tysel/bin"
cp target/release/tysel \
   target/release/tysel-service \
   target/release/tysel-worker \
   "$HOME/.tysel/bin/"
```

Add the installation directory to your shell configuration:

```bash
export PATH="$HOME/.tysel/bin:$PATH"
```

Verify the installation:

```bash
tysel --version
tysel init hello-tysel
```

## Linux release archives

Tagged releases publish reproducible archives for `linux-x64` and
`linux-arm64`. Download the archive and its `.sha256` sidecar from the same
release, then verify the digest before extracting it:

```bash
archive=tysel-VERSION-linux-x64.tar.gz
expected=$(tr -d '[:space:]' < "$archive.sha256")
actual=$(sha256sum "$archive" | awk '{print $1}')
test "$actual" = "$expected"
tar -xzf "$archive"
```

The three executables are under the extracted archive's `bin/` directory. Copy
all three to one directory on `PATH`. For production admission, also verify the
release signatures, reproducibility proof, and evidence described in
[Production operations](operations/production.md); a checksum alone does not
establish publisher identity.

Prebuilt Darwin archives and `https://tysel.dev/install.sh` are not part of the
current release contract. On macOS, build from source until those artifacts are
published and signed.

The planned installer, `tysel doctor`, `tysel upgrade`, and TypeScript package
work are tracked in the [developer toolchain iteration plan](toolchain-plan.md).
