# Install Tysel

Tysel's developer toolchain contains three native executables from one release:

- `tysel` — CLI;
- `tysel-service` — native service runtime and build stub;
- `tysel-worker` — isolated-profile worker.

Install and upgrade them as one unit. Copying only `tysel` produces an incomplete
installation.

## Current availability

Tysel has not published a tagged binary release yet. The current installation
path is a source build with Rust, Node.js 22+, pnpm 11+, and the pinned
workspace dependencies:

```sh
git clone https://github.com/wangcch/tysel.git
cd tysel
pnpm install
cargo build --locked --release \
  -p tysel-cli --bin tysel \
  -p tysel-runtime --bin tysel-service \
  -p tysel-isolate --bin tysel-worker
export PATH="$PWD/target/release:$PATH"
tysel doctor
```

Keep all three binaries together. Doctor reports this as an unmanaged source
installation.

## Managed installer

The managed installer is implemented for Linux and macOS on x64 and arm64, but
the `latest` URL becomes usable only after the first tagged release publishes
`install.sh` and signed platform archives.

After that release exists, the install command will be:

```sh
curl -fsSL https://github.com/wangcch/tysel/releases/latest/download/install.sh | sh
```

The managed installation does not require Rust, Node.js, npm, administrator
access, or `sudo`. Restart the shell if the installer changes its startup file,
then run `tysel doctor --install`.

Create a project:

```sh
tysel init hello-tysel
cd hello-tysel
tysel check
tysel test
```

Node.js is optional after a managed installation. Once the matching npm
packages are published, generated dependencies provide editor declarations and
compiler feedback; the native runtime and packaged production executable do
not need `node_modules`.

## Installer options

After releases are available, select an immutable version:

```sh
curl -fsSL https://github.com/wangcch/tysel/releases/latest/download/install.sh | sh -s -- --version VERSION
```

Preview target, paths, and URLs without changing the machine:

```sh
curl -fsSL https://github.com/wangcch/tysel/releases/latest/download/install.sh | sh -s -- --dry-run
```

Use another absolute managed root or leave shell startup files untouched:

```sh
curl -fsSL https://github.com/wangcch/tysel/releases/latest/download/install.sh | sh -s -- \
  --prefix "$HOME/Tools/tysel" --no-modify-path
```

`TYSEL_HOME` also selects the persistent root. `TYSEL_DOWNLOAD_BASE` is reserved
for an explicit CI fixture, trusted mirror, or enterprise release endpoint.
Unknown options, relative roots, unsupported targets, and roots owned by another
user are rejected.

The default layout is:

```text
~/.tysel/
  versions/vVERSION/bin/{tysel,tysel-service,tysel-worker}
  bin -> versions/vVERSION/bin
  state.json
  trust.json
  upgrade.lock
```

The single `bin` link activates all three executables together. Downloads,
checksums, archive members, the release manifest, file hashes, permissions, and
binary identities are checked before activation. A failed install restores the
previous link and state.

## Diagnose

Local installation, platform, and nearest-project checks run without network or
application execution:

```sh
tysel doctor
tysel doctor --install
tysel doctor --project path/to/project
tysel doctor --json
```

Network checks are opt-in. They authenticate the stable pointer and immutable
manifest with the installed trust policy, then check the target asset:

```sh
tysel doctor --network
```

Warnings return exit status 0; a failed check returns 1. JSON output has a stable
schema version and check IDs suitable for support tooling.

## Upgrade and rollback

Check without changing the installation:

```sh
tysel upgrade --check
```

Upgrade interactively, or confirm explicitly in automation:

```sh
tysel upgrade
tysel upgrade --yes --json
```

If the native version is already current but the authenticated trust policy has
changed, `upgrade` asks before applying a `trust-refresh`. JSON output reports
that mutation with `action: "trust-refresh"` and `changed: true`; `--check`
never changes `trust.json`.

Select an immutable published version. Downgrades additionally require
`--force`:

```sh
tysel upgrade --version VERSION
tysel upgrade --version OLDER_VERSION --force
```

Return to the retained previous release:

```sh
tysel upgrade --rollback
```

Upgrade works only for managed installations. It authenticates release metadata
and the complete archive, stages on the same filesystem, switches the one `bin`
link atomically, and runs a post-switch installation doctor. Failure restores
the prior release. Before resolving release metadata, upgrade authenticates the
latest signed trust policy with the currently installed policy; a successful
upgrade rotates `trust.json` in the same rollback transaction. Systems that stay
offline beyond the installed policy's validity window must reinstall from the
official HTTPS bootstrap.

Release-key rotation uses a bounded overlap ceremony. The transition release
contains the new active key and the previous retired key, and its trust policy
is signed by the previous key so installed clients can authenticate the change.
Release metadata and archives are signed by the new key. Keep
`TYSEL_RELEASE_PREVIOUS_KEY_HEX` configured for the transition release; remove
it only after the overlap window when the new key can sign the next policy by
itself. The repository variables `TYSEL_RELEASE_KEY_VALID_FROM_UNIX` and, during
rotation, `TYSEL_RELEASE_PREVIOUS_KEY_VALID_FROM_UNIX` record each key's original
inception time. `TYSEL_RELEASE_PREVIOUS_KEY_VALID_UNTIL_UNIX` records the fixed
retirement deadline. None of these values may be reset or extended on a later
release. The workflow compares every new policy with the currently published
policy and rejects replay, status regression, deadline changes, and premature
key removal. Clients that miss the complete window must reinstall from the
official bootstrap. A compromised signing key still requires suspending
publication and an out-of-band bootstrap recovery; rotation is not a substitute
for an offline root of trust.

## Security boundary

The preview bootstrap model uses HTTPS plus the release SHA-256 before executing
the downloaded CLI. The checksum detects corruption but does not independently
protect against compromise of the HTTPS distribution origin. The installer then
stores a validated Ed25519 trust policy; `tysel upgrade` requires signed trust,
channel, manifest, and archive metadata. Stable publication accepts only final
`MAJOR.MINOR.PATCH` tags. CI first stages and verifies a draft release, then
publishes matching `@tysel/types` and `@tysel/test` artifacts, and only then
makes the complete GitHub Release public. Prereleases therefore cannot advance
the stable pointer, and native releases cannot generate projects whose matching
npm contracts are unavailable.

Windows native archives are not currently supported. Use WSL on Windows.
