#!/usr/bin/env bash
# Fetch the pinned DMN TCK revision used for the conformance claim.
set -euo pipefail
TCK_REV=5cd912f281831bc535e710aba435e16de198028a   # dmn-tck/tck master, 2026-11-04
DEST="${1:-.tck}"
if [ ! -d "$DEST/.git" ]; then
  git clone --quiet https://github.com/dmn-tck/tck.git "$DEST"
fi
git -C "$DEST" fetch --quiet origin "$TCK_REV" 2>/dev/null || true
git -C "$DEST" checkout --quiet "$TCK_REV"
echo "DMN TCK at $(git -C "$DEST" rev-parse HEAD) in $DEST"
