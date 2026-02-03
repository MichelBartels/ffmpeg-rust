use once_cell::sync::Lazy;
use serde::Deserialize;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::raw::{c_char, c_int, c_longlong, c_uchar, c_void};
use std::path::Path;
use std::sync::{Arc, Mutex};

const AVSEEK_SIZE: i32 = 0x10000;
const FALLBACK_PATH: &str =
    "/Users/michelbartels/Documents/personal-projects/backend-torrent/ffmpeg/Big_Buck_Bunny.mp4";

pub trait Source: Send + Sync {
    fn open(&self) -> std::io::Result<Box<dyn ReadSeek>>;
    fn size(&self) -> std::io::Result<i64>;
    fn is_streamed(&self) -> bool {
        false
    }
    fn cancel(&self) {}
}

pub trait ReadSeek: Read + Seek + Send {}
impl<T: Read + Seek + Send> ReadSeek for T {}

pub struct FileSource {
    path: String,
}

impl FileSource {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            path: path.as_ref().to_string_lossy().to_string(),
        }
    }
}

impl Source for FileSource {
    fn open(&self) -> std::io::Result<Box<dyn ReadSeek>> {
        let file = File::open(&self.path)?;
        Ok(Box::new(file))
    }

    fn size(&self) -> std::io::Result<i64> {
        Ok(std::fs::metadata(&self.path)?.len() as i64)
    }
}

struct Registry {
    next_id: u64,
    sources: HashMap<u64, Arc<dyn Source>>,
}

static REGISTRY: Lazy<Mutex<Registry>> = Lazy::new(|| {
    Mutex::new(Registry {
        next_id: 1,
        sources: HashMap::new(),
    })
});

struct SourceHandle {
    id: u64,
}

impl SourceHandle {
    pub fn url(&self) -> String {
        format!("myproto://{}", self.id)
    }

    pub fn cancel(&self) {
        cancel_source(self.id);
    }
}

impl Drop for SourceHandle {
    fn drop(&mut self) {
        let mut reg = REGISTRY.lock().unwrap();
        reg.sources.remove(&self.id);
    }
}

fn register_source(source: Arc<dyn Source>) -> SourceHandle {
    let mut reg = REGISTRY.lock().unwrap();
    let id = reg.next_id;
    reg.next_id += 1;
    reg.sources.insert(id, source);
    SourceHandle { id }
}

fn cancel_source(id: u64) {
    let reg = REGISTRY.lock().unwrap();
    if let Some(source) = reg.sources.get(&id) {
        source.cancel();
    }
}

fn parse_id(uri: &CStr) -> Option<u64> {
    let s = uri.to_string_lossy();
    let s = match s.split_once("://") {
        Some((_, rest)) => rest,
        None => s.as_ref(),
    };
    let s = s.split('/').next().unwrap_or(s);
    s.parse::<u64>().ok()
}

struct RsProtoCtx {
    handle: Box<dyn ReadSeek>,
    size: i64,
}

#[no_mangle]
pub extern "C" fn rsproto_open(
    uri: *const c_char,
    _flags: c_int,
    is_streamed: *mut c_int,
) -> *mut c_void {
    if uri.is_null() {
        return std::ptr::null_mut();
    }

    let uri = unsafe { CStr::from_ptr(uri) };
    let mut source: Option<(Box<dyn ReadSeek>, Option<i64>)> = None;
    let mut streamed = false;

    if let Some(id) = parse_id(uri) {
        if let Some(source_entry) = REGISTRY.lock().unwrap().sources.get(&id).cloned() {
            match source_entry.open() {
                Ok(s) => {
                    streamed = source_entry.is_streamed();
                    source = Some((s, source_entry.size().ok()));
                }
                Err(_) => return std::ptr::null_mut(),
            }
        }
    }

    if source.is_none() {
        let fallback = FileSource::new(FALLBACK_PATH);
        match fallback.open() {
            Ok(s) => source = Some((s, fallback.size().ok())),
            Err(_) => return std::ptr::null_mut(),
        }
    }

    if !is_streamed.is_null() {
        unsafe { *is_streamed = if streamed { 1 } else { 0 } };
    }

    let (handle, size) = source.unwrap();
    let ctx = RsProtoCtx {
        handle,
        size: size.unwrap_or(-1),
    };
    Box::into_raw(Box::new(ctx)) as *mut c_void
}

