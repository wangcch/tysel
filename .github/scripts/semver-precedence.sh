#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <candidate-version> <current-version>" >&2
  exit 2
fi

candidate="$1"
current="$2"
script_dir="$(cd "$(dirname "$0")" && pwd)"
bash "$script_dir/release-channel.sh" "$candidate" >/dev/null
bash "$script_dir/release-channel.sh" "$current" >/dev/null

compare_numeric() {
  local left="$1"
  local right="$2"
  if (( ${#left} < ${#right} )); then
    comparison=older
  elif (( ${#left} > ${#right} )); then
    comparison=newer
  elif [[ "$left" == "$right" ]]; then
    comparison=equal
  elif [[ "$left" < "$right" ]]; then
    comparison=older
  else
    comparison=newer
  fi
}

candidate_core="${candidate%%-*}"
current_core="${current%%-*}"
IFS=. read -r -a candidate_parts <<< "$candidate_core"
IFS=. read -r -a current_parts <<< "$current_core"
for index in 0 1 2; do
  compare_numeric "${candidate_parts[$index]}" "${current_parts[$index]}"
  if [[ "$comparison" != equal ]]; then
    printf '%s\n' "$comparison"
    exit 0
  fi
done

candidate_pre=
current_pre=
[[ "$candidate" == *-* ]] && candidate_pre="${candidate#*-}"
[[ "$current" == *-* ]] && current_pre="${current#*-}"
if [[ -z "$candidate_pre" && -z "$current_pre" ]]; then
  printf '%s\n' equal
  exit 0
elif [[ -z "$candidate_pre" ]]; then
  printf '%s\n' newer
  exit 0
elif [[ -z "$current_pre" ]]; then
  printf '%s\n' older
  exit 0
fi

IFS=. read -r -a candidate_ids <<< "$candidate_pre"
IFS=. read -r -a current_ids <<< "$current_pre"
count=${#candidate_ids[@]}
(( ${#current_ids[@]} > count )) && count=${#current_ids[@]}
for ((index = 0; index < count; index++)); do
  if (( index >= ${#candidate_ids[@]} )); then
    printf '%s\n' older
    exit 0
  elif (( index >= ${#current_ids[@]} )); then
    printf '%s\n' newer
    exit 0
  fi
  left="${candidate_ids[$index]}"
  right="${current_ids[$index]}"
  [[ "$left" == "$right" ]] && continue
  if [[ "$left" =~ ^[0-9]+$ && "$right" =~ ^[0-9]+$ ]]; then
    compare_numeric "$left" "$right"
  elif [[ "$left" =~ ^[0-9]+$ ]]; then
    comparison=older
  elif [[ "$right" =~ ^[0-9]+$ ]]; then
    comparison=newer
  elif [[ "$left" < "$right" ]]; then
    comparison=older
  else
    comparison=newer
  fi
  printf '%s\n' "$comparison"
  exit 0
done

printf '%s\n' equal
