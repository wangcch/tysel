#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"

[[ "$(bash "$script_dir/release-channel.sh" 0.3.0)" == stable ]]
[[ "$(bash "$script_dir/release-channel.sh" 0.3.0-alpha.2)" == canary ]]
[[ "$(bash "$script_dir/release-channel.sh" 0.3.0-rc.1)" == canary ]]
[[ "$(bash "$script_dir/npm-dist-tag.sh" 0.3.0)" == latest ]]
[[ "$(bash "$script_dir/npm-dist-tag.sh" 0.3.0-alpha.2)" == canary ]]
[[ "$(bash "$script_dir/semver-precedence.sh" 1.0.0 1.0.0-rc.9)" == newer ]]
[[ "$(bash "$script_dir/semver-precedence.sh" 1.0.0-alpha.10 1.0.0-alpha.2)" == newer ]]
[[ "$(bash "$script_dir/semver-precedence.sh" 1.0.0-alpha 1.0.0-alpha.1)" == older ]]
[[ "$(bash "$script_dir/semver-precedence.sh" 1.0.0-1 1.0.0-alpha)" == older ]]
[[ "$(bash "$script_dir/semver-precedence.sh" 99999999999999999999.0.0 2.0.0)" == newer ]]
[[ "$(bash "$script_dir/semver-precedence.sh" 1.2.3 1.2.3)" == equal ]]

for invalid in v0.3.0 01.2.3 1.02.3 1.2.03 1.2.3-01 1.2.3+build 1.2.3-; do
  if bash "$script_dir/release-channel.sh" "$invalid" >/dev/null 2>&1; then
    echo "accepted invalid release version: $invalid" >&2
    exit 1
  fi
done

echo "release channel policy tests passed"