#[no_mangle]
pub extern "C" fn rsproto_read(ctx: *mut c_void, buf: *mut c_uchar, size: c_int) -> c_int {
    if ctx.is_null() || buf.is_null() || size <= 0 {
        return -1;
    }

    let ctx = unsafe { &mut *(ctx as *mut RsProtoCtx) };
    let slice = unsafe { std::slice::from_raw_parts_mut(buf, size as usize) };

    match ctx.handle.read(slice) {
        Ok(0) => 0,
        Ok(n) => n as c_int,
        Err(_) => -1,
    }
}

#[no_mangle]
pub extern "C" fn rsproto_seek(ctx: *mut c_void, pos: c_longlong, whence: c_int) -> c_longlong {
    if ctx.is_null() {
        return -1;
    }

    let ctx = unsafe { &mut *(ctx as *mut RsProtoCtx) };

    if whence == AVSEEK_SIZE {
        return ctx.size as c_longlong;
    }

    let new_pos = match whence {
        0 => pos as i64,
        1 => match ctx.handle.stream_position() {
            Ok(cur) => cur as i64 + pos as i64,
            Err(_) => return -1,
        },
        2 => ctx.size + pos as i64,
        _ => return -1,
    };

    if new_pos < 0 {
        return -1;
    }

    match ctx.handle.seek(SeekFrom::Start(new_pos as u64)) {
        Ok(v) => v as c_longlong,
        Err(_) => -1,
    }
}

#[no_mangle]
pub extern "C" fn rsproto_close(ctx: *mut c_void) -> c_int {
    if ctx.is_null() {
        return 0;
    }
    unsafe {
        let _ = Box::from_raw(ctx as *mut RsProtoCtx);
    }
    0
}

#[no_mangle]
pub extern "C" fn rsproto_version() -> c_int {
    1
}

#[repr(C)]
struct FftoolsContext {
    _private: [u8; 0],
}

#[repr(C)]
struct FFProbeContext {
    _private: [u8; 0],
}

struct FfmpegCtxState {
    ptr: *mut FftoolsContext,
}

unsafe impl Send for FfmpegCtxState {}
unsafe impl Sync for FfmpegCtxState {}

impl Drop for FfmpegCtxState {
    fn drop(&mut self) {
        unsafe { ffmpeg_ctx_free(self.ptr) };
    }
}

struct FFProbeCtxState {
    ptr: *mut FFProbeContext,
}

unsafe impl Send for FFProbeCtxState {}
unsafe impl Sync for FFProbeCtxState {}

impl Drop for FFProbeCtxState {
    fn drop(&mut self) {
        unsafe { ffprobe_ctx_free(self.ptr) };
    }
}

struct CaptureBuffer {
    buf: Mutex<Vec<u8>>,
}

impl CaptureBuffer {
    fn new() -> Self {
        Self {
            buf: Mutex::new(Vec::new()),
        }
    }

    fn into_inner(self) -> Vec<u8> {
        match self.buf.into_inner() {
            Ok(v) => v,
            Err(e) => e.into_inner(),
        }
    }
}

