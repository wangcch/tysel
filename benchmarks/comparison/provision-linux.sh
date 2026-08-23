#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "comparison provisioning supports Linux only" >&2
  exit 1
fi

for command in curl jq sha256sum tar unzip; do
  if ! command -v "${command}" >/dev/null 2>&1; then
    echo "missing required command: ${command}" >&2
    exit 1
  fi
done

case "$(uname -m)" in
  x86_64)
    comparison_arch="x86_64"
    node_arch="x64"
    bun_arch="x64"
    node_sha256="14b342e71204f811bde6153be8e04b62aef63c236fef92b55f9c83154b409647"
    bun_asset="bun-linux-x64.zip"
    bun_sha256="2d03fb5fb83ac8b567aca0a281b2ce1a1a19d488f56c2968d88c3f25e92fe452"
    deno_asset="deno-x86_64-unknown-linux-gnu.zip"
    deno_sha256="8b010a3b1a4a0188a67cdb8a7a27348b2a501af78aec7fc74f2ace167368d530"
    ;;
  aarch64|arm64)
    comparison_arch="aarch64"
    node_arch="arm64"
    bun_arch="aarch64"
    node_sha256="01443c1e1a29e531ccad5a46fefa6df490d2189c49f7955904aecdbb0fe86fdc"
    bun_asset="bun-linux-aarch64.zip"
    bun_sha256="4b1a332ee861983eb93bcfe6f770fff94e3e31b2c388bdaea3c8ed35e58eed0e"
    deno_asset="deno-aarch64-unknown-linux-gnu.zip"
    deno_sha256="6b7cae3a8fc4385a59dea3146fcb8bad7fea4230e0ad36a8c692afacbc254be0"
    ;;
  *)
    echo "unsupported Linux architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

node_version="24.19.0"
bun_version="1.4.0"
deno_version="2.9.5"
workspace_root="$(git rev-parse --show-toplevel)"
runtime_lock="${workspace_root}/benchmarks/comparison/runtimes.lock.json"
[[ "$(jq -r '.runtimes[] | select(.id == "node") | .expectedVersion' "${runtime_lock}")" == "${node_version}" ]]
[[ "$(jq -r '.runtimes[] | select(.id == "bun") | .expectedVersion' "${runtime_lock}")" == "${bun_version}" ]]
[[ "$(jq -r '.runtimes[] | select(.id == "deno") | .expectedVersion' "${runtime_lock}")" == "${deno_version}" ]]
tool_root="${workspace_root}/target/benchmark-comparison/tools/${comparison_arch}"
download_root="${tool_root}/downloads"
bin_root="${tool_root}/bin"
mkdir -p "${download_root}" "${bin_root}"

fetch_verified() {
  local url="$1"
  local destination="$2"
  local expected_sha256="$3"
  if [[ -f "${destination}" ]] && printf '%s  %s\n' "${expected_sha256}" "${destination}" | sha256sum --check --status; then
    return
  fi
  curl --fail --location --retry 3 --output "${destination}" "${url}"
  printf '%s  %s\n' "${expected_sha256}" "${destination}" | sha256sum --check --status
}

node_asset="node-v${node_version}-linux-${node_arch}.tar.xz"
node_archive="${download_root}/${node_asset}"
bun_archive="${download_root}/${bun_asset}"
deno_archive="${download_root}/${deno_asset}"

fetch_verified "https://nodejs.org/download/release/v${node_version}/${node_asset}" "${node_archive}" "${node_sha256}"
fetch_verified "https://github.com/oven-sh/bun/releases/download/bun-v${bun_version}/${bun_asset}" "${bun_archive}" "${bun_sha256}"
fetch_verified "https://github.com/denoland/deno/releases/download/v${deno_version}/${deno_asset}" "${deno_archive}" "${deno_sha256}"

extract_root="$(mktemp -d)"
trap 'rm -rf "${extract_root}"' EXIT
tar -xJf "${node_archive}" -C "${extract_root}"
unzip -q "${bun_archive}" -d "${extract_root}/bun"
unzip -q "${deno_archive}" -d "${extract_root}/deno"

install -m 0755 "${extract_root}/node-v${node_version}-linux-${node_arch}/bin/node" "${bin_root}/node"
install -m 0755 "${extract_root}/bun/bun-linux-${bun_arch}/bun" "${bin_root}/bun"
install -m 0755 "${extract_root}/deno/deno" "${bin_root}/deno"

PATH="${bin_root}:${PATH}"
export PATH
[[ "$(node --version)" == "v${node_version}" ]]
[[ "$(bun --version)" == "${bun_version}" ]]
[[ "$(deno --version | head -n 1)" == "deno ${deno_version}"* ]]

echo "provisioned Node ${node_version}, Bun ${bun_version}, and Deno ${deno_version} for ${comparison_arch}" >&2
printf '%s\n' "${bin_root}"
