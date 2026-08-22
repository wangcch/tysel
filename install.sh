#!/bin/sh
# Tysel developer-toolchain bootstrap installer.
set -eu

PROGRAM=tysel-install
DEFAULT_DOWNLOAD_BASE=https://github.com/wangcch/tysel/releases
CHANNEL=stable
VERSION=
if [ -n "${TYSEL_HOME:-}" ]; then
  PREFIX=$TYSEL_HOME
elif [ -n "${HOME:-}" ]; then
  PREFIX=$HOME/.tysel
else
  PREFIX=
fi
MODIFY_PATH=1
DRY_RUN=0
DOWNLOAD_BASE=${TYSEL_DOWNLOAD_BASE:-$DEFAULT_DOWNLOAD_BASE}
LOCK_PATH=
LOCK_CANDIDATE=
LOCK_ACQUIRED=0
WORK_DIR=
ACTIVATED=0
OLD_LINK=
OLD_STATE=
OLD_TRUST=

say() { printf '%s\n' "$*"; }
fail() { printf '%s: %s\n' "$PROGRAM" "$*" >&2; exit 1; }

usage() {
  cat <<'EOF'
usage: install.sh [--version <semver>] [--channel stable] [--prefix <absolute-path>]
                  [--no-modify-path] [--dry-run]
EOF
}

cleanup() {
  status=$?
  trap - EXIT HUP INT TERM
  if [ "$status" -ne 0 ] && [ "$ACTIVATED" -eq 1 ]; then
    if [ -n "$OLD_LINK" ]; then
      replace_link "$OLD_LINK" || true
    else
      rm -f "$PREFIX/bin" || true
    fi
    if [ -n "$OLD_STATE" ] && [ -f "$OLD_STATE" ]; then
      cp "$OLD_STATE" "$PREFIX/.state.rollback.$$" 2>/dev/null || true
      mv -f "$PREFIX/.state.rollback.$$" "$PREFIX/state.json" 2>/dev/null || true
    else
      rm -f "$PREFIX/state.json" || true
    fi
    if [ -n "$OLD_TRUST" ] && [ -f "$OLD_TRUST" ]; then
      cp "$OLD_TRUST" "$PREFIX/.trust.rollback.$$" 2>/dev/null || true
      mv -f "$PREFIX/.trust.rollback.$$" "$PREFIX/trust.json" 2>/dev/null || true
    else
      rm -f "$PREFIX/trust.json" || true
    fi
  fi
  [ -z "$WORK_DIR" ] || rm -rf "$WORK_DIR"
  if [ "$LOCK_ACQUIRED" -eq 1 ]; then
    rm -f "$LOCK_PATH" 2>/dev/null || true
  fi
  [ -z "$LOCK_CANDIDATE" ] || rm -f "$LOCK_CANDIDATE" 2>/dev/null || true
  exit "$status"
}
trap cleanup EXIT HUP INT TERM

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version)
      [ "$#" -ge 2 ] || fail "--version requires a value"
      VERSION=$2
      shift 2
      ;;
    --channel)
      [ "$#" -ge 2 ] || fail "--channel requires a value"
      CHANNEL=$2
      shift 2
      ;;
    --prefix)
      [ "$#" -ge 2 ] || fail "--prefix requires a value"
      PREFIX=$2
      shift 2
      ;;
    --no-modify-path) MODIFY_PATH=0; shift ;;
    --dry-run) DRY_RUN=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) fail "unknown option: $1" ;;
  esac
done

