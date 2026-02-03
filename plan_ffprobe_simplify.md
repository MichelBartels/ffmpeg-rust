Plan: Simplify ffprobe to a Single Async Entry Point

Goal
Provide one async Rust API for ffprobe that runs with a fixed CLI argument set and returns parsed JSON, so the result shape is stable and types can be stricter with fewer Option fields.

Non-goals
- Changing ffmpeg run paths beyond keeping `run_ffmpeg` as-is.
- Supporting arbitrary ffprobe arguments at call sites.

Current Surface (Rust)
- `run_ffprobe(...) -> RunHandle` plus `RunHandle::wait_with_output()` and `FfprobeOutput::parse_json()` in `rustproto/src/lib.rs`.
- `run_ffmpeg(...)` uses the same handle type and must remain.

Proposed Public API
- Single async function (no feature flags):
  - `pub async fn ffprobe<S: Source + 'static>(source: S) -> Result<FfprobeOutput, FfprobeError>`
- Expose the fixed argument list as a constant for transparency:
  - `pub const FFPROBE_ARGS: &[&str] = &[ ... ]`
- Rename `FfprobeJson` to `FfprobeOutput` (parsed JSON struct, not raw bytes).
- Error type includes stderr and args for debugging without adding extra public functions:
  - `FfprobeError { message, stderr, args }`

Fixed ffprobe Arguments (candidate)
- Always output JSON and only the fields we model:
  - `-hide_banner`
  - `-loglevel error`
  - `-show_optional_fields always` (ensure optional fields are emitted as `"N/A"`/`"unknown"`)
  - `-print_format json`
  - `-show_format`
  - `-show_streams`
  - `-print_filename input`
  - `-i {input}`
  - Optional: `-show_entries format=...,stream=...` to guarantee field set
- Keep `-show_entries` if we want stricter non-optional fields; otherwise rely on `show_format/streams` and keep some Option fields.

Type Model Tightening
- Audit the current optional fields and reclassify based on fixed args:
  - Make required if ffprobe always emits them with `-show_format -show_streams` (and any `-show_entries` list we adopt).
  - Keep optional if they depend on stream type or are omitted for some containers.
  - Keep `#[serde(default)]` for maps (`tags`, `disposition`) and `#[serde(flatten)] unknown` for forward compatibility.

Field Audit (Which Fields Can Be `N/A`)
Source of truth: `fftools/ffprobe.c` `show_format` and `show_stream` with default (non-`-bitexact`) settings.

Format fields that can be `N/A` (keep `Option<T>` and map `"N/A"` -> `None`)
- `start_time` (N/A when `AV_NOPTS_VALUE`)
- `duration` (N/A when `AV_NOPTS_VALUE`)
- `size` (N/A when size < 0)
- `bit_rate` (N/A when bit_rate <= 0)

Format fields that are always emitted (remove `Option<T>`)
- `format_long_name` (prints `"unknown"` when missing)
- `probe_score`
- `filename`, `nb_streams`, `nb_programs`, `nb_stream_groups`, `format_name`

Stream common fields that can be `N/A` (keep `Option<T>` and map `"N/A"` -> `None`)
- `start_time` (N/A when `stream->start_time == AV_NOPTS_VALUE`)
- `duration` (N/A when `stream->duration == AV_NOPTS_VALUE`)
- `bit_rate` (N/A when `par->bit_rate <= 0`)

Stream common fields that are always emitted (remove `Option<T>`)
- `codec_type`, `codec_name`, `codec_long_name`, `profile` (use `"unknown"` when missing)
- `codec_tag`, `codec_tag_string`
- `r_frame_rate`, `avg_frame_rate`, `time_base` (can be `0/0` but still emitted)
- `index`

Stream fields that can be `N/A` and/or are type-specific (keep `Option<T>` unless using per-type variants)
- Video-only + can be `N/A`: `sample_aspect_ratio`, `display_aspect_ratio`
- Subtitle-only + can be `N/A`: `width`, `height` (bitmap-less subtitles)

Stream fields that are type-specific but not `N/A` (can be required if we split by `codec_type`)
- Video-only: `width`, `height`, `pix_fmt`, `level`
- Audio-only: `sample_rate`, `channels`, `channel_layout`, `bits_per_sample`

Optional reduction via variants (recommended if we want fewer `Option`s)
- Introduce `enum Stream { Video(VideoStream), Audio(AudioStream), Subtitle(SubtitleStream), Other(OtherStream) }` keyed by `codec_type`.
- Make variant-specific fields required within their struct; keep `Option<T>` only for fields that can be `"N/A"`.
  - `VideoStream` required: `width`, `height`, `pix_fmt`, `level`, `sample_aspect_ratio`, `display_aspect_ratio`
  - `AudioStream` required: `sample_rate`, `channels`, `channel_layout`, `bits_per_sample`
  - `SubtitleStream` required: `width`, `height` (keep `Option` to preserve `"N/A"` for bitmap-less subtitles)

Implementation Steps
1. Add a fixed CLI argument list constant and a small builder that injects `{input}`.
2. Implement `ffprobe(...)`:
   - Prepare args (with `{input}` replacement).
   - Run ffprobe in-process with captured stdout/stderr (reuse existing capture logic).
   - Parse stdout into `FfprobeOutput`, return parse error with stderr and args.
3. Keep `run_ffmpeg` intact. Remove or de-publicize `run_ffprobe`, `RunHandle`, and raw-output `FfprobeOutput` (retain internal helpers if needed).
4. Rename `FfprobeJson` to `FfprobeOutput` and update all call sites.
5. Tighten `Format`/`Stream` fields based on the fixed args; reduce `Option` usage where guaranteed.
6. Adjust tests to use the new async API and validate the narrowed schema.

Tests
- `ffprobe` returns valid JSON for `Big_Buck_Bunny.mp4`.
- Args are stable and match the constant.
- Schema checks: required fields present; optional fields only where expected.
- Concurrent calls do not interfere (if concurrency is still a requirement).

Migration Notes
- Replace call sites of `run_ffprobe(...).wait_with_output().parse_json()` with `ffprobe(...)`.
- If any caller relies on stderr or raw output, funnel those into `FfprobeError` or expand the error type.

Decisions
- Do not use `-show_entries` initially. It does not change which fields are `N/A`, and it risks omitting fields that vary by codec/container. Fixed args include `-hide_banner -loglevel error -show_optional_fields always -print_format json -show_format -show_streams -print_filename input -i {input}`. We can add `-show_entries` later only if output size or stability becomes a problem.
- Optional → required decisions are defined by the audit above; only `N/A`-capable fields remain `Option<T>`.
