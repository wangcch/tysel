#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <version> <target> <release-output>" >&2
  exit 2
fi

version="$1"
target="$2"
release_output="$3"
channel="$(bash "$(dirname "$0")/release-channel.sh" "$version")"
if [[ "$channel" == stable ]]; then
  alternate_channel=canary
else
  alternate_channel=stable
fi
archive="tysel-${version}-${target}.tar.gz"
metadata="${release_output}/release-asset-${target}.json"
[[ -f "${release_output}/${archive}" && -f "${release_output}/${archive}.sha256" && -f "$metadata" ]] || {
  echo "installer fixture requires a packaged release target" >&2
  exit 1
}

fixture="$(mktemp -d)"
if [[ "$channel" == stable ]]; then
  channel_directory="${fixture}/server/latest/download"
else
  channel_directory="${fixture}/server/download/canary"
fi
server_pid=
cleanup() {
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  rm -rf "$fixture"
}
trap cleanup EXIT

mkdir -p "${fixture}/fake-bin"
printf '%s\n' '#!/bin/sh' 'if [ "$1" = "-s" ]; then printf "%s\\n" "$TEST_UNAME_S"; else printf "%s\\n" "$TEST_UNAME_M"; fi' \
  > "${fixture}/fake-bin/uname"
chmod +x "${fixture}/fake-bin/uname"
for mapping in 'Linux x86_64 linux-x64' 'Linux aarch64 linux-arm64' \
  'Darwin x86_64 darwin-x64' 'Darwin arm64 darwin-arm64'; do
  set -- $mapping
  output="$(env PATH="${fixture}/fake-bin:$PATH" TEST_UNAME_S="$1" TEST_UNAME_M="$2" \
    sh install.sh --version "$version" --prefix "${fixture}/dry root" --dry-run)"
  grep -F "target:  $3" <<< "$output" >/dev/null
done
canary_plan="$(sh install.sh --channel canary --prefix "${fixture}/canary" --dry-run)"
grep -F 'version: <canary-version>' <<< "$canary_plan" >/dev/null
for invalid in 01.2.3 1.02.3 1.2.03 1.2.3-01 1.2.3+build; do
  if sh install.sh --version "$invalid" --prefix "${fixture}/invalid" --dry-run \
    >/dev/null 2>&1; then
    echo "installer accepted invalid release version: $invalid" >&2
    exit 1
  fi
done
if sh install.sh --version "$version" --prefix relative --dry-run >/dev/null 2>&1; then
  echo "installer accepted a relative root" >&2
  exit 1
fi
if env -u HOME -u TYSEL_HOME sh install.sh --version "$version" --dry-run \
  >"${fixture}/missing-home.log" 2>&1; then
  echo "installer accepted a missing HOME without an explicit prefix" >&2
  exit 1
fi
grep -F 'HOME is not set' "${fixture}/missing-home.log" >/dev/null

release_dir="${fixture}/server/download/v${version}"
trust_dir="${fixture}/server/download/trust"
mkdir -p "$release_dir" "$trust_dir" "$channel_directory" "${fixture}/home"
cp "${release_output}/${archive}" "${release_output}/${archive}.sha256" "$release_dir/"
printf '%s\n' "$version" > "${channel_directory}/${channel}-version"
printf '%064d\n' 7 > "${fixture}/fixture.key"
chmod 600 "${fixture}/fixture.key"
key_info="$(target/repro-1/release/tysel release key-info --key "${fixture}/fixture.key")"
target/repro-1/release/tysel release sign-artifact "$release_dir/$archive" \
  --target "$target" --key "${fixture}/fixture.key" >/dev/null
now_unix="$(date +%s)"
issued_at_unix="$((now_unix - 5))"
expires_at_unix="$((issued_at_unix + 7776000))"
jq -n --argjson key "$key_info" --argjson issued "$issued_at_unix" --argjson expires "$expires_at_unix" \
  '{policy_version: 1, issued_at_unix: $issued, expires_at_unix: $expires,
    keys: [{key_id: $key.key_id, algorithm: $key.algorithm,
      public_key: $key.public_key, status: "active", valid_from_unix: $issued}]}' \
  > "$release_dir/trust.json"

