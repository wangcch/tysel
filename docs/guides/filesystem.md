# Read and write bounded files

This guide grants two directory roots, transforms one UTF-8 JSON file, writes
the result beneath a separate output root, and verifies that paths outside the
allowlist remain denied.

## Prepare the roots

Create the directories before starting Tysel. The runtime pins each configured
root as a directory and does not create a missing root for you:

```sh
mkdir -p input output
printf '%s\n' '{"items":["alpha","beta"]}' > input/jobs.json
```

Grant read and write independently:

```toml
[app]
name = "filesystem-transform"
entry = "src/index.ts"
profile = "service"

[permissions]
fs_read = ["./input"]
fs_write = ["./output"]
```

Relative roots resolve from the manifest directory for project commands.
Absolute roots remain absolute. In a packaged executable, relative runtime
paths resolve from the process working directory, so production must set a
stable working directory or use deliberate absolute deployment roots.

`fs_read` never implies write access, and `fs_write` never implies read access.
Keep them separate even when an application currently needs both.

## Transform one file

Generate the capability environment after updating the manifest:

```sh
tysel types
```

```ts
import type { TyselApp } from "@tysel/types";
import type { TyselEnv } from "../tysel-env.js";

export default {
  async fetch(_request, runtime) {
    const source = await runtime.fs.read("input/jobs.json");
    const input = JSON.parse(source) as { items: string[] };
    const result = {
      count: input.items.length,
      items: input.items.map((item) => item.toUpperCase()),
    };
    await runtime.fs.write("output/result.json", JSON.stringify(result, null, 2));
    return Response.json(result);
  },
} satisfies TyselApp<TyselEnv>;
```

Run the complete [filesystem transform example](https://github.com/wangcch/tysel/tree/main/examples/filesystem-transform):

```sh
cd examples/filesystem-transform
tysel check
tysel inspect
tysel run
```

From another terminal:

```sh
curl -sS http://127.0.0.1:3000/transform
cat output/result.json
```

Both commands show two uppercase items.

## Understand confinement

The capability accepts only UTF-8 regular files on Unix platforms. Each read
or write is limited to 1 MiB. A write creates or truncates the final file with
mode `0644`; its parent directory must already exist.

Tysel rejects:

- an empty path or an operation with no corresponding roots;
- a path outside every declared root;
- `..` traversal in either the root or requested path;
- symlink and magic-link traversal beneath a pinned root;
- directories, FIFOs, devices, sockets, and other non-regular files;
- invalid UTF-8 reads;
- reads or writes larger than 1 MiB.

The root directory descriptor is pinned before access. Replacing the configured
directory path with a symlink after startup does not redirect later operations
outside the original root.

Verify a denial without changing the manifest:

```ts
try {
  await tysel.fs.read("./tysel.toml");
  throw new Error("unexpected filesystem grant");
} catch (error) {
  console.log(String(error)); // path is not permitted
}
```

Do not return raw filesystem errors to untrusted callers. Log the bounded
capability failure with the request ID and return an application-owned error.

## Profile and deployment differences

- `service` can use the JavaScript filesystem client with matching roots.
- `isolated` denies filesystem calls even when roots are listed.
- `component` uses versioned WIT imports and also requires an explicit packaged
  deployment grant; see [Component capabilities](../reference/component/capabilities.md).
- native JavaScript filesystem confinement requires Unix. Windows users should
  use WSL rather than expect ambient Win32 path behavior.

Mount production roots read-only or read-write at the operating-system layer to
match the manifest, create output directories before activation, and keep
temporary/untrusted uploads outside application source and executable paths.

See [Filesystem permissions](../reference/manifest/permissions.md#filesystem),
[Host capabilities](../reference/runtime/capabilities.md#filesystem), and
[Limits and defaults](../reference/limits-and-defaults.md#data-capabilities).
