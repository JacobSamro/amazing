#!/usr/bin/env bash
# Fetch compact evidence only. Raw logs/results/pixels remain on the build host.
# Usage: lane-fetch.sh LANE DEST remote/{name.compact,name.summary,name.manifest}.json ...
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN="$(dirname "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)")"
LANE="${1:?lane}"; DEST="${2:?destination}"; shift 2
[ "$#" -gt 0 ] || { echo 'lane-fetch: explicit compact paths required' >&2; exit 2; }
# No globs/recursive pulls or raw artifact escape hatch.
for src in "$@"; do
  [[ "$src" =~ ^[a-zA-Z0-9_./-]*/([a-zA-Z0-9_.-]+\.)?(compact|summary|manifest)\.json$ ]] || {
    echo "lane-fetch: only explicit compact/summary/manifest JSON paths are allowed" >&2; exit 2;
  }
done
mkdir -p "$DEST"
HOST="$("$REPO/scripts/linux-run.sh" "$LANE" --which)"
if [ "$HOST" = boat ]; then
  read -r HOST PORT _ < <("$MAIN/scripts/boat-lane.py" endpoint "$LANE")
  SSH=(ssh -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -i "$HOME/.ssh/ascii_box_ed25519" -p "$PORT")
  SCP=(scp -q -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -i "$HOME/.ssh/ascii_box_ed25519" -P "$PORT")
else
  SSH=(ssh -o BatchMode=yes -o ControlPath=none "$HOST")
  SCP=(scp -q -o ControlPath=none)
fi
total=0
for src in "$@"; do
  # Remote size/hash before transfer: never pull an oversized/raw payload first.
  if [ "${SSH[${#SSH[@]}-1]}" = "$HOST" ]; then contact=("${SSH[@]}")
  else contact=("${SSH[@]}" "$HOST"); fi
  facts="$("${contact[@]}" "test -f '$src' && test \$(stat -c %s '$src') -le 524288 && sha256sum '$src' && stat -c %s '$src'")" || exit 3
  hash="${facts%% *}"; size="${facts##*$'\n'}"
  total=$((total + size)); [ "$total" -le 2097152 ] || { echo 'lane-fetch: ticket exceeds 2 MiB' >&2; exit 2; }
  tmp="$(mktemp "$DEST/.fetch.XXXXXX")"
  if ! "${SCP[@]}" "$HOST:$src" "$tmp"; then rm -f "$tmp"; exit 3; fi
  actual="$(shasum -a 256 "$tmp")"; actual="${actual%% *}"
  [ "$actual" = "$hash" ] || { rm -f "$tmp"; echo 'lane-fetch: compact hash mismatch' >&2; exit 3; }
  mv "$tmp" "$DEST/$(basename "$src")"
done
