#!/bin/sh
# Tysel developer-toolchain bootstrap installer.
set -eu

PROGRAM=tysel-install
DEFAULT_DOWNLOAD_BASE=https://github.com/wangcch/tysel/releases
CHANNEL=stable
CHANNEL_EXPLICIT=0
CHANNEL_RESOLVED=0
CHANNEL_BASE=
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
PROFILE_TEMP=
PROFILE_PATH=
PROFILE_BACKUP=
PROFILE_EXISTED=0
VERSION_BACKUP=
VERSION_REPLACED=0

say() { printf '%s\n' "$*"; }
warn() { printf '%s: warning: %s\n' "$PROGRAM" "$*" >&2; }
fail() { printf '%s: %s\n' "$PROGRAM" "$*" >&2; exit 1; }

restore_profile() {
  [ -n "$PROFILE_PATH" ] || return 0
  if [ "$PROFILE_EXISTED" -eq 1 ]; then
    [ -f "$PROFILE_BACKUP" ] || return 1
    cp -p "$PROFILE_BACKUP" "$PROFILE_PATH" || return 1
  else
    rm -f "$PROFILE_PATH" || return 1
  fi
  PROFILE_PATH=
  PROFILE_BACKUP=
  PROFILE_EXISTED=0
}

usage() {
  cat <<'EOF'
usage: install.sh [--version <semver>] [--channel stable|canary] [--prefix <absolute-path>]
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
  if [ "$status" -ne 0 ] && [ -n "$PROFILE_PATH" ]; then
    restore_profile 2>/dev/null || true
  fi
  if [ "$status" -ne 0 ] && [ "$VERSION_REPLACED" -eq 1 ]; then
    rm -rf "$version_dir" 2>/dev/null || true
    mv "$VERSION_BACKUP" "$version_dir" 2>/dev/null || true
  fi
  [ -z "$WORK_DIR" ] || rm -rf "$WORK_DIR"
  if [ "$LOCK_ACQUIRED" -eq 1 ]; then
    rm -f "$LOCK_PATH" 2>/dev/null || true
  fi
  [ -z "$LOCK_CANDIDATE" ] || rm -f "$LOCK_CANDIDATE" 2>/dev/null || true
  [ -z "$PROFILE_TEMP" ] || rm -f "$PROFILE_TEMP" 2>/dev/null || true
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
      CHANNEL_EXPLICIT=1
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

case "$CHANNEL" in
  stable|canary) ;;
  *) fail "unsupported channel: $CHANNEL (expected stable or canary)" ;;
esac
VERSION_REQUESTED=$VERSION
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
  candidate=$1
  case "$candidate" in
    *+*) return 1 ;;
  esac
  printf '%s\n' "$candidate" \
    | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z-]+(\.[0-9A-Za-z-]+)*)?$' \
    || return 1
  core=${candidate%%-*}
  old_ifs=$IFS
  IFS=.
  set -- $core
  IFS=$old_ifs
  for identifier in "$@"; do
    case "$identifier" in
      0|[1-9]|[1-9][0-9]*) ;;
      *) return 1 ;;
    esac
  done
  case "$candidate" in
    *-*) prerelease=${candidate#*-} ;;
    *) return 0 ;;
  esac
  old_ifs=$IFS
  IFS=.
  set -- $prerelease
  IFS=$old_ifs
  for identifier in "$@"; do
    case "$identifier" in
      0|[1-9]|[1-9][0-9]*) ;;
      0[0-9]*) return 1 ;;
    esac
  done
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
    VERSION="<${CHANNEL}-version>"
  else
    version_file=$(mktemp "${TMPDIR:-/tmp}/tysel-version.XXXXXX") || fail "cannot create a temporary file"
    if [ "$CHANNEL" = stable ]; then
      CHANNEL_BASE="${DOWNLOAD_BASE%/}/latest/download"
    else
      CHANNEL_BASE="${DOWNLOAD_BASE%/}/download/canary"
    fi
    download "${CHANNEL_BASE}/${CHANNEL}-version" "$version_file"
    [ "$(wc -c < "$version_file" | tr -d '[:space:]')" -le 128 ] \
      || fail "release channel version pointer exceeds the 128-byte limit"
    VERSION=$(tr -d '[:space:]' < "$version_file")
    rm -f "$version_file"
    CHANNEL_RESOLVED=1
  fi
fi
[ "$VERSION" = '<stable-version>' ] || [ "$VERSION" = '<canary-version>' ] \
  || valid_version "$VERSION" || fail "invalid semantic version: $VERSION"

case "$VERSION" in
  '<stable-version>'|'<canary-version>')
  release_base="${DOWNLOAD_BASE%/}/download/v<VERSION>"
  archive="tysel-<VERSION>-${TARGET}.tar.gz"
  ;;
