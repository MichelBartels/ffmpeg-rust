Plan: Thread-Safe In-Memory ffprobe Output (Option 1)

Goal
Capture ffprobe output in memory without files or stdout redirection, and keep it thread-safe for concurrent runs.

Approach Summary
Introduce raw callback sinks on `FFProbeContext` for stdout and stderr, route all ffprobe output through those per-context callbacks, and expose a Rust API that returns captured output in a structured result. Streaming output is not required; capture into in-memory buffers only.

JSON Schema Source of Truth (already identified)
1. Primary schema structure is defined by `doc/ffprobe.xsd` (XML schema), which enumerates all sections and fields.
2. Section layout and array/wrapper rules are defined in `fftools/ffprobe.c` via the `sections[]` table:
   - Root sections: `program_version`, `library_versions`, `pixel_formats`, `packets`, `frames`, `packets_and_frames`, `programs`, `stream_groups`, `streams`, `chapters`, `format`, `error`.
   - Arrays vs objects are indicated by `AV_TEXTFORMAT_SECTION_FLAG_IS_ARRAY`.
3. JSON formatting behavior is verified against `tests/ref/fate/ffprobe_json`:
   - `packets_and_frames` is an array whose items include a `"type"` discriminator (`"packet"`, `"frame"`, `"subtitle"`).
4. JSON writer semantics are described in `doc/ffprobe.texi` (no schema details, but confirms JSON is a first-class output format).

JSON Schema Mapping (XML -> JSON)
1. Root XML element `ffprobe` maps to a JSON object with top-level keys for each shown section.
2. XML elements marked as arrays map to JSON arrays (`...s` sections).
3. XML attributes map to JSON object fields.
4. Sections with `HAS_TYPE` add a `"type"` string field in JSON (observed in `packets_and_frames` output).
5. Variable-field sections (`tags`, `side_data`, etc.) map to JSON objects with dynamic keys, using `element_name` for entries when needed.

Recommended Schema Subset (Docs + Rust Types)
1. Focus on `-show_format -show_streams` output to keep payload stable and small.
2. `format`:
   - Required: `filename`, `nb_streams`, `nb_programs`, `nb_stream_groups`, `format_name`
   - Optional: `format_long_name`, `start_time`, `duration`, `size`, `bit_rate`, `probe_score`
   - `tags` (map)
3. `streams[]` (all types, including subtitles):
   - Required: `index`, `codec_tag`, `codec_tag_string`, `time_base`, `r_frame_rate`, `avg_frame_rate`
   - Common optional: `codec_type`, `codec_name`, `codec_long_name`, `profile`, `level`, `start_time`, `duration`, `bit_rate`
   - Video: `width`, `height`, `pix_fmt`, `sample_aspect_ratio`, `display_aspect_ratio`, `avg_frame_rate`, `r_frame_rate`
   - Audio: `sample_rate`, `channels`, `channel_layout`, `bits_per_sample`
   - Subtitles: `disposition` (default/forced), `tags` (`language`, `title`), plus `width/height` if bitmap-based
   - `disposition` (map of flags), `tags` (map)
4. Explicitly out of scope: `packets`, `frames`, `packets_and_frames`, `pixel_formats`, `programs`, `stream_groups`, and `chapters` unless later required.

Rust Type Outline (Subset)
1. `FfprobeJson` (top-level, expects `-show_format -show_streams`):
   - `format: Format`
   - `streams: Vec<Stream>` (empty if no streams)
   - `unknown: HashMap<String, serde_json::Value>` (`#[serde(flatten)]`)
2. `Format`:
   - `filename: String`
   - `nb_streams: i64`
   - `nb_programs: i64`
   - `nb_stream_groups: i64`
   - `format_name: String`
   - `format_long_name: Option<String>`
   - `duration: Option<f64>` (parsed from string/number; `"N/A"` -> `None`)
   - `size: Option<i64>` (parsed from string/number; `"N/A"` -> `None`)
   - `bit_rate: Option<i64>` (parsed from string/number; `"N/A"` -> `None`)
   - `start_time: Option<f64>` (parsed from string/number; `"N/A"` -> `None`)
   - `probe_score: Option<i64>` (present on most files)
   - `tags: HashMap<String, String>` (`#[serde(default)]`)
   - `unknown: HashMap<String, serde_json::Value>` (`#[serde(flatten)]`)
3. `Stream`:
   - Common:
     - `index: i64`
     - `codec_tag: String`
     - `codec_tag_string: String`
     - `codec_type: Option<String>` (e.g., "video", "audio", "subtitle")
     - `codec_name: Option<String>`
     - `codec_long_name: Option<String>`
     - `profile: Option<String>`
     - `level: Option<i64>`
     - `time_base: Rational` (parsed from string like "1/48000")
     - `start_time: Option<f64>` (parsed from string/number; `"N/A"` -> `None`)
     - `duration: Option<f64>` (parsed from string/number; `"N/A"` -> `None`)
     - `bit_rate: Option<i64>` (parsed from string/number; `"N/A"` -> `None`)
   - Video:
     - `width: Option<i64>`
     - `height: Option<i64>`
     - `pix_fmt: Option<String>`
     - `sample_aspect_ratio: Option<String>`
     - `display_aspect_ratio: Option<String>`
     - `avg_frame_rate: Rational` (parsed from string like "24000/1001")
     - `r_frame_rate: Rational`
   - Audio:
     - `sample_rate: Option<i64>` (parsed from string/number)
     - `channels: Option<i64>`
     - `channel_layout: Option<String>`
     - `bits_per_sample: Option<i64>`
   - Subtitles:
     - `disposition: HashMap<String, i64>` (`#[serde(default)]`)
     - `tags: HashMap<String, String>` (`#[serde(default)]`)
     - `width: Option<i64>` / `height: Option<i64>` (bitmap-based)
   - `disposition: HashMap<String, i64>` (`#[serde(default)]`)
   - `tags: HashMap<String, String>` (`#[serde(default)]`)
   - `unknown: HashMap<String, serde_json::Value>` (`#[serde(flatten)]`)