jq -s \
  --arg version "$version" \
  --arg channel "$channel" \
  --arg target "$target" \
  --arg archiveUrl "https://fixture.invalid/${archive}" \
  --arg sha256 "$(awk 'NR == 1 { print $1 }' "${release_output}/${archive}.sha256")" \
  --argjson byteSize "$(wc -c < "${release_output}/${archive}" | tr -d '[:space:]')" \
  '.[0] + {
    archiveUrl: $archiveUrl,
    byteSize: $byteSize,
    sha256: $sha256,
    signature: {algorithm: "ed25519", url: "https://fixture.invalid/archive.sig.json", keyId: ("ab" * 32)}
  } | {
    schemaVersion: 1,
    version: $version,
    sourceCommit: .buildInfo[0].sourceCommit,
    publishedAt: "2026-08-22T00:00:00Z",
    channel: $channel,
    minimumUpdaterVersion: "0.0.1",
    compatibility: {minimumTapVersion: 1, maximumTapVersion: 3, capabilityAbiVersion: "0.4.0", typesVersion: $version},
    requiredFeatures: ["atomic-bin-link", "build-info-v1", "ed25519-manifest"],
    optionalFeatures: {},
    assets: [.]
  }' "$metadata" > "$release_dir/release-manifest.json"
target/repro-1/release/tysel release sign-metadata "$release_dir/release-manifest.json" \
  --key "${fixture}/fixture.key" >/dev/null
manifest_sha256="$(shasum -a 256 "$release_dir/release-manifest.json" | awk '{print $1}')"
manifest_size="$(wc -c < "$release_dir/release-manifest.json" | tr -d '[:space:]')"
manifest_key_id="$(jq -er '.key_id' "$release_dir/release-manifest.json.sig.json")"
jq -n \
  --arg version "$version" \
  --arg channel "$channel" \
  --argjson manifestSize "$manifest_size" \
  --arg manifestSha256 "$manifest_sha256" \
  --arg keyId "$manifest_key_id" \
  '{schemaVersion: 1, channel: $channel, version: $version,
    publishedAt: "2026-08-22T00:00:00Z",
    manifestUrl: "https://fixture.invalid/release-manifest.json",
    manifestByteSize: $manifestSize, manifestSha256: $manifestSha256,
    manifestSignature: {algorithm: "ed25519",
      url: "https://fixture.invalid/release-manifest.json.sig.json", keyId: $keyId},
    requiredFeatures: ["atomic-bin-link", "build-info-v1", "ed25519-manifest"]}' \
  > "${channel_directory}/channel-pointer.json"
target/repro-1/release/tysel release sign-metadata \
  "${channel_directory}/channel-pointer.json" \
  --key "${fixture}/fixture.key" >/dev/null
target/repro-1/release/tysel release sign-metadata "$release_dir/trust.json" \
  --key "${fixture}/fixture.key" >/dev/null
cp "$release_dir/trust.json" "$release_dir/trust.json.sig.json" \
  "$trust_dir/"

port=$((20000 + ($$ % 20000)))
python3 -m http.server "$port" --bind 127.0.0.1 --directory "${fixture}/server" \
  > "${fixture}/server.log" 2>&1 &
server_pid=$!
for _ in {1..50}; do
  curl -fsS "http://127.0.0.1:${port}/${channel_directory#${fixture}/server/}/${channel}-version" \
    >/dev/null 2>&1 && break
  sleep 0.1
done
curl -fsS "http://127.0.0.1:${port}/${channel_directory#${fixture}/server/}/${channel}-version" \
  >/dev/null

# A moving channel install fails closed when its signed pointer is corrupted.
cp "${channel_directory}/channel-pointer.json.sig.json" \
  "${fixture}/channel-pointer.signature"
printf '%s\n' '{"invalid":true}' \
  > "${channel_directory}/channel-pointer.json.sig.json"
if env HOME="${fixture}/home" SHELL=/bin/sh \
  TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  sh install.sh --channel "$channel" --prefix "${fixture}/home/corrupt-pointer" \
  --no-modify-path \
  >/dev/null 2>&1; then
  echo "installer accepted a corrupted channel pointer signature" >&2
  exit 1
fi
mv "${fixture}/channel-pointer.signature" \
  "${channel_directory}/channel-pointer.json.sig.json"

install_root="${fixture}/home/root with spaces"
env HOME="${fixture}/home" SHELL=/bin/sh \
  TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  sh install.sh --channel "$channel" --prefix "$install_root" --no-modify-path
"${install_root}/bin/tysel" --version | grep -Fx "tysel ${version}"
[[ "$(readlink "${install_root}/bin")" == "versions/v${version}/bin" ]]
jq -e --arg version "$version" --arg target "$target" --arg channel "$channel" \
  '.schemaVersion == 1 and .activeVersion == $version and .target == $target
    and .channel == $channel' \
  "${install_root}/state.json" >/dev/null