*)
  release_base="${DOWNLOAD_BASE%/}/download/v${VERSION}"
  archive="tysel-${VERSION}-${TARGET}.tar.gz"
  ;;
esac

say "Tysel install plan"
say "  version: $VERSION"
say "  target:  $TARGET"
say "  root:    $PREFIX"
say "  archive: ${release_base}/${archive}"
[ "$DRY_RUN" -eq 0 ] || exit 0

if [ -e "$PREFIX" ]; then
  owner=unknown
  if owner_candidate=$(stat -c '%u' "$PREFIX" 2>/dev/null); then
    owner=$owner_candidate
  elif owner_candidate=$(stat -f '%u' "$PREFIX" 2>/dev/null); then
    owner=$owner_candidate
  fi
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
manifest_signature_path="$WORK_DIR/release-manifest.json.sig.json"
trust_path="$WORK_DIR/trust.json"
trust_signature_path="$WORK_DIR/trust.json.sig.json"
channel_pointer_path="$WORK_DIR/channel-pointer.json"
channel_pointer_signature_path="$WORK_DIR/channel-pointer.json.sig.json"
download "${release_base}/${archive}" "$archive_path"
download "${release_base}/${archive}.sha256" "$checksum_path"
download "${release_base}/release-manifest.json" "$manifest_path"
download "${release_base}/release-manifest.json.sig.json" "$manifest_signature_path"
download "${release_base}/${archive}.sig.json" "${archive_path}.sig.json"
trust_base="${DOWNLOAD_BASE%/}/download/trust"
download "${trust_base}/trust.json" "$trust_path"
download "${trust_base}/trust.json.sig.json" "$trust_signature_path"
if [ "$CHANNEL_RESOLVED" -eq 1 ]; then
  download "${CHANNEL_BASE}/channel-pointer.json" "$channel_pointer_path"
  download "${CHANNEL_BASE}/channel-pointer.json.sig.json" "$channel_pointer_signature_path"
fi

[ "$(wc -c < "$archive_path" | tr -d '[:space:]')" -le 268435456 ] || fail "release archive exceeds the 256 MiB bootstrap limit"
[ "$(wc -c < "$manifest_path" | tr -d '[:space:]')" -le 4194304 ] \
  || fail "release manifest exceeds the 4 MiB limit"
for metadata_path in "$manifest_signature_path" "$trust_path" "$trust_signature_path" \
  "${archive_path}.sig.json"; do
  [ "$(wc -c < "$metadata_path" | tr -d '[:space:]')" -le 1048576 ] \
    || fail "release signature or trust metadata exceeds the 1 MiB limit"
done
if [ "$CHANNEL_RESOLVED" -eq 1 ]; then
  for metadata_path in "$channel_pointer_path" "$channel_pointer_signature_path"; do
    [ "$(wc -c < "$metadata_path" | tr -d '[:space:]')" -le 1048576 ] \
      || fail "release channel metadata exceeds the 1 MiB limit"
  done
fi
[ "$(wc -c < "$checksum_path" | tr -d '[:space:]')" -le 1024 ] \
  || fail "release checksum sidecar exceeds the 1 KiB limit"
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
# POSIX shells may express -f in 512-byte blocks, while bash commonly uses KiB.
# This therefore allows at least 512 MiB per file; the tree limit below is exact.
(ulimit -f 1048576 2>/dev/null || true; tar -xzf "$archive_path" -C "$extract_dir") \
  || fail "cannot extract release archive"
stage_root="$extract_dir/$root_name"
extracted_sizes="$WORK_DIR/extracted-file-sizes"
find "$stage_root" -type f -exec wc -c {} \; > "$extracted_sizes" \
  || fail "cannot measure extracted release"
extracted_size=$(awk '{ total += $1 } END { printf "%.0f\n", total + 0 }' "$extracted_sizes")
[ "$extracted_size" -le 536870912 ] || fail "extracted release exceeds the 512 MiB limit"
for binary in tysel tysel-service tysel-worker; do
  [ -f "$stage_root/bin/$binary" ] && [ -x "$stage_root/bin/$binary" ] \
    || fail "release is missing executable bin/$binary"
done

previous_version=
if [ -L "$PREFIX/bin" ]; then
  OLD_LINK=$(readlink "$PREFIX/bin")
  case "$OLD_LINK" in
    versions/v*/bin)
      previous_version=${OLD_LINK#versions/v}
      previous_version=${previous_version%/bin}
      valid_version "$previous_version" \
        || fail "$PREFIX/bin is not a managed Tysel symbolic link"
      ;;
    *) fail "$PREFIX/bin is not a managed Tysel symbolic link" ;;
  esac
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

