#!/usr/bin/env bash
# The console's build output is committed, so it can drift from its source two
# ways: a hand-edit under crates/tessari-s3-api/assets/, or a source edit with
# no rebuild. Neither fails anything else. The build is deterministic, so
# running it must leave the asset BYTES exactly as they were.
set -euo pipefail

panel="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(git -C "$panel" rev-parse --show-toplevel)"
assets="crates/tessari-s3-api/assets"

if [ ! -d "$panel/node_modules" ]; then
  echo "panel: node_modules is missing — run 'npm install' in $panel" >&2
  exit 2
fi

fingerprint() { find "$root/$assets" -type f -exec shasum {} + | sed "s|$root/||" | sort; }

before="$(fingerprint)"
( cd "$panel" && npm run --silent typecheck && npm run --silent test && npm run --silent build )
after="$(fingerprint)"

if [ "$before" != "$after" ]; then
  echo "panel: the committed assets do not match what the source builds." >&2
  echo "       Re-run the build and commit the result, or revert the hand-edit." >&2
  exit 1
fi
echo "panel: committed assets match the source"