# Replaying an older signed pointer cannot downgrade an existing same-channel install.
if [[ "$channel" == stable ]]; then
  newer_version=9999.0.0
else
  newer_version=9999.0.0-canary.1
fi
cp "${install_root}/state.json" "${fixture}/installed-state.json"
jq --arg version "$newer_version" \
  '.activeVersion = $version | .previousVersion = null' \
  "${fixture}/installed-state.json" > "${install_root}/state.json"
if env HOME="${fixture}/home" SHELL=/bin/sh \
  TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  sh install.sh --channel "$channel" --prefix "$install_root" --no-modify-path \
  >/dev/null 2>&1; then
  echo "installer accepted a same-channel downgrade from a signed stale pointer" >&2
  exit 1
fi
mv "${fixture}/installed-state.json" "${install_root}/state.json"
TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  "${install_root}/bin/tysel" upgrade --check --version "$version" --json \
  | jq -e '.schemaVersion == 1 and .changed == false and .toVersion == $version' \
      --arg version "$version" >/dev/null

# A same-version trust refresh is an explicit, truthfully reported mutation.
refreshed_issued="$(date +%s)"
refreshed_expires="$((refreshed_issued + 7776000))"
jq -n --argjson key "$key_info" --argjson issued "$refreshed_issued" \
  --argjson expires "$refreshed_expires" --argjson validFrom "$issued_at_unix" \
  '{policy_version: 1, issued_at_unix: $issued, expires_at_unix: $expires,
    keys: [{key_id: $key.key_id, algorithm: $key.algorithm,
      public_key: $key.public_key, status: "active", valid_from_unix: $validFrom}]}' \
  > "$trust_dir/trust.json"
target/repro-1/release/tysel release sign-metadata \
  "$trust_dir/trust.json" --key "${fixture}/fixture.key" >/dev/null
TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  "${install_root}/bin/tysel" upgrade --yes --version "$version" --json \
  | jq -e '.schemaVersion == 1 and .action == "trust-refresh"
      and .changed == true and .fromVersion == $version and .toVersion == $version' \
      --arg version "$version" >/dev/null
cmp "$trust_dir/trust.json" "${install_root}/trust.json"

# Publish byte-different, signed metadata for the same fixture version. Public
# releases are immutable; this exercises custom mirrors and local recovery.
jq '.publishedAt = "2026-08-22T00:00:01Z"' \
  "$release_dir/release-manifest.json" > "${fixture}/reissued-manifest.json"
mv "${fixture}/reissued-manifest.json" "$release_dir/release-manifest.json"
target/repro-1/release/tysel release sign-metadata \
  "$release_dir/release-manifest.json" --key "${fixture}/fixture.key" >/dev/null

golden_project="${fixture}/golden-project"
env -i HOME="${fixture}/home" TMPDIR="${TMPDIR:-/tmp}" \
  PATH="${install_root}/bin:/usr/bin:/bin" \
  tysel init "$golden_project"
(cd "$golden_project" && env -i HOME="${fixture}/home" TMPDIR="${TMPDIR:-/tmp}" \
  PATH="${install_root}/bin:/usr/bin:/bin" tysel check)
(cd "$golden_project" && env -i HOME="${fixture}/home" TMPDIR="${TMPDIR:-/tmp}" \
  PATH="${install_root}/bin:/usr/bin:/bin" tysel test)

# A repeat install adopts the authenticated metadata and leaves the same release active.
env HOME="${fixture}/home" SHELL=/bin/sh \
  TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  sh install.sh --version "$version" --prefix "$install_root"
[[ "$(readlink "${install_root}/bin")" == "versions/v${version}/bin" ]]
[[ "$(grep -Fc '# >>> tysel managed PATH >>>' "${fixture}/home/.profile")" == 1 ]]
cmp "$release_dir/release-manifest.json" \
  "${install_root}/versions/v${version}/release-manifest.json"
installed_manifest_sha="$(shasum -a 256 \
  "${install_root}/versions/v${version}/release-manifest.json" | awk '{print $1}')"
jq -e --arg sha "$installed_manifest_sha" '.manifestSha256 == $sha' \
  "${install_root}/state.json" >/dev/null

# Reinstalling replaces a locally drifted manifest instead of blessing its hash
# merely because the binaries still match the authenticated release manifest.
printf '\n' >> "${install_root}/versions/v${version}/release-manifest.json"
if cmp -s "$release_dir/release-manifest.json" \
  "${install_root}/versions/v${version}/release-manifest.json"; then
  echo "installer fixture failed to create manifest drift" >&2
  exit 1