#[derive(Debug)]
pub struct FfprobeRunOutput {
    pub tempdir: tempfile::TempDir,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

extern "C" {

    fn ffmpeg_ctx_create(install_signal_handlers: c_int, stdin_interaction: c_int)
        -> *mut FftoolsContext;
    fn ffmpeg_ctx_free(ctx: *mut FftoolsContext);
    fn ffmpeg_ctx_request_exit(ctx: *mut FftoolsContext);
    fn ffmpeg_run_with_ctx(ctx: *mut FftoolsContext, argc: c_int, argv: *mut *mut c_char)
        -> c_int;

    fn ffprobe_ctx_create() -> *mut FFProbeContext;
    fn ffprobe_ctx_free(ctx: *mut FFProbeContext);
    fn ffprobe_ctx_request_exit(ctx: *mut FFProbeContext);
    fn ffprobe_ctx_set_output(
        ctx: *mut FFProbeContext,
        out_cb: Option<extern "C" fn(*mut c_void, *const u8, c_int) -> c_int>,
        out_opaque: *mut c_void,
        err_cb: Option<extern "C" fn(*mut c_void, *const u8, c_int) -> c_int>,
        err_opaque: *mut c_void,
    );
    fn ffprobe_run_with_ctx(
        ctx: *mut FFProbeContext,
        argc: c_int,
        argv: *mut *mut c_char,
        install_signal_handlers: c_int,
        stdin_interaction: c_int,
    ) -> c_int;
}

/// Handle for an in-process ffmpeg/ffprobe run.
///
/// Dropping this handle will request cancellation (ffmpeg only) and block
/// until the underlying run completes so the source and temp directory stay
/// valid for the duration of the run.
pub struct RunHandle {
    tempdir: Option<tempfile::TempDir>,
    join: Option<std::thread::JoinHandle<Result<(), String>>>,
    ffprobe_stdout: Option<Box<CaptureBuffer>>,
    ffprobe_stderr: Option<Box<CaptureBuffer>>,
    _source: SourceHandle,
    ffmpeg_ctx: Option<std::sync::Arc<FfmpegCtxState>>,
    ffprobe_ctx: Option<std::sync::Arc<FFProbeCtxState>>,
}

impl RunHandle {
    pub fn path(&self) -> &Path {
        self.tempdir
            .as_ref()
            .expect("RunHandle tempdir missing")
            .path()
    }

    pub fn wait(self) -> Result<tempfile::TempDir, String> {
        let output = self.wait_with_output()?;
        Ok(output.tempdir)
    }

    pub fn wait_with_output(mut self) -> Result<FfprobeRunOutput, String> {
        if let Some(join) = self.join.take() {
            match join.join() {
                Ok(res) => res?,
                Err(_) => return Err("ffmpeg_run thread panicked".to_string()),
            }
        }
        let stdout = self
            .ffprobe_stdout
            .take()
            .map(|b| b.into_inner())
            .unwrap_or_default();
        let stderr = self
            .ffprobe_stderr
            .take()
            .map(|b| b.into_inner())
            .unwrap_or_default();
        let _ = self.ffmpeg_ctx.take();
        let _ = self.ffprobe_ctx.take();
        let dir = self
            .tempdir
            .take()
            .ok_or_else(|| "ffmpeg_run tempdir already taken".to_string())?;
        Ok(FfprobeRunOutput {
            tempdir: dir,
            stdout,
            stderr,
        })
    }

    pub fn cancel(&self) {
        self._source.cancel();
        if let Some(ctx) = &self.ffmpeg_ctx {
            unsafe { ffmpeg_ctx_request_exit(ctx.ptr) };
        }
        if let Some(ctx) = &self.ffprobe_ctx {
            unsafe { ffprobe_ctx_request_exit(ctx.ptr) };
        }
    }

    pub fn cancel_handle(&self) -> CancelHandle {
        CancelHandle {
            source_id: self._source.id,
            ffmpeg_ctx: self.ffmpeg_ctx.clone(),
            ffprobe_ctx: self.ffprobe_ctx.clone(),
        }
    }

