# Filesystem transform

This service reads one UTF-8 JSON file beneath `./input` and writes a transformed
result beneath the independent `./output` root.

```sh
tysel check
tysel inspect
tysel run
```

From another terminal:

```sh
curl -sS http://127.0.0.1:3000/transform
cat output/result.json
```

Paths outside the declared roots, traversal, symlinks, non-regular files, and
operations larger than 1 MiB are denied. See the
[filesystem guide](../../docs/guides/filesystem.md).