fi
env HOME="${fixture}/home" SHELL=/bin/sh \
  TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  sh install.sh --version "$version" --prefix "$install_root" --no-modify-path
cmp "$release_dir/release-manifest.json" \
  "${install_root}/versions/v${version}/release-manifest.json"
installed_manifest_sha="$(shasum -a 256 \
  "${install_root}/versions/v${version}/release-manifest.json" | awk '{print $1}')"
jq -e --arg sha "$installed_manifest_sha" '.manifestSha256 == $sha' \
  "${install_root}/state.json" >/dev/null

# A verified staging tree repairs a damaged managed version directory.
printf '%s\n' damaged > "${install_root}/versions/v${version}/bin/tysel"
chmod +x "${install_root}/versions/v${version}/bin/tysel"
env HOME="${fixture}/home" SHELL=/bin/sh \
  TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  sh install.sh --version "$version" --prefix "$install_root" --no-modify-path
repaired_sha="$(shasum -a 256 "${install_root}/versions/v${version}/bin/tysel" | awk '{print $1}')"
expected_binary_sha="$(jq -er '.files[] | select(.path == "bin/tysel") | .sha256' "$metadata")"
[[ "$repaired_sha" == "$expected_binary_sha" ]]

# A shell-profile failure after doctor keeps the repaired tree and restores the profile.
printf '%s\n' damaged-again > "${install_root}/versions/v${version}/bin/tysel"
chmod +x "${install_root}/versions/v${version}/bin/tysel"
malformed_home="${fixture}/malformed-home"
mkdir -p "$malformed_home"
printf '%s\n' \
  '# existing configuration' \
  '# >>> tysel managed PATH >>>' \
  'export PATH=/broken:$PATH' \
  > "${malformed_home}/.profile"
cp "${malformed_home}/.profile" "${fixture}/malformed-profile.before"
path_failure_output="$(env HOME="$malformed_home" SHELL=/bin/sh \
  TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  sh install.sh --version "$version" --prefix "$install_root" 2>&1)"
grep -F 'warning: could not safely update' <<< "$path_failure_output" >/dev/null
grep -F 'PATH: not modified' <<< "$path_failure_output" >/dev/null
cmp "${fixture}/malformed-profile.before" "${malformed_home}/.profile"
repaired_sha="$(shasum -a 256 "${install_root}/versions/v${version}/bin/tysel" | awk '{print $1}')"
[[ "$repaired_sha" == "$expected_binary_sha" ]]

# On Darwin, creating .bash_profile must not shadow an existing .profile.
if [[ "$(uname -s)" == Darwin ]]; then
  darwin_bash_home="${fixture}/darwin-bash-home"
  darwin_bash_root="${fixture}/darwin-bash-root"
  mkdir -p "$darwin_bash_home"
  printf '%s\n' '# existing login-shell configuration' > "${darwin_bash_home}/.profile"
  env HOME="$darwin_bash_home" SHELL=/bin/bash \
    TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
    sh install.sh --version "$version" --prefix "$darwin_bash_root"
  [[ ! -e "${darwin_bash_home}/.bash_profile" ]]
  [[ ! -e "${darwin_bash_home}/.bash_login" ]]
  grep -Fx '# existing login-shell configuration' "${darwin_bash_home}/.profile" >/dev/null
  [[ "$(grep -Fc '# >>> tysel managed PATH >>>' "${darwin_bash_home}/.profile")" == 1 ]]
fi

# An explicit channel must agree with the immutable manifest.
channel_mismatch_root="${fixture}/home/channel-mismatch"
if env HOME="${fixture}/home" SHELL=/bin/sh \
  TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  sh install.sh --version "$version" --channel "$alternate_channel" \
    --prefix "$channel_mismatch_root" --no-modify-path; then
  echo "installer accepted a ${channel} manifest for the ${alternate_channel} channel" >&2
  exit 1
fi
[[ ! -L "${channel_mismatch_root}/bin" ]]

# A custom prefix remains safe when HOME is absent and PATH modification is requested.
homeless_root="${fixture}/homeless-root"
homeless_output="$(env -u HOME SHELL=/bin/sh \
  TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  sh install.sh --version "$version" --prefix "$homeless_root")"
grep -F 'PATH: not modified' <<< "$homeless_output" >/dev/null
[[ "$(jq -r '.channel' "${homeless_root}/state.json")" == "$channel" ]]

