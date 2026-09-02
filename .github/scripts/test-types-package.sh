#!/usr/bin/env bash
set -euo pipefail

root="$(pwd)"
temporary="$(mktemp -d)"
trap 'rm -rf "$temporary"' EXIT
export npm_config_cache="$temporary/npm-cache"

pnpm --dir "$root/packages/tysel-types" pack --pack-destination "$temporary" > /dev/null
pnpm --dir "$root/packages/tysel-test" pack --pack-destination "$temporary" > /dev/null
pnpm --dir "$root/packages/tysel" pack --pack-destination "$temporary" > /dev/null
types_archive="$(find "$temporary" -maxdepth 1 -name 'tysel-types-*.tgz' -print -quit)"
test_archive="$(find "$temporary" -maxdepth 1 -name 'tysel-test-*.tgz' -print -quit)"
sdk_archive="$(find "$temporary" -maxdepth 1 -name 'tysel-sdk-*.tgz' -print -quit)"
[[ -n "$types_archive" ]] || { echo "@tysel/types package archive was not created" >&2; exit 1; }
[[ -n "$test_archive" ]] || { echo "@tysel/test package archive was not created" >&2; exit 1; }
[[ -n "$sdk_archive" ]] || { echo "@tysel/sdk package archive was not created" >&2; exit 1; }

contents="$(tar -tzf "$types_archive")"
if grep -Eq '^package/(src|node_modules)/|\.js$' <<< "$contents" \
  || grep -E '\.ts$' <<< "$contents" | grep -Evq '\.d\.ts$'; then
  echo "@tysel/types archive contains source, JavaScript, or dependencies" >&2
  exit 1
fi
for required in package/package.json package/dist/index.d.ts package/README.md package/LICENSE; do
  grep -Fxq "$required" <<< "$contents" || {
    echo "@tysel/types archive is missing ${required}" >&2
    exit 1
  }
done
tar -xOf "$types_archive" package/package.json | jq -e '
  .name == "@tysel/types"
  and (.dependencies // {} | length) == 0
  and .repository.type == "git"
  and .repository.url == "git+https://github.com/wangcch/tysel.git"
  and .repository.directory == "packages/tysel-types"
  and (.scripts.preinstall // null) == null
  and (.scripts.install // null) == null
  and (.scripts.postinstall // null) == null' > /dev/null

test_contents="$(tar -tzf "$test_archive")"
if grep -Eq '^package/(src|node_modules)/' <<< "$test_contents" \
  || grep -E '\.ts$' <<< "$test_contents" | grep -Evq '\.d\.ts$'; then
  echo "@tysel/test archive contains TypeScript source or dependencies" >&2
  exit 1
fi
for required in package/package.json package/dist/index.js package/dist/index.d.ts package/README.md package/LICENSE; do
  grep -Fxq "$required" <<< "$test_contents" || {
    echo "@tysel/test archive is missing ${required}" >&2
    exit 1
  }
done
tar -xOf "$test_archive" package/package.json | jq -e '
  .name == "@tysel/test"
  and .dependencies["@tysel/types"] == .version
  and .repository.type == "git"
  and .repository.url == "git+https://github.com/wangcch/tysel.git"
  and .repository.directory == "packages/tysel-test"
  and (.scripts.preinstall // null) == null
  and (.scripts.install // null) == null
  and (.scripts.postinstall // null) == null' > /dev/null

sdk_contents="$(tar -tzf "$sdk_archive")"
if grep -Eq '^package/(src|node_modules)/' <<< "$sdk_contents" \
  || grep -E '\.ts$' <<< "$sdk_contents" | grep -Evq '\.d\.ts$'; then
  echo "@tysel/sdk archive contains TypeScript source or dependencies" >&2
  exit 1
fi
for required in package/package.json package/dist/index.js package/dist/index.d.ts package/README.md package/LICENSE; do
  grep -Fxq "$required" <<< "$sdk_contents" || {
    echo "@tysel/sdk archive is missing ${required}" >&2
    exit 1
  }
done
tar -xOf "$sdk_archive" package/package.json | jq -e '
  .name == "@tysel/sdk"
  and .dependencies["@tysel/types"] == .version
  and .repository.type == "git"
  and .repository.url == "git+https://github.com/wangcch/tysel.git"
  and .repository.directory == "packages/tysel"
  and (.scripts.preinstall // null) == null
  and (.scripts.install // null) == null
  and (.scripts.postinstall // null) == null' > /dev/null

mkdir -p "$temporary/consumer"
cd "$temporary/consumer"
cp "$root/packages/tysel-types/test/consumer/package.json" .
npm install --ignore-scripts --no-audit --no-fund \
  "$types_archive" "$test_archive" "$sdk_archive" > /dev/null
cp "$root/packages/tysel-types/test/consumer/tsconfig.json" .
cp "$root/packages/tysel-types/test/consumer/index.ts" .

"$root/node_modules/.bin/tsc" -p tsconfig.json
node --input-type=module -e \
  'import { invokeFetch } from "@tysel/test"; const response = await invokeFetch(() => new Response("ok"), "https://example.test"); if (await response.text() !== "ok") process.exit(1)'
node --input-type=module -e \
  'import { defineApp, mcp } from "@tysel/sdk"; const app = { fetch: () => new Response("ok") }; const task = mcp({ description: "echo", input: { value: "string" }, handler: ({ value }) => value }); if (defineApp(app) !== app || task.handler({ value: "ok" }) !== "ok") process.exit(1)'