# A healthy managed installation authenticates downloaded code before it is
# executed. A fresh or damaged installation retains the documented HTTPS
# bootstrap path so the verified staging tree can still repair it.
verifier="$stage_root/bin/tysel"
if [ -n "$OLD_LINK" ] && [ -f "$PREFIX/trust.json" ] && [ -x "$PREFIX/bin/tysel" ] \
  && "$PREFIX/bin/tysel" release validate-trust "$PREFIX/trust.json" >/dev/null 2>&1; then
  verifier="$PREFIX/bin/tysel"
fi

"$verifier" release validate-trust "$trust_path" >/dev/null \
  || fail "release trust policy did not validate"
if [ -f "$PREFIX/trust.json" ]; then
  "$verifier" release verify-metadata \
    "$trust_path" "$trust_signature_path" --trust "$PREFIX/trust.json" >/dev/null \
    || fail "refreshed trust policy was not signed by an installed trusted key"
  if ! cmp -s "$PREFIX/trust.json" "$trust_path"; then
    "$verifier" release validate-trust-transition \
      "$PREFIX/trust.json" "$trust_path" >/dev/null \
      || fail "release trust policy would move backward or change key identity"
  fi
else
  "$verifier" release verify-metadata \
    "$trust_path" "$trust_signature_path" --trust "$trust_path" >/dev/null \
    || fail "release trust policy signature did not validate"
fi
"$verifier" release verify-metadata \
  "$manifest_path" "$manifest_signature_path" --trust "$trust_path" >/dev/null \
  || fail "release manifest signature did not validate"
"$verifier" release verify-artifact \
  "$archive_path" --trust "$trust_path" --target "$TARGET" >/dev/null \
  || fail "release archive signature did not validate"
if [ "$CHANNEL_RESOLVED" -eq 1 ]; then
  "$verifier" release verify-metadata \
    "$channel_pointer_path" "$channel_pointer_signature_path" --trust "$trust_path" >/dev/null \
    || fail "release channel pointer signature did not validate"
fi

"$stage_root/bin/tysel" release verify-installation "$manifest_path" "$stage_root" \
  --target "$TARGET" --version "$VERSION" >/dev/null \
  || fail "release manifest, hashes, or binary identities did not verify"
if [ "$CHANNEL_RESOLVED" -eq 1 ]; then
  if [ -n "$OLD_STATE" ]; then
    "$stage_root/bin/tysel" release verify-channel-selection \
      "$channel_pointer_path" "$manifest_path" "$manifest_signature_path" \
      --channel "$CHANNEL" --version "$VERSION" --installed-state "$OLD_STATE" >/dev/null \
      || fail "release channel pointer did not select this immutable release"
  else
    "$stage_root/bin/tysel" release verify-channel-selection \
      "$channel_pointer_path" "$manifest_path" "$manifest_signature_path" \
      --channel "$CHANNEL" --version "$VERSION" >/dev/null \
      || fail "release channel pointer did not select this immutable release"
  fi
fi
manifest_channel=$(sed -n \
  's/.*"channel"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$manifest_path" \
  | head -n 1)
case "$manifest_channel" in
  stable|canary) ;;
  *) fail "release manifest has an unsupported channel" ;;
esac
if [ -z "${VERSION_REQUESTED:-}" ] || [ "$CHANNEL_EXPLICIT" -eq 1 ]; then
  [ "$manifest_channel" = "$CHANNEL" ] \
    || fail "release manifest is for $manifest_channel, expected $CHANNEL"
fi
CHANNEL=$manifest_channel
cp "$manifest_path" "$stage_root/release-manifest.json"

