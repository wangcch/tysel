# Tysel native toolchain

This archive contains one version-matched Tysel toolchain for its named target:

- `bin/tysel` — CLI;
- `bin/tysel-service` — service runtime and application build stub;
- `bin/tysel-worker` — isolated-profile worker.

Keep the three executables together. Add `bin/` to `PATH`, then verify the
installation:

```sh
tysel --version
tysel doctor --install
```

The managed installer is recommended for authenticated installation, upgrades,
and rollback:

```sh
curl -fsSL https://tysel.dev/install.sh | sh
```

Start with the [installation guide](https://tysel.dev/docs/install/) or
[create your first service](https://tysel.dev/docs/getting-started/). Review the
[security model](https://tysel.dev/docs/security/) and
[production operations](https://tysel.dev/docs/operations/production/) before
deploying.

Tysel is licensed under [Apache-2.0](LICENSE).
