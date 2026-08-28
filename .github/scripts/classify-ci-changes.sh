#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <changed-files>" >&2
  exit 2
fi

changed_files="$1"
[[ -f "$changed_files" ]] || {
  echo "changed-files input does not exist: $changed_files" >&2
  exit 2
}

docs_only=true
file_count=0
while IFS= read -r path || [[ -n "$path" ]]; do
  [[ -n "$path" ]] || continue
  file_count=$((file_count + 1))
  case "$path" in
    docs/*|website/*|brand/*|README.md|CHANGELOG.md|CONTRIBUTING.md|CODE_OF_CONDUCT.md|SECURITY.md|mkdocs.yml|requirements-docs.txt)
      ;;
    *)
      docs_only=false
      ;;
  esac
done < "$changed_files"

# An empty or unrecognized comparison must never select the reduced CI path.
if [[ "$file_count" -eq 0 ]]; then
  docs_only=false
fi

if [[ "$docs_only" == true ]]; then
  mode=docs-only
  full=false
  benchmark_required=false
else
  mode=full
  full=true
  benchmark_required=true
fi

printf 'mode=%s\n' "$mode"
printf 'docs_only=%s\n' "$docs_only"
printf 'full=%s\n' "$full"
printf 'benchmark_required=%s\n' "$benchmark_required"
echo "CI change classification: ${mode} (${file_count} files)" >&2