    #[cfg(feature = "tokio")]
    pub async fn wait_async(self) -> Result<tempfile::TempDir, String> {
        tokio::task::spawn_blocking(move || self.wait())
            .await
            .map_err(|_| "ffmpeg_run async join failed".to_string())?
    }
}

impl Drop for RunHandle {
    fn drop(&mut self) {
        self.cancel();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
        let _ = self.ffprobe_stdout.take();
        let _ = self.ffprobe_stderr.take();
        let _ = self.ffmpeg_ctx.take();
        let _ = self.ffprobe_ctx.take();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rational {
    pub num: i64,
    pub den: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FfprobeOutput {
    pub format: Format,
    #[serde(default)]
    pub streams: Vec<Stream>,
    #[serde(flatten)]
    pub unknown: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Format {
    pub filename: String,
    #[serde(deserialize_with = "deserialize_i64")]
    pub nb_streams: i64,
    #[serde(deserialize_with = "deserialize_i64")]
    pub nb_programs: i64,
    #[serde(deserialize_with = "deserialize_i64")]
    pub nb_stream_groups: i64,
    pub format_name: String,
    pub format_long_name: String,
    #[serde(default, deserialize_with = "deserialize_f64_opt")]
    pub start_time: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_f64_opt")]
    pub duration: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_i64_opt")]
    pub size: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_i64_opt")]
    pub bit_rate: Option<i64>,
    #[serde(deserialize_with = "deserialize_i64")]
    pub probe_score: i64,
    #[serde(default)]
    pub tags: HashMap<String, String>,
    #[serde(flatten)]
    pub unknown: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StreamCommon {
    #[serde(deserialize_with = "deserialize_i64")]
    pub index: i64,
    pub codec_tag: String,
    pub codec_tag_string: String,
    pub codec_type: String,
    pub codec_name: String,
    pub codec_long_name: String,
    pub profile: String,
    #[serde(deserialize_with = "deserialize_rational")]
    pub time_base: Rational,
    #[serde(deserialize_with = "deserialize_rational")]
    pub avg_frame_rate: Rational,
    #[serde(deserialize_with = "deserialize_rational")]
    pub r_frame_rate: Rational,
    #[serde(default, deserialize_with = "deserialize_f64_opt")]
    pub start_time: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_f64_opt")]
    pub duration: Option<f64>,
    #[serde(default, deserialize_with = "deserialize_i64_opt")]
    pub bit_rate: Option<i64>,
    #[serde(default)]
    pub disposition: HashMap<String, i64>,
    #[serde(default)]
    pub tags: HashMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VideoStream {
    #[serde(flatten)]
    pub common: StreamCommon,
    #[serde(deserialize_with = "deserialize_i64")]
    pub width: i64,
    #[serde(deserialize_with = "deserialize_i64")]
    pub height: i64,
    pub pix_fmt: String,
    #[serde(deserialize_with = "deserialize_i64")]
    pub level: i64,
    #[serde(default, deserialize_with = "deserialize_string_opt")]
    pub sample_aspect_ratio: Option<String>,
    #[serde(default, deserialize_with = "deserialize_string_opt")]
    pub display_aspect_ratio: Option<String>,
    #[serde(flatten)]
    pub unknown: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AudioStream {
    #[serde(flatten)]
    pub common: StreamCommon,
    #[serde(deserialize_with = "deserialize_i64")]
    pub sample_rate: i64,
    #[serde(deserialize_with = "deserialize_i64")]
    pub channels: i64,
    pub channel_layout: String,
    #[serde(deserialize_with = "deserialize_i64")]
    pub bits_per_sample: i64,
    #[serde(flatten)]
    pub unknown: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SubtitleStream {
    #[serde(flatten)]
    pub common: StreamCommon,
    #[serde(default, deserialize_with = "deserialize_i64_opt")]
    pub width: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_i64_opt")]
    pub height: Option<i64>,
    #[serde(flatten)]
    pub unknown: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OtherStream {
    #[serde(flatten)]
    pub common: StreamCommon,
    #[serde(flatten)]
    pub unknown: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
pub enum Stream {
    Video(VideoStream),
    Audio(AudioStream),
    Subtitle(SubtitleStream),
    Other(OtherStream),
}

impl<'de> Deserialize<'de> for Stream {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let codec_type = value
            .get("codec_type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        match codec_type {
            "video" => serde_json::from_value(value)
                .map(Stream::Video)
                .map_err(serde::de::Error::custom),
            "audio" => serde_json::from_value(value)
                .map(Stream::Audio)
                .map_err(serde::de::Error::custom),
            "subtitle" => serde_json::from_value(value)
                .map(Stream::Subtitle)
                .map_err(serde::de::Error::custom),
            _ => serde_json::from_value(value)
                .map(Stream::Other)
                .map_err(serde::de::Error::custom),
        }
    }
}

fn deserialize_i64<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Number(num) => num
            .as_i64()
            .or_else(|| num.as_u64().map(|v| v as i64))
            .ok_or_else(|| serde::de::Error::custom("invalid number")),
        serde_json::Value::String(s) => {
            if s.eq_ignore_ascii_case("N/A") {
                Err(serde::de::Error::custom("unexpected N/A for required field"))
            } else {
                s.parse::<i64>()
                    .map_err(|_| serde::de::Error::custom("invalid integer"))
            }
        }
        _ => Err(serde::de::Error::custom("invalid integer")),
    }
}

fn deserialize_i64_opt<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Number(num) => Ok(num
            .as_i64()
            .or_else(|| num.as_u64().map(|v| v as i64))),
        serde_json::Value::String(s) => {
            if s.eq_ignore_ascii_case("N/A") {
                Ok(None)
            } else {
                s.parse::<i64>()
                    .map(Some)
                    .map_err(|_| serde::de::Error::custom("invalid integer"))
            }
        }
        _ => Err(serde::de::Error::custom("invalid integer")),
    }
}

fn deserialize_string_opt<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(s) => {
            if s.eq_ignore_ascii_case("N/A") {
                Ok(None)
            } else {
                Ok(Some(s))
            }
        }
        _ => Err(serde::de::Error::custom("invalid string")),
    }
}

fn deserialize_f64_opt<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Number(num) => Ok(num.as_f64()),
        serde_json::Value::String(s) => {
            if s.eq_ignore_ascii_case("N/A") {
                Ok(None)
            } else {
                s.parse::<f64>()
                    .map(Some)
                    .map_err(|_| serde::de::Error::custom("invalid float"))
            }
        }
        _ => Err(serde::de::Error::custom("invalid float")),
    }
}

fn deserialize_rational<'de, D>(deserializer: D) -> Result<Rational, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::String(s) => parse_rational(&s)
            .ok_or_else(|| serde::de::Error::custom("invalid rational")),
        serde_json::Value::Number(num) => {
            let n = num
                .as_i64()
                .or_else(|| num.as_u64().map(|v| v as i64))
                .ok_or_else(|| serde::de::Error::custom("invalid rational"))?;
            Ok(Rational { num: n, den: 1 })
        }
        _ => Err(serde::de::Error::custom("invalid rational")),
    }
}

