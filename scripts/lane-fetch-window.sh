#!/usr/bin/env bash
# Separate bounded visual proof transfer; never use gate-fetch for raw artifacts.
# Usage: lane-fetch-window.sh LANE DEST exact.window.manifest.json
set -euo pipefail
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MAIN="$(dirname "$(git -C "$REPO" rev-parse --path-format=absolute --git-common-dir)")"
LANE="${1:?lane}"; DEST="${2:?destination}"; MANIFEST="${3:?exact window manifest}"
[[ "$MANIFEST" =~ ^[A-Za-z0-9_./-]+\.window\.manifest\.json$ ]] || { echo 'explicit window manifest required' >&2; exit 2; }
[[ "/$MANIFEST/" != *"/../"* ]] || { echo 'manifest traversal rejected' >&2; exit 2; }
mkdir -p "$DEST"
TMP="$(mktemp -d "$DEST/.window-fetch.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
bash "$REPO/scripts/lane-fetch.sh" "$LANE" "$TMP" "$MANIFEST"
LOCAL="$TMP/$(basename "$MANIFEST")"
python3 "$REPO/scripts/window-proof.py" validate "$LOCAL" "$LANE" > "$TMP/images"
HOST="$(bash "$REPO/scripts/linux-run.sh" "$LANE" --which)"
if [ "$HOST" = boat ]; then
  read -r HOST PORT _ < <("$MAIN/scripts/boat-lane.py" endpoint "$LANE")
  SSH=(ssh -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -i "$HOME/.ssh/ascii_box_ed25519" -p "$PORT" "$HOST")
  SCP=(scp -q -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR -i "$HOME/.ssh/ascii_box_ed25519" -P "$PORT")
else
  SSH=(ssh -o BatchMode=yes -o ControlPath=none "$HOST")
  SCP=(scp -q -o ControlPath=none)
fi
while read -r hash bytes path; do
  facts="$("${SSH[@]}" "test -f '$path' && test \$(stat -c %s '$path') -le 524288 && sha256sum '$path' && stat -c %s '$path'")"
  [ "${facts%% *}" = "$hash" ] && [ "${facts##*$'\n'}" = "$bytes" ] || { echo 'remote window image differs from manifest' >&2; exit 3; }
  "${SCP[@]}" "$HOST:$path" "$TMP/$(basename "$path")"
done < "$TMP/images"
python3 "$REPO/scripts/window-proof.py" verify "$LOCAL" "$LANE" "$TMP"
# Reject collisions before publishing any image; prior proof bytes stay intact.
[ ! -e "$DEST/$(basename "$MANIFEST")" ] || { echo 'proof destination already exists' >&2; exit 3; }
while read -r hash bytes path; do
  [ ! -e "$DEST/$(basename "$path")" ] || { echo 'image destination already exists' >&2; exit 3; }
done < "$TMP/images"
while read -r hash bytes path; do mv "$TMP/$(basename "$path")" "$DEST/"; done < "$TMP/images"
mv "$LOCAL" "$DEST/"
echo "window proof: $DEST/$(basename "$MANIFEST")"