4. Helpers:
   - `Rational { num: i64, den: i64 }` with serde parser for `"num/den"` (keeps `"0/0"` as-is).
   - `deserialize_i64_from_str_or_num`, `deserialize_f64_from_str_or_num` (treat `"N/A"` as `None`).

API Design
1. Add a new output interface in the ffprobe C code using raw callbacks.
2. Provide a C API that accepts separate callbacks for stdout and stderr.
3. Replace the existing Rust `run_ffprobe` to always use the capture path (single entrypoint).
4. Define interaction with `-o`:
   - If callbacks are configured, ignore `-o` and document it, or fail fast with a clear error.

Proposed C API (concept)
1. `typedef int (*ffprobe_write_cb)(void *opaque, const uint8_t *buf, int len);`
2. `void ffprobe_ctx_set_output(FFProbeContext *ctx, ffprobe_write_cb out_cb, void *out_opaque, ffprobe_write_cb err_cb, void *err_opaque);`
3. `int ffprobe_run_with_ctx_output(FFProbeContext *ctx, int argc, char **argv, int install_signal_handlers, int stdin_interaction);`

FFprobe Code Changes
1. Add stdout and stderr callback fields to `FFProbeContext`.
2. Centralize output writes in helpers like `ffprobe_write_out(ctx, buf, len)` and `ffprobe_write_err(ctx, buf, len)`.
3. Replace all output emission sites to use the helpers.
4. Preserve existing stdout and stderr behavior when no sink is configured.
5. Ensure formatting remains identical to current ffprobe output when using standard IO.
6. Add `avtextwriter_create_callback()` or equivalent to let the writer target the per-context output sink.
7. Route `av_log` output to the per-context stderr sink using TLS (`ffprobe_ctx`) so concurrent runs are thread-safe.

Thread-Safety Guarantees
1. Each `FFProbeContext` has independent output configuration.
2. No global stdout redirection, no shared buffers.
3. Multiple concurrent runs are safe as long as each context has its own sink.

Rust Integration Changes
1. Update `run_ffprobe` (only entrypoint) to use the new C API and capture both stdout and stderr into Rust-owned buffers.
2. Use a raw callback that appends to `Vec<u8>` guarded by a `Mutex` or `parking_lot::Mutex` in the callback state.
3. Return a typed result from `RunHandle::wait_with_output()` such as:
   - `pub struct FfprobeOutput { pub tempdir: tempfile::TempDir, pub stdout: Vec<u8>, pub stderr: Vec<u8> }`
   - `pub fn wait_with_output(self) -> Result<FfprobeOutput, String>`
4. `RunHandle::wait()` should still exist (for callers who don’t need output) but must drain/drop captured output safely.
5. Add `FfprobeJson` (serde) parsing helpers:
   - `pub fn parse_json(&self) -> Result<FfprobeJson, serde_json::Error>`
   - Use `serde` with `#[serde(default)]` and `#[serde(flatten)]` to tolerate unknown/new fields.
6. Define serde helpers for mixed JSON numeric encodings (strings vs numbers) and for rationals.
7. Decide whether `FfprobeOutput` includes `json: Option<FfprobeJson>` or keeps parsing as a separate helper.
8. Rust function signature (updated single entrypoint):
   - `pub fn run_ffprobe<S: Source + 'static>(source: S, args: &[String]) -> Result<RunHandle, String>`
   - `impl RunHandle { pub fn wait_with_output(self) -> Result<FfprobeOutput, String> }`

Testing Plan
1. Use `Big_Buck_Bunny.mp4` as the input source for all tests.
2. Add a unit test that runs two concurrent ffprobe invocations and verifies independent outputs.
3. Add a test that matches captured stdout and stderr to a baseline obtained from the current stdout and stderr behavior for the same args.
4. Add a test that verifies no deadlock or data races under parallel load.
5. Add a JSON parse test against `tests/ref/fate/ffprobe_json` shape (at minimum verify keys + packet/frame item discrimination).
6. Add a JSON parse test for the recommended subset using `-show_format -show_streams` against Big Buck Bunny.
7. Add a subtitle stream test (if a subtitle-containing sample is available) to validate subtitle-specific fields.

Rollout Steps
1. Implement output sink in ffprobe and update call sites.
2. Add C API and wire it into Rust via `extern "C"`.
3. Add Rust wrapper and tests.
4. Validate output fidelity vs existing behavior.

Open Questions
1. Confirm whether to generate Rust structs from the XML schema (manual mapping) or to model only the subset we need and keep a `serde_json::Value` fallback for the rest.