fn parse_rational(s: &str) -> Option<Rational> {
    let (num_str, den_str) = s.split_once('/')?;
    let num = num_str.parse::<i64>().ok()?;
    let den = den_str.parse::<i64>().ok()?;
    Some(Rational { num, den })
}

#[derive(Clone)]
pub struct CancelHandle {
    source_id: u64,
    ffmpeg_ctx: Option<std::sync::Arc<FfmpegCtxState>>,
    ffprobe_ctx: Option<std::sync::Arc<FFProbeCtxState>>,
}

impl CancelHandle {
    pub fn cancel(&self) {
        cancel_source(self.source_id);
        if let Some(ctx) = &self.ffmpeg_ctx {
            unsafe { ffmpeg_ctx_request_exit(ctx.ptr) };
        }
        if let Some(ctx) = &self.ffprobe_ctx {
            unsafe { ffprobe_ctx_request_exit(ctx.ptr) };
        }
    }
}

fn prepare_run<S: Source + 'static>(
    source: S,
    args: &[String],
) -> Result<(tempfile::TempDir, SourceHandle, Vec<String>), String> {
    let dir = tempfile::TempDir::new().map_err(|e| e.to_string())?;
    let handle = register_source(Arc::new(source));
    let url = handle.url();

    let mut replaced = Vec::with_capacity(args.len());
    let mut saw_input = false;
    let outdir = dir.path().to_string_lossy().to_string();
    for arg in args {
        if arg.contains("{input}") {
            saw_input = true;
        }
        let mut arg = arg.replace("{input}", &url);
        arg = arg.replace("{outdir}", &outdir);
        replaced.push(arg);
    }

    if !saw_input {
        return Err("args must include {input} placeholder".to_string());
    }

    Ok((dir, handle, replaced))
}