[ "$CHANNEL" = stable ] || fail "unsupported channel: $CHANNEL"
[ -n "$PREFIX" ] || fail "HOME is not set; pass --prefix or TYSEL_HOME"
case "$PREFIX" in
  /*) ;;
  *) fail "install root must be an absolute path" ;;
esac
case "$PREFIX" in
  *"
"*) fail "install root must not contain a newline" ;;
esac

valid_version() {
  printf '%s\n' "$1" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+([+-][0-9A-Za-z.-]+)?$'
}

detect_target() {
  os=$(uname -s 2>/dev/null || true)
  arch=$(uname -m 2>/dev/null || true)
  case "$os:$arch" in
    Linux:x86_64|Linux:amd64) printf '%s\n' linux-x64 ;;
    Linux:aarch64|Linux:arm64) printf '%s\n' linux-arm64 ;;
    Darwin:x86_64)
      if [ "$(/usr/sbin/sysctl -in sysctl.proc_translated 2>/dev/null || true)" = 1 ]; then
        printf '%s\n' darwin-arm64
      else
        printf '%s\n' darwin-x64
      fi
      ;;
    Darwin:arm64) printf '%s\n' darwin-arm64 ;;
    *) return 1 ;;
  esac
}

TARGET=$(detect_target) || fail "unsupported platform: $(uname -s 2>/dev/null || printf unknown)/$(uname -m 2>/dev/null || printf unknown); Windows users should use WSL"

download() {
  url=$1
  output=$2
  command -v curl >/dev/null 2>&1 || fail "curl is required to download Tysel"
  if [ "$DOWNLOAD_BASE" = "$DEFAULT_DOWNLOAD_BASE" ]; then
    curl --proto '=https' --tlsv1.2 --location --fail --silent --show-error \
      --max-redirs 5 --retry 3 --connect-timeout 10 --max-time 120 \
      --output "$output" "$url"
  else
    curl --location --fail --silent --show-error --max-redirs 5 --retry 3 \
      --connect-timeout 10 --max-time 120 --output "$output" "$url"
  fi
}

if [ -z "$VERSION" ]; then
  if [ "$DRY_RUN" -eq 1 ]; then
    VERSION='<stable-version>'
  else
    version_file=$(mktemp "${TMPDIR:-/tmp}/tysel-version.XXXXXX") || fail "cannot create a temporary file"
    download "${DOWNLOAD_BASE%/}/latest/download/stable-version" "$version_file"
    VERSION=$(tr -d '[:space:]' < "$version_file")
    rm -f "$version_file"
  fi
fi
[ "$VERSION" = '<stable-version>' ] || valid_version "$VERSION" || fail "invalid semantic version: $VERSION"

if [ "$VERSION" = '<stable-version>' ]; then
  release_base="${DOWNLOAD_BASE%/}/download/v<VERSION>"
  archive="tysel-<VERSION>-${TARGET}.tar.gz"
else
  release_base="${DOWNLOAD_BASE%/}/download/v${VERSION}"
  archive="tysel-${VERSION}-${TARGET}.tar.gz"
fi

say "Tysel install plan"
say "  version: $VERSION"
say "  target:  $TARGET"
say "  root:    $PREFIX"
say "  archive: ${release_base}/${archive}"
[ "$DRY_RUN" -eq 0 ] || exit 0

if [ -e "$PREFIX" ]; then
  owner=$(stat -f '%u' "$PREFIX" 2>/dev/null || stat -c '%u' "$PREFIX" 2>/dev/null || printf unknown)
  [ "$owner" = "$(id -u)" ] || fail "install root is not owned by the current user"
fi
umask 077
mkdir -p "$PREFIX" "$PREFIX/versions" "$PREFIX/.staging"
LOCK_PATH="$PREFIX/upgrade.lock"
LOCK_CANDIDATE="$PREFIX/.upgrade-lock-$$"
printf '%s\n' "$$" > "$LOCK_CANDIDATE"
attempt=0
while ! ln "$LOCK_CANDIDATE" "$LOCK_PATH" 2>/dev/null; do
  attempt=$((attempt + 1))
  lock_pid=$(sed -n '1p' "$LOCK_PATH" 2>/dev/null || true)
  case "$lock_pid" in
    ''|*[!0-9]*) ;;
    *)
      if ! kill -0 "$lock_pid" 2>/dev/null; then
        rm -f "$LOCK_PATH" 2>/dev/null || true
        continue
      fi
      ;;
  esac
  [ "$attempt" -lt 10 ] || fail "another install or upgrade holds $LOCK_PATH"
  sleep 1
done
LOCK_ACQUIRED=1
rm -f "$LOCK_CANDIDATE"
LOCK_CANDIDATE=

WORK_DIR="$PREFIX/.staging/install-$$"
mkdir "$WORK_DIR"
archive_path="$WORK_DIR/$archive"
checksum_path="$WORK_DIR/$archive.sha256"
manifest_path="$WORK_DIR/release-manifest.json"
trust_path="$WORK_DIR/trust.json"
download "${release_base}/${archive}" "$archive_path"
download "${release_base}/${archive}.sha256" "$checksum_path"
download "${release_base}/release-manifest.json" "$manifest_path"
download "${release_base}/trust.json" "$trust_path"

[ "$(wc -c < "$archive_path" | tr -d '[:space:]')" -le 268435456 ] || fail "release archive exceeds the 256 MiB bootstrap limit"
expected=$(awk 'NR == 1 { print $1 }' "$checksum_path")
case "$expected" in
  [0-9a-f][0-9a-f]*) [ "${#expected}" -eq 64 ] || fail "invalid SHA-256 sidecar" ;;
  *) fail "invalid SHA-256 sidecar" ;;
esac
if command -v sha256sum >/dev/null 2>&1; then
  actual=$(sha256sum "$archive_path" | awk '{print $1}')
else
  actual=$(shasum -a 256 "$archive_path" | awk '{print $1}')
fi
[ "$actual" = "$expected" ] || fail "release archive SHA-256 mismatch"

members="$WORK_DIR/archive-members"
details="$WORK_DIR/archive-details"
tar -tzf "$archive_path" > "$members" || fail "cannot list release archive"
tar -tvzf "$archive_path" > "$details" || fail "cannot inspect release archive"
root_name="tysel-${VERSION}-${TARGET}"
awk 'substr($1, 1, 1) != "-" && substr($1, 1, 1) != "d" { exit 1 }' "$details" \
  || fail "release archive contains a link, device, or unsupported member"
while IFS= read -r member; do
  case "$member" in
    /*|../*|*/../*|*/..|./*|*/./*|*/.|*//* ) fail "release archive contains an unsafe member: $member" ;;
  esac
  case "$member" in
    "$root_name"|"$root_name/"|"$root_name/bin"|"$root_name/bin/"|\
    "$root_name/bin/tysel"|"$root_name/bin/tysel-service"|"$root_name/bin/tysel-worker"|\
    "$root_name/LICENSE"|"$root_name/README.md"|\
    "$root_name/share"|"$root_name/share/"|"$root_name/share/acceptance"|\
    "$root_name/share/acceptance/"|"$root_name/share/acceptance/"*) ;;
    *) fail "release archive contains unexpected member: $member" ;;
  esac
