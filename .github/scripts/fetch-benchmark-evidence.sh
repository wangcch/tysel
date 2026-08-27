#!/usr/bin/env bash
# Download the latest linux-x64 admission evidence artifact from main-branch CI
# and write a website snapshot consumed by /benchmarks.
#
# Usage:
#   bash .github/scripts/fetch-benchmark-evidence.sh [output-json]
#
# Environment:
#   GITHUB_REPOSITORY  owner/repo (default: wangcch/tysel)
#   GH_TOKEN / GITHUB_TOKEN  required for the Actions API
#   MAX_EVIDENCE_AGE_DAYS  reject older successful runs (default: 30)
#   BENCHMARK_WORKFLOW_RUN_ID  optional exact CI run to consume
#   BENCHMARK_SOURCE_COMMIT  optional exact source commit to require
#
# Failure mode: writes {"status":"unpublished"} and exits 0 so website builds
# still succeed when evidence is missing or invalid. Never keeps a stale
# published snapshot from the checkout.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="${1:-$ROOT/website/data/benchmarks/admission-linux-x64.json}"
REPO="${GITHUB_REPOSITORY:-wangcch/tysel}"
TOKEN="${GH_TOKEN:-${GITHUB_TOKEN:-}}"
ARTIFACT_NAME="benchmark-evidence-linux-x64"
EVIDENCE_FILE="benchmark-evidence-v2-linux-x64.json"
WORKFLOW_PATH=".github/workflows/ci.yml"
MAX_AGE_DAYS="${MAX_EVIDENCE_AGE_DAYS:-30}"
EXPECTED_RUN_ID="${BENCHMARK_WORKFLOW_RUN_ID:-}"
EXPECTED_SOURCE_COMMIT="${BENCHMARK_SOURCE_COMMIT:-}"
API="https://api.github.com"

write_unpublished() {
  local reason="${1:-unpublished}"
  echo "fetch-benchmark-evidence: $reason → writing unpublished snapshot" >&2
  printf '%s\n' '{"status":"unpublished"}' >"$OUT"
}

if [ -z "$TOKEN" ]; then
  echo "fetch-benchmark-evidence: GH_TOKEN or GITHUB_TOKEN is required" >&2
  exit 1
fi
if ! [[ "$MAX_AGE_DAYS" =~ ^[1-9][0-9]*$ ]]; then
  echo "fetch-benchmark-evidence: MAX_EVIDENCE_AGE_DAYS must be a positive integer" >&2
  exit 1
fi
if [ -n "$EXPECTED_RUN_ID" ] && ! [[ "$EXPECTED_RUN_ID" =~ ^[1-9][0-9]*$ ]]; then
  echo "fetch-benchmark-evidence: BENCHMARK_WORKFLOW_RUN_ID must be numeric" >&2
  exit 1
fi
if [ -n "$EXPECTED_SOURCE_COMMIT" ] \
  && ! [[ "$EXPECTED_SOURCE_COMMIT" =~ ^[0-9a-f]{40}$ ]]; then
  echo "fetch-benchmark-evidence: BENCHMARK_SOURCE_COMMIT must be a 40-character commit SHA" >&2
  exit 1
fi

mkdir -p "$(dirname "$OUT")"
tmpdir="$(mktemp -d)"
trap 'rm -rf "$tmpdir"' EXIT

auth() {
  curl --fail --silent --show-error \
    -H "Authorization: Bearer $TOKEN" \
    -H "Accept: application/vnd.github+json" \
    -H "X-GitHub-Api-Version: 2022-11-28" \
    "$@"
}