extern "C" fn capture_write(opaque: *mut c_void, buf: *const u8, len: c_int) -> c_int {
    if opaque.is_null() || buf.is_null() || len <= 0 {
        return 0;
    }
    let state = unsafe { &*(opaque as *const CaptureBuffer) };
    let slice = unsafe { std::slice::from_raw_parts(buf, len as usize) };
    let mut guard = match state.buf.lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    guard.extend_from_slice(slice);
    len
}

pub const FFPROBE_ARGS: &[&str] = &[
    "ffprobe",
    "-hide_banner",
    "-loglevel",
    "error",
    "-show_optional_fields",
    "always",
    "-of",
    "json",
    "-show_format",
    "-show_streams",
    "-print_filename",
    "input",
    "-i",
    "{input}",
];

#[derive(Debug, Clone)]
pub struct FfprobeError {
    pub message: String,
    pub stderr: Vec<u8>,
    pub args: Vec<String>,
}

impl std::fmt::Display for FfprobeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for FfprobeError {}

pub async fn ffprobe<S: Source + 'static>(source: S) -> Result<FfprobeOutput, FfprobeError> {
    ffprobe_blocking(source)
}

fn ffprobe_blocking<S: Source + 'static>(source: S) -> Result<FfprobeOutput, FfprobeError> {
    let args = ffprobe_args();
    let capture = match run_ffprobe_capture(source, &args) {
        Ok(capture) => capture,
        Err((message, capture)) => {
            return Err(FfprobeError {
                message,
                stderr: capture.stderr,
                args,
            })
        }
    };

    let parsed = serde_json::from_slice(&capture.stdout).map_err(|e| FfprobeError {
        message: format!("ffprobe json parse: {e}"),
        stderr: capture.stderr,
        args,
    })?;

    Ok(parsed)
}

fn ffprobe_args() -> Vec<String> {
    FFPROBE_ARGS.iter().map(|arg| (*arg).to_string()).collect()
}

