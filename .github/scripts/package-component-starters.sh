#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <output-dir> <version> <source-date-epoch>" >&2
  exit 2
fi

output_dir="$1"
version="$2"
source_date_epoch="$3"
bash "$(dirname "$0")/release-channel.sh" "$version" > /dev/null
[[ "$source_date_epoch" =~ ^(0|[1-9][0-9]*)$ ]] || {
  echo "source-date-epoch must be a non-negative integer" >&2
  exit 2
}

root="tysel-component-starters"
staging="target/component-starters-package"
archive="$output_dir/${root}.tar.gz"
rm -rf "$staging"
mkdir -p \
  "$staging/$root/rust-echo/src" \
  "$staging/$root/rust-echo/wit/component" \
  "$staging/$root/rust-echo/sdk/tysel-component-sdk/src" \
  "$staging/$root/go-echo/export_wit_world" \
  "$staging/$root/go-echo/wit/component" \
  "$staging/$root/go-echo/sdk/component-go"
mkdir -p "$output_dir"
cp LICENSE "$staging/$root/LICENSE"

cp sdk/examples/rust-echo/.gitignore "$staging/$root/rust-echo/"
cp sdk/examples/rust-echo/Cargo.lock "$staging/$root/rust-echo/"
cp sdk/examples/rust-echo/tysel.toml "$staging/$root/rust-echo/"
cp crates/tysel-component-sdk/src/lib.rs \
  "$staging/$root/rust-echo/sdk/tysel-component-sdk/src/"
cp wit/component/task.wit "$staging/$root/rust-echo/wit/component/"

sed \
  's#../../../crates/tysel-component-sdk#sdk/tysel-component-sdk#' \
  sdk/examples/rust-echo/Cargo.toml \
  > "$staging/$root/rust-echo/Cargo.toml"
sed \
  's#../../../wit/component#wit/component#' \
  sdk/examples/rust-echo/src/lib.rs \
  > "$staging/$root/rust-echo/src/lib.rs"
sed \
  's#../../../docs/guides/wasm-component-rust.md#https://tysel.dev/docs/guides/wasm-component-rust#' \
  sdk/examples/rust-echo/README.md \
  > "$staging/$root/rust-echo/README.md"
sed \
  -e 's#../../sdk/examples/rust-echo/README.md#https://tysel.dev/docs/guides/wasm-component-rust#' \
  -e 's#../../sdk/README.md#https://tysel.dev/reference/component/rust-sdk#' \
  crates/tysel-component-sdk/README.md \
  > "$staging/$root/rust-echo/sdk/tysel-component-sdk/README.md"
cat > "$staging/$root/rust-echo/sdk/tysel-component-sdk/Cargo.toml" <<EOF
[package]
name = "tysel-component-sdk"
version = "$version"
edition = "2024"
license = "Apache-2.0"
description = "Language-level JSON task contract for Tysel Wasm Components."
readme = "README.md"
publish = false

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
EOF

cp sdk/examples/go-echo/.gitignore "$staging/$root/go-echo/"
cp sdk/examples/go-echo/go.sum "$staging/$root/go-echo/"
cp sdk/examples/go-echo/tysel.toml "$staging/$root/go-echo/"
cp sdk/examples/go-echo/wit_exports.go "$staging/$root/go-echo/"
cp sdk/examples/go-echo/export_wit_world/exports.go \
  "$staging/$root/go-echo/export_wit_world/"
cp sdk/component-go/go.mod "$staging/$root/go-echo/sdk/component-go/"
cp sdk/component-go/task.go "$staging/$root/go-echo/sdk/component-go/"
cp sdk/component-go/task_test.go "$staging/$root/go-echo/sdk/component-go/"
cp wit/component/task.wit "$staging/$root/go-echo/wit/component/"

sed \
  's#../../component-go#./sdk/component-go#' \
  sdk/examples/go-echo/go.mod \
  > "$staging/$root/go-echo/go.mod"
sed \
  -e 's#../../../wit/component#wit/component#g' \
  -e 's#../../../docs/guides/wasm-component-go.md#https://tysel.dev/docs/guides/wasm-component-go#' \
  sdk/examples/go-echo/README.md \
  > "$staging/$root/go-echo/README.md"
sed \
  's#../examples/go-echo/README.md#https://tysel.dev/docs/guides/wasm-component-go#' \
  sdk/component-go/README.md \
  > "$staging/$root/go-echo/sdk/component-go/README.md"

tar --sort=name --format=ustar --owner=0 --group=0 --numeric-owner \
  --mtime="@${source_date_epoch}" -C "$staging" -cf - "$root" \
  | gzip -n -9 > "$archive"
sha256sum "$archive" | awk '{print $1}' > "${archive}.sha256"

tar -tzf "$archive" | grep -Fx "${root}/rust-echo/wit/component/task.wit" > /dev/null
tar -tzf "$archive" | grep -Fx "${root}/go-echo/wit/component/task.wit" > /dev/null
tar -tzf "$archive" | grep -Fx "${root}/LICENSE" > /dev/null
echo "packaged ${archive}"