done < "$members"

extract_dir="$WORK_DIR/extract"
mkdir "$extract_dir"
(ulimit -f 1048576 2>/dev/null || true; tar -xzf "$archive_path" -C "$extract_dir") \
  || fail "cannot extract release archive"
stage_root="$extract_dir/$root_name"
for binary in tysel tysel-service tysel-worker; do
  [ -f "$stage_root/bin/$binary" ] && [ -x "$stage_root/bin/$binary" ] \
    || fail "release is missing executable bin/$binary"
done

"$stage_root/bin/tysel" release verify-installation "$manifest_path" "$stage_root" \
  --target "$TARGET" --version "$VERSION" >/dev/null \
  || fail "release manifest, hashes, or binary identities did not verify"
"$stage_root/bin/tysel" release validate-trust "$trust_path" >/dev/null \
  || fail "release trust policy did not validate"
cp "$manifest_path" "$stage_root/release-manifest.json"

version_dir="$PREFIX/versions/v$VERSION"
if [ -e "$version_dir" ]; then
  "$version_dir/bin/tysel" release verify-installation "$manifest_path" "$version_dir" \
    --target "$TARGET" --version "$VERSION" >/dev/null \
    || fail "existing version directory is not the requested verified release"
else
  mv "$stage_root" "$version_dir"
fi

if [ -L "$PREFIX/bin" ]; then
  OLD_LINK=$(readlink "$PREFIX/bin")
elif [ -e "$PREFIX/bin" ]; then
  fail "$PREFIX/bin exists and is not a managed symbolic link"
fi
if [ -f "$PREFIX/state.json" ]; then
  OLD_STATE="$WORK_DIR/previous-state.json"
  cp "$PREFIX/state.json" "$OLD_STATE"
