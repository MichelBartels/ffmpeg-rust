#!/usr/bin/env bash
set -euo pipefail

BIN="${FFMPEG_CONCURRENT_BIN:-./ffmpeg_concurrent_parity}"

if [[ ! -x "$BIN" ]]; then
  make ffmpeg_concurrent_parity
fi

"$BIN"
