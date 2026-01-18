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

extern "C" {
    fn ffmpeg_run_with_options(
        argc: c_int,
        argv: *mut *mut c_char,
        install_signal_handlers: c_int,
        stdin_interaction: c_int,
    ) -> c_int;
    fn ffprobe_run_with_options(
        argc: c_int,
        argv: *mut *mut c_char,
        install_signal_handlers: c_int,
        stdin_interaction: c_int,
    ) -> c_int;
}

/// Run ffmpeg in-process with a temporary output directory.
///
/// The `args` must include `{input}` which will be replaced with a
/// `myproto://<id>` URL pointing to `source`. You can also use `{outdir}` in
/// any argument to substitute the temp directory path returned by this
/// function.
///
/// For HLS fMP4, `-hls_fmp4_init_filename` expects a basename (e.g. `init.mp4`)
/// rather than an absolute path; combine it with `-hls_segment_filename
/// {outdir}/seg_%05d.m4s` so the init segment lands in the temp directory.
pub fn run_ffmpeg<S: Source + 'static>(
    source: S,
    args: &[String],
) -> Result<tempfile::TempDir, String> {
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

    let ret = unsafe { ffmpeg_run_with_options(argv.len() as c_int, argv.as_mut_ptr(), 0, 0) };
    if ret == 0 {
        Ok(dir)
    } else {
        Err(format!("ffmpeg_run failed: {}", ret))
    }
}

/// Run ffprobe in-process with a temporary output directory.
///
/// The `args` must include `{input}` which will be replaced with a
/// `myproto://<id>` URL pointing to `source`. You can also use `{outdir}` in
/// any argument to substitute the temp directory path returned by this
/// function.
pub fn run_ffprobe<S: Source + 'static>(
    source: S,
    args: &[String],
) -> Result<tempfile::TempDir, String> {
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

    let ret = unsafe { ffprobe_run_with_options(argv.len() as c_int, argv.as_mut_ptr(), 0, 0) };
    if ret == 0 {
        Ok(dir)
    } else {
        Err(format!("ffprobe_run failed: {}", ret))
    }
}
