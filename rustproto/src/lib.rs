use once_cell::sync::Lazy;
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

    pub fn wait(mut self) -> Result<tempfile::TempDir, String> {
        if let Some(join) = self.join.take() {
            match join.join() {
                Ok(res) => res?,
                Err(_) => return Err("ffmpeg_run thread panicked".to_string()),
            }
        }
        let _ = self.ffmpeg_ctx.take();
        let _ = self.ffprobe_ctx.take();
        self.tempdir
            .take()
            .ok_or_else(|| "ffmpeg_run tempdir already taken".to_string())
    }

    pub fn cancel(&self) {
        if let Some(ctx) = &self.ffmpeg_ctx {
            unsafe { ffmpeg_ctx_request_exit(ctx.ptr) };
        }
        if let Some(ctx) = &self.ffprobe_ctx {
            unsafe { ffprobe_ctx_request_exit(ctx.ptr) };
        }
    }

    pub fn cancel_handle(&self) -> CancelHandle {
        CancelHandle {
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
        let _ = self.ffmpeg_ctx.take();
        let _ = self.ffprobe_ctx.take();
    }
}

#[derive(Clone)]
pub struct CancelHandle {
    ffmpeg_ctx: Option<std::sync::Arc<FfmpegCtxState>>,
    ffprobe_ctx: Option<std::sync::Arc<FFProbeCtxState>>,
}

impl CancelHandle {
    pub fn cancel(&self) {
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
        _source: handle,
        ffmpeg_ctx: Some(ctx_arc),
        ffprobe_ctx: None,
    })
}

/// Run ffprobe in-process with a temporary output directory.
///
/// The `args` must include `{input}` which will be replaced with a
/// `myproto://<id>` URL pointing to `source`. You can also use `{outdir}` in
/// any argument to substitute the temp directory path returned by this
/// function. Use `RunHandle::path()` to access the output directory before
/// the run completes, and `RunHandle::wait()` to wait for completion.
pub fn run_ffprobe<S: Source + 'static>(
    source: S,
    args: &[String],
) -> Result<RunHandle, String> {
    let (dir, handle, replaced) = prepare_run(source, args)?;
    let ctx = unsafe { ffprobe_ctx_create() };
    if ctx.is_null() {
        return Err("ffprobe_ctx_create failed".to_string());
    }
    let ctx_arc = std::sync::Arc::new(FFProbeCtxState { ptr: ctx });
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
            ffprobe_run_with_ctx(
                ctx_for_thread.ptr,
                argv.len() as c_int,
                argv.as_mut_ptr(),
                0,
                0,
            )
        };
        if ret == 0 {
            Ok(())
        } else {
            Err(format!("ffprobe_run failed: {}", ret))
        }
    });

    Ok(RunHandle {
        tempdir: Some(dir),
        join: Some(join),
        _source: handle,
        ffmpeg_ctx: None,
        ffprobe_ctx: Some(ctx_arc),
    })
}