echo "fetch-benchmark-evidence: listing artifacts for $REPO ($ARTIFACT_NAME)"
page=1
artifact_id=""
workflow_run_id=""
workflow_head_sha=""
workflow_created_at=""
while [ "$page" -le 5 ]; do
  if [ -n "$EXPECTED_RUN_ID" ]; then
    if ! response="$(auth "$API/repos/$REPO/actions/runs/$EXPECTED_RUN_ID/artifacts?name=$ARTIFACT_NAME&per_page=100")"; then
      write_unpublished "artifact listing failed"
      exit 0
    fi
    candidates="$(printf '%s' "$response" | jq -r --arg run_id "$EXPECTED_RUN_ID" '
      .artifacts | sort_by(.created_at) | reverse[]
      | select(.expired == false and .name == "benchmark-evidence-linux-x64")
      | "\(.id) \($run_id)"
    ')"
  else
    if ! response="$(auth "$API/repos/$REPO/actions/artifacts?name=$ARTIFACT_NAME&per_page=30&page=$page")"; then
      write_unpublished "artifact listing failed"
      exit 0
    fi
    candidates="$(printf '%s' "$response" | jq -r '
      .artifacts | sort_by(.created_at) | reverse[]
      | select(.expired == false and .workflow_run.id != null)
      | "\(.id) \(.workflow_run.id)"
    ')"
  fi
  if [ -z "$candidates" ]; then
    total="$(printf '%s' "$response" | jq -r '.total_count // 0')"
    if [ "$total" -eq 0 ]; then
      break
    fi
    if [ -n "$EXPECTED_RUN_ID" ]; then
      break
    fi
    max_page=$(( (total + 29) / 30 ))
    if [ "$page" -ge "$max_page" ]; then
      break
    fi
    page=$((page + 1))
    continue
  fi

  while IFS= read -r candidate; do
    [ -z "$candidate" ] && continue
    aid="${candidate%% *}"
    rid="${candidate#* }"
    if ! run_json="$(auth "$API/repos/$REPO/actions/runs/$rid")"; then
      write_unpublished "workflow metadata lookup failed"
      exit 0
    fi
    conclusion="$(printf '%s' "$run_json" | jq -r '.conclusion // empty')"
    head_branch="$(printf '%s' "$run_json" | jq -r '.head_branch // empty')"
    head_sha="$(printf '%s' "$run_json" | jq -r '.head_sha // empty')"
    event_name="$(printf '%s' "$run_json" | jq -r '.event // empty')"
    workflow_path="$(printf '%s' "$run_json" | jq -r '.path // empty')"
    created_at="$(printf '%s' "$run_json" | jq -r '.created_at // empty')"
    fresh="$(printf '%s' "$run_json" | jq -r --argjson max_age_seconds "$((MAX_AGE_DAYS * 86400))" '
      try ((.created_at | fromdateiso8601) >= (now - $max_age_seconds)) catch false
    ')"
    source_matches="true"
    if [ -n "$EXPECTED_SOURCE_COMMIT" ] \
      && [ "$head_sha" != "$EXPECTED_SOURCE_COMMIT" ]; then
      source_matches="false"
    fi
    # Publish only fresh evidence from the canonical successful main push workflow.
    if [ "$conclusion" = "success" ] && [ "$head_branch" = "main" ] \
      && [ "$event_name" = "push" ] && [ "$workflow_path" = "$WORKFLOW_PATH" ] \
      && [[ "$head_sha" =~ ^[0-9a-f]{40}$ ]] && [ "$fresh" = "true" ] \
      && [ "$source_matches" = "true" ]; then
      artifact_id="$aid"
      workflow_run_id="$rid"
      workflow_head_sha="$head_sha"
      workflow_created_at="$created_at"
      break
    fi
  done <<EOF
$candidates
EOF

  if [ -n "$artifact_id" ]; then
    break
  fi
  if [ -n "$EXPECTED_RUN_ID" ]; then
    break
  fi
  page=$((page + 1))
done

if [ -z "$artifact_id" ]; then
  write_unpublished "no successful main-branch artifact found"
  exit 0
fi

echo "fetch-benchmark-evidence: downloading artifact $artifact_id (run $workflow_run_id)"
if ! auth -L "$API/repos/$REPO/actions/artifacts/$artifact_id/zip" \
  --output "$tmpdir/artifact.zip"; then
  write_unpublished "artifact download failed"
  exit 0
fi

if ! unzip -q "$tmpdir/artifact.zip" -d "$tmpdir/artifact"; then
  write_unpublished "artifact unzip failed"
  exit 0
fi

if [ ! -f "$tmpdir/artifact/$EVIDENCE_FILE" ]; then
  write_unpublished "missing $EVIDENCE_FILE in artifact"
  exit 0
fi

if ! jq -e --arg source_commit "$workflow_head_sha" \
  -f "$ROOT/.github/scripts/validate-benchmark-evidence.jq" \
  "$tmpdir/artifact/$EVIDENCE_FILE" >/dev/null; then
  write_unpublished "evidence failed identity or seven-gate schema checks"
  exit 0
fi

fetched_at="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
run_url="https://github.com/$REPO/actions/runs/$workflow_run_id"
if command -v sha256sum >/dev/null 2>&1; then
  evidence_sha256="$(sha256sum "$tmpdir/artifact/$EVIDENCE_FILE" | awk '{print $1}')"
else
  evidence_sha256="$(shasum -a 256 "$tmpdir/artifact/$EVIDENCE_FILE" | awk '{print $1}')"
fi

jq -n \
  --slurpfile evidence "$tmpdir/artifact/$EVIDENCE_FILE" \
  --arg fetched_at "$fetched_at" \
  --arg run_url "$run_url" \
  --argjson workflow_run_id "$workflow_run_id" \
  --arg workflow_path "$WORKFLOW_PATH" \
  --arg artifact_name "$ARTIFACT_NAME" \
  --argjson artifact_id "$artifact_id" \
  --arg run_created_at "$workflow_created_at" \
  --arg evidence_sha256 "$evidence_sha256" \
  '{
    status: "published",
    provenance: {
      workflow_run_url: $run_url,
      workflow_run_id: $workflow_run_id,
      workflow_path: $workflow_path,
      artifact_name: $artifact_name,
      artifact_id: $artifact_id,
      run_created_at: $run_created_at,
      fetched_at: $fetched_at,
      evidence_sha256: $evidence_sha256,
      sample: false
    },
    evidence: $evidence[0]
  }' >"$OUT"

echo "fetch-benchmark-evidence: wrote $OUT"