fi
if [ -f "$PREFIX/trust.json" ]; then
  OLD_TRUST="$WORK_DIR/previous-trust.json"
  cp "$PREFIX/trust.json" "$OLD_TRUST"
fi
previous_version=
case "$OLD_LINK" in
  versions/v*/bin) previous_version=${OLD_LINK#versions/v}; previous_version=${previous_version%/bin} ;;
esac
if [ "$previous_version" = "$VERSION" ] && [ -n "$OLD_STATE" ]; then
  previous_version=$(sed -n 's/.*"previousVersion"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$OLD_STATE" | head -n 1)
fi

replace_link() {
  link_target=$1
  new_link="$PREFIX/.bin-new-$$"
  rm -f "$new_link"
  ln -s "$link_target" "$new_link"
  case "$(uname -s)" in
    Darwin) mv -fh "$new_link" "$PREFIX/bin" ;;
    *) mv -Tf "$new_link" "$PREFIX/bin" ;;
  esac
}

replace_link "versions/v$VERSION/bin"
ACTIVATED=1
trust_tmp="$PREFIX/.trust-new-$$"
cp "$trust_path" "$trust_tmp"
mv -f "$trust_tmp" "$PREFIX/trust.json"
if command -v sha256sum >/dev/null 2>&1; then
  manifest_sha=$(sha256sum "$version_dir/release-manifest.json" | awk '{print $1}')
else
  manifest_sha=$(shasum -a 256 "$version_dir/release-manifest.json" | awk '{print $1}')
fi
state_tmp="$PREFIX/.state-new-$$"
if [ -n "$previous_version" ] && [ "$previous_version" != "$VERSION" ]; then
  previous_json="\"$previous_version\""
else
  previous_json=null
fi
printf '{"schemaVersion":1,"activeVersion":"%s","previousVersion":%s,"channel":"stable","target":"%s","installMethod":"installer","manifestSha256":"%s"}\n' \
  "$VERSION" "$previous_json" "$TARGET" "$manifest_sha" > "$state_tmp"
mv -f "$state_tmp" "$PREFIX/state.json"

PATH="$PREFIX/bin:$PATH" "$PREFIX/bin/tysel" doctor --install --json >/dev/null \
  || fail "post-install doctor rejected the activated toolchain"

path_action="not modified"
if [ "$MODIFY_PATH" -eq 1 ]; then
  case "${SHELL:-}" in
    */zsh) profile=${ZDOTDIR:-${HOME:-}}/.zshrc ;;
    */bash) profile=${HOME:-}/.bashrc ;;
    */sh) profile=${HOME:-}/.profile ;;
    *) profile= ;;
  esac
  if [ -n "$profile" ]; then
    mkdir -p "$(dirname "$profile")"
    touch "$profile"
    escaped_prefix=$(printf '%s' "$PREFIX" | sed 's/[\\"]/\\&/g')
    if grep -Fq '# >>> tysel managed PATH >>>' "$profile"; then
      path_tmp="$profile.tysel.$$"
      awk -v prefix="$escaped_prefix" '
        $0 == "# >>> tysel managed PATH >>>" {
          if (managed) exit 2
          print
          print "export PATH=\"" prefix "/bin:$PATH\""
          managed=1
          skip=1
          next
        }
        $0 == "# <<< tysel managed PATH <<<" && skip {
          print
          skip=0
          closed=1
          next
        }
        !skip { print }
        END { if (managed && !closed) exit 3 }
      ' "$profile" > "$path_tmp" || {
        rm -f "$path_tmp"
        fail "managed PATH block in $profile is malformed"
      }
      mv -f "$path_tmp" "$profile"
      path_action="updated $profile"
    else
      {
        printf '\n# >>> tysel managed PATH >>>\n'
        printf 'export PATH="%s/bin:$PATH"\n' "$escaped_prefix"
        printf '# <<< tysel managed PATH <<<\n'
      } >> "$profile"
      path_action="updated $profile"
    fi
  else
    path_action="not modified (unsupported shell; add $PREFIX/bin manually)"
  fi
fi

ACTIVATED=0
say "Installed Tysel $VERSION ($TARGET) in $version_dir"
say "PATH: $path_action"
say "Next: $PREFIX/bin/tysel init hello-tysel"