# Shell metacharacters in an absolute install root remain literal PATH data.
dangerous_root="${fixture}/home/root '\$(touch PWNED)'"
env HOME="${fixture}/home" SHELL=/bin/sh \
  TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  sh install.sh --version "$version" --prefix "$dangerous_root"
(
  cd "$fixture"
  PATH=/usr/bin:/bin
  export PATH
  . "${fixture}/home/.profile"
  printf '%s\n' "$PATH" > sourced-path
)
[[ ! -e "${fixture}/PWNED" ]]
grep -F "${dangerous_root}/bin:" "${fixture}/sourced-path" >/dev/null

# Linked shell profiles are never replaced or edited behind the link.
linked_home="${fixture}/linked-home"
mkdir -p "${linked_home}/dotfiles"
printf '%s\n' '# externally managed profile' > "${linked_home}/dotfiles/profile"
ln -s dotfiles/profile "${linked_home}/.profile"
env HOME="$linked_home" SHELL=/bin/sh \
  TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  sh install.sh --version "$version" --prefix "$install_root"
[[ -L "${linked_home}/.profile" ]]
[[ "$(readlink "${linked_home}/.profile")" == dotfiles/profile ]]
grep -Fx '# externally managed profile' "${linked_home}/dotfiles/profile" >/dev/null
if grep -Fq '# >>> tysel managed PATH >>>' "${linked_home}/dotfiles/profile"; then
  echo "installer modified a linked shell profile" >&2
  exit 1
fi

# A custom root cannot cause an unrelated bin link to be overwritten.
unmanaged_root="${fixture}/home/unmanaged-root"
mkdir -p "$unmanaged_root"
ln -s "${fixture}/unrelated-bin" "${unmanaged_root}/bin"
if env HOME="${fixture}/home" SHELL=/bin/sh \
  TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  sh install.sh --version "$version" --prefix "$unmanaged_root" --no-modify-path; then
  echo "installer overwrote an unmanaged bin symbolic link" >&2
  exit 1
fi
[[ "$(readlink "${unmanaged_root}/bin")" == "${fixture}/unrelated-bin" ]]

# Reinstalling under a new root updates the existing managed PATH block.
alternate_root="${fixture}/home/alternate root"
env HOME="${fixture}/home" SHELL=/bin/sh \
  TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  sh install.sh --version "$version" --prefix "$alternate_root"
grep -F "export PATH='${alternate_root}/bin':\$PATH" "${fixture}/home/.profile" >/dev/null
if grep -F "export PATH='${install_root}/bin':\$PATH" "${fixture}/home/.profile" >/dev/null; then
  echo "installer left the previous managed root in PATH" >&2
  exit 1
fi

# Bootstrap corruption must fail before activation and preserve the working link/state.
cp "${install_root}/state.json" "${fixture}/state.before"
printf '%064d\n' 0 > "${release_dir}/${archive}.sha256"
if env HOME="${fixture}/home" SHELL=/bin/sh \
  TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  sh install.sh --version "$version" --prefix "$install_root" --no-modify-path; then
  echo "installer accepted a corrupt checksum" >&2
  exit 1
fi
cmp "${fixture}/state.before" "${install_root}/state.json"
[[ "$(readlink "${install_root}/bin")" == "versions/v${version}/bin" ]]
"${install_root}/bin/tysel" --version | grep -Fx "tysel ${version}"

# A correctly checksummed traversal archive is rejected by member policy.
python3 -c 'import io,sys,tarfile
p,n=sys.argv[1:]
with tarfile.open(p,"w:gz") as archive:
    entry=tarfile.TarInfo(n + "/share/acceptance/../../escape")
    entry.size=4
    archive.addfile(entry,io.BytesIO(b"nope"))' \
  "${release_dir}/${archive}" "tysel-${version}-${target}"
shasum -a 256 "${release_dir}/${archive}" | awk '{print $1}' \
  > "${release_dir}/${archive}.sha256"
if env HOME="${fixture}/home" SHELL=/bin/sh \
  TYSEL_DOWNLOAD_BASE="http://127.0.0.1:${port}" \
  sh install.sh --version "$version" --prefix "$install_root" --no-modify-path; then
  echo "installer accepted an archive traversal" >&2
  exit 1
fi
cmp "${fixture}/state.before" "${install_root}/state.json"
[[ "$(readlink "${install_root}/bin")" == "versions/v${version}/bin" ]]

echo "installer fixture passed for ${target}"
