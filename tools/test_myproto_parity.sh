#!/usr/bin/env bash
set -euo pipefail

FFMPEG_BIN="${FFMPEG_BIN:-./ffmpeg}"
INPUT_FILE="/Users/michelbartels/Documents/personal-projects/backend-torrent/ffmpeg/Big_Buck_Bunny.mp4"
SEG_TIME="${SEG_TIME:-4}"
ABR_KBPS="${ABR_KBPS:-128}"
MAX_SECONDS="${MAX_SECONDS:-600}"

if [[ ! -x "$FFMPEG_BIN" ]]; then
  echo "ffmpeg binary not found/executable: $FFMPEG_BIN" >&2
  exit 1
fi

if [[ ! -f "$INPUT_FILE" ]]; then
  echo "Input file not found: $INPUT_FILE" >&2
  exit 1
fi

workdir="$(mktemp -d)"
trap 'rm -rf "$workdir"' EXIT

out_direct="$workdir/direct"
out_proto="$workdir/proto"
mkdir -p "$out_direct" "$out_proto"

run_cmd() {
  local input="$1"
  local outdir="$2"
  "$FFMPEG_BIN" -hide_banner -loglevel error -y \
    -fflags +genpts -i "$input" \
    -c:v copy -tag:v hvc1 -c:a aac -b:a "${ABR_KBPS}k" -ac 2 \
    -f hls -hls_time "$SEG_TIME" -hls_list_size 0 -hls_flags independent_segments \
    -hls_playlist_type event -hls_segment_type fmp4 \
    -hls_fmp4_init_filename init.mp4 \
    -hls_segment_filename "$outdir/seg_%05d.m4s" \
    -t "$MAX_SECONDS" \
    "$outdir/out.m3u8"
}

run_cmd "$INPUT_FILE" "$out_direct"
run_cmd "myproto://bbb" "$out_proto"

# Compare file lists
(cd "$out_direct" && find . -type f | sort > "$workdir/direct_files.txt")
(cd "$out_proto" && find . -type f | sort > "$workdir/proto_files.txt")

diff -u "$workdir/direct_files.txt" "$workdir/proto_files.txt"

# Compare checksums for each file
(
  cd "$out_direct"
  while IFS= read -r f; do
    shasum -a 256 "$f"
  done < "$workdir/direct_files.txt"
) | sort > "$workdir/direct_sha.txt"

(
  cd "$out_proto"
  while IFS= read -r f; do
    shasum -a 256 "$f"
  done < "$workdir/proto_files.txt"
) | sort > "$workdir/proto_sha.txt"

if diff -u "$workdir/direct_sha.txt" "$workdir/proto_sha.txt"; then
  echo "PASS: myproto output matches direct file output"
else
  echo "FAIL: outputs differ"
  exit 1
fi