struct FfprobeCapture {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_ffprobe_capture<S: Source + 'static>(
    source: S,
    args: &[String],
) -> Result<FfprobeCapture, (String, FfprobeCapture)> {
    let (_dir, handle, replaced) = match prepare_run(source, args) {
        Ok(v) => v,
        Err(message) => {
            return Err((
                message,
                FfprobeCapture {
                    stdout: Vec::new(),
                    stderr: Vec::new(),
                },
            ))
        }
    };
    let ctx = unsafe { ffprobe_ctx_create() };
    if ctx.is_null() {
        return Err((
            "ffprobe_ctx_create failed".to_string(),
            FfprobeCapture {
                stdout: Vec::new(),
                stderr: Vec::new(),
            },
        ));
    }

    let stdout_capture = Box::new(CaptureBuffer::new());
    let stderr_capture = Box::new(CaptureBuffer::new());
    let stdout_ptr = stdout_capture.as_ref() as *const CaptureBuffer as *mut c_void;
    let stderr_ptr = stderr_capture.as_ref() as *const CaptureBuffer as *mut c_void;
    unsafe {
        ffprobe_ctx_set_output(
            ctx,
            Some(capture_write),
            stdout_ptr,
            Some(capture_write),
            stderr_ptr,
        );
    }

    let ctx_state = FFProbeCtxState { ptr: ctx };

    let mut cstrings: Vec<CString> = Vec::with_capacity(replaced.len());
    for arg in &replaced {
        cstrings.push(
            CString::new(arg.as_bytes())
                .map_err(|_| format!("arg contains null byte: {}", arg))
                .map_err(|message| {
                    (
                        message,
                        FfprobeCapture {
                            stdout: Vec::new(),
                            stderr: Vec::new(),
                        },
                    )
                })?,
        );
    }

    let mut argv: Vec<*mut c_char> = cstrings
        .iter()
        .map(|s| s.as_ptr() as *mut c_char)
        .collect();

    let ret = unsafe { ffprobe_run_with_ctx(ctx_state.ptr, argv.len() as c_int, argv.as_mut_ptr(), 0, 0) };

    let stdout = stdout_capture.into_inner();
    let stderr = stderr_capture.into_inner();

    drop(ctx_state);
    drop(handle);

    if ret == 0 {
        Ok(FfprobeCapture { stdout, stderr })
    } else {
        Err((
            format!("ffprobe_run failed: {}", ret),
            FfprobeCapture { stdout, stderr },
        ))
    }
}

/// Run ffmpeg in-process with a temporary output directory.
///
/// The `args` must include `{input}` which will be replaced with a
/// `myproto://<id>` URL pointing to `source`. You can also use `{outdir}` in
/// any argument to substitute the temp directory path returned by this
/// function. Use `RunHandle::path()` to access the output directory before
/// the run completes, and `RunHandle::wait()` to wait for completion.
///
/// For HLS fMP4, `-hls_fmp4_init_filename` expects a basename (e.g. `init.mp4`)
/// rather than an absolute path; combine it with `-hls_segment_filename
/// {outdir}/seg_%05d.m4s` so the init segment lands in the temp directory.
pub fn run_ffmpeg<S: Source + 'static>(
    source: S,
    args: &[String],
) -> Result<RunHandle, String> {
    let (dir, handle, replaced) = prepare_run(source, args)?;
    let ctx = unsafe { ffmpeg_ctx_create(0, 0) };
    if ctx.is_null() {
        return Err("ffmpeg_ctx_create failed".to_string());
    }
    let ctx_arc = std::sync::Arc::new(FfmpegCtxState { ptr: ctx });
    let ctx_for_thread = std::sync::Arc::clone(&ctx_arc);
    let join = std::thread::spawn(move || {
        let mut cstrings: Vec<CString> = Vec::with_capacity(replaced.len());
        for arg in &replaced {
            cstrings.push(
                CString::new(arg.as_bytes())
                    .map_err(|_| format!("arg contains null byte: {}", arg))?,
            );
        }

        let mut argv: Vec<*mut c_char> = cstrings
            .iter()
            .map(|s| s.as_ptr() as *mut c_char)
            .collect();

        let ret = unsafe {
            ffmpeg_run_with_ctx(ctx_for_thread.ptr, argv.len() as c_int, argv.as_mut_ptr())
        };
        if ret == 0 {
            Ok(())
        } else {
            Err(format!("ffmpeg_run failed: {}", ret))
        }
    });

    Ok(RunHandle {
        tempdir: Some(dir),
        join: Some(join),
        ffprobe_stdout: None,
        ffprobe_stderr: None,
        _source: handle,
        ffmpeg_ctx: Some(ctx_arc),
        ffprobe_ctx: None,
    })
}