if [ "$previous_version" = "$VERSION" ] && [ -n "$OLD_STATE" ]; then
  previous_version=$(sed -n 's/.*"previousVersion"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$OLD_STATE" | head -n 1)
fi

version_dir="$PREFIX/versions/v$VERSION"
if [ -L "$version_dir" ]; then
  fail "managed version path is a symbolic link: $version_dir"
elif [ -e "$version_dir" ]; then
  if "$stage_root/bin/tysel" release verify-installation "$manifest_path" "$version_dir" \
    --target "$TARGET" --version "$VERSION" >/dev/null 2>&1 \
    && cmp -s "$manifest_path" "$version_dir/release-manifest.json"; then
    :
  else
    [ -d "$version_dir" ] || fail "managed version path is not a directory: $version_dir"
    VERSION_BACKUP="$WORK_DIR/existing-version"
    mv "$version_dir" "$VERSION_BACKUP"
    mv "$stage_root" "$version_dir"
    VERSION_REPLACED=1
  fi
else
  mv "$stage_root" "$version_dir"
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
printf '{"schemaVersion":1,"activeVersion":"%s","previousVersion":%s,"channel":"%s","target":"%s","installMethod":"installer","manifestSha256":"%s"}\n' \
  "$VERSION" "$previous_json" "$CHANNEL" "$TARGET" "$manifest_sha" > "$state_tmp"
mv -f "$state_tmp" "$PREFIX/state.json"

doctor_output="$WORK_DIR/post-install-doctor"
if ! PATH="$PREFIX/bin:$PATH" "$PREFIX/bin/tysel" doctor --install --json \
  > "$doctor_output" 2>&1; then
  cat "$doctor_output" >&2
  fail "post-install doctor rejected the activated toolchain"
fi

# The activated toolchain is now verified and committed. Shell startup-file
# configuration is a separate best-effort transaction and must not resurrect a
# damaged version tree if it cannot be updated.
ACTIVATED=0
VERSION_REPLACED=0

update_profile_path() {
  target_profile=$1
  mkdir -p "$(dirname "$target_profile")" || return 1
  PROFILE_PATH=$target_profile
  PROFILE_BACKUP="$WORK_DIR/profile-backup"
  PROFILE_EXISTED=0
  if [ -e "$target_profile" ]; then
    cp -p "$target_profile" "$PROFILE_BACKUP" || {
      PROFILE_PATH=
      PROFILE_BACKUP=
      return 1
    }
    PROFILE_EXISTED=1
  fi
  touch "$target_profile" || return 1
  quoted_bin=$(printf '%s' "$PREFIX/bin" | sed "s/'/'\\\\''/g") || return 1
  path_line="export PATH='${quoted_bin}':\$PATH"
  profile_content="$WORK_DIR/profile-content"
  if grep -Fq '# >>> tysel managed PATH >>>' "$target_profile"; then
    TYSEL_PATH_LINE="$path_line" awk '
      $0 == "# >>> tysel managed PATH >>>" {
        if (managed) exit 2
        print
        print ENVIRON["TYSEL_PATH_LINE"]
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
    ' "$target_profile" > "$profile_content" || return 1
  else
    {
      cat "$target_profile"
      printf '\n# >>> tysel managed PATH >>>\n'
      printf '%s\n' "$path_line"
      printf '# <<< tysel managed PATH <<<\n'
    } > "$profile_content" || return 1
  fi
  PROFILE_TEMP="$target_profile.tysel.$$"
  cp -p "$target_profile" "$PROFILE_TEMP" || return 1
  cat "$profile_content" > "$PROFILE_TEMP" || return 1
  mv -f "$PROFILE_TEMP" "$target_profile" || return 1
  PROFILE_TEMP=
}

path_action="not modified"
if [ "$MODIFY_PATH" -eq 1 ]; then
  case "${SHELL:-}" in
    */zsh) [ -n "${ZDOTDIR:-${HOME:-}}" ] && profile=${ZDOTDIR:-$HOME}/.zshrc || profile= ;;
    */bash)
      if [ -z "${HOME:-}" ]; then
        profile=
      elif [ "$(uname -s 2>/dev/null || true)" = Darwin ]; then
        if [ -e "$HOME/.bash_profile" ] || [ -L "$HOME/.bash_profile" ]; then
          profile=$HOME/.bash_profile
        elif [ -e "$HOME/.bash_login" ] || [ -L "$HOME/.bash_login" ]; then
          profile=$HOME/.bash_login
        else
          profile=$HOME/.profile
        fi
      else
        profile=$HOME/.bashrc
      fi
      ;;
    */sh) [ -n "${HOME:-}" ] && profile=$HOME/.profile || profile= ;;
    *) profile= ;;
  esac
  if [ -n "$profile" ]; then
    if [ -L "$profile" ]; then
      path_action="not modified ($profile is a symbolic link; add $PREFIX/bin manually)"
    else
      if update_profile_path "$profile"; then
        path_action="updated $profile"
        PROFILE_PATH=
        PROFILE_BACKUP=
        PROFILE_EXISTED=0
      else
        if restore_profile; then
          path_action="not modified (could not safely update $profile; add $PREFIX/bin manually)"
          warn "could not safely update $profile; the installed toolchain was kept"
        else
          fail "could not update or restore shell profile $profile"
        fi
      fi
    fi
  else
    path_action="not modified (unsupported shell; add $PREFIX/bin manually)"
  fi
fi

PROFILE_PATH=
say "Installed Tysel $VERSION ($TARGET) in $version_dir"
say "PATH: $path_action"
say "Next: $PREFIX/bin/tysel init hello-tysel"
