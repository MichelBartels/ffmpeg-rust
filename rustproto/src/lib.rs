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

pub trait Source: Send {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize>;
    fn seek(&mut self, pos: i64, whence: i32) -> std::io::Result<i64>;
    fn size(&mut self) -> std::io::Result<i64>;
    fn is_streamed(&self) -> bool {
        false
    }
}

pub trait SourceFactory: Send + Sync {
    fn open(&self) -> std::io::Result<Box<dyn Source>>;
    fn is_streamed(&self) -> bool {
        false
    }
}

pub struct FileSourceFactory {
    path: String,
}

impl FileSourceFactory {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        Self {
            path: path.as_ref().to_string_lossy().to_string(),
        }
    }
}

impl SourceFactory for FileSourceFactory {
    fn open(&self) -> std::io::Result<Box<dyn Source>> {
        let file = File::open(&self.path)?;
        let size = file.metadata()?.len() as i64;
        Ok(Box::new(FileSource { file, size }))
    }
}

struct FileSource {
    file: File,
    size: i64,
}

impl Source for FileSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }

    fn seek(&mut self, pos: i64, whence: i32) -> std::io::Result<i64> {
        let new_pos = match whence {
            0 => pos,
            1 => self.file.stream_position()? as i64 + pos,
            2 => self.size + pos,
            _ => return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "bad whence")),
        };
        if new_pos < 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "negative seek"));
        }
        self.file.seek(SeekFrom::Start(new_pos as u64))?;
        Ok(new_pos)
    }

    fn size(&mut self) -> std::io::Result<i64> {
        Ok(self.size)
    }
}

struct Registry {
    next_id: u64,
    factories: HashMap<u64, Arc<dyn SourceFactory>>,
}

static REGISTRY: Lazy<Mutex<Registry>> = Lazy::new(|| {
    Mutex::new(Registry {
        next_id: 1,
        factories: HashMap::new(),
    })
});

pub struct SourceHandle {
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
        reg.factories.remove(&self.id);
    }
}

pub fn register_source(factory: Arc<dyn SourceFactory>) -> SourceHandle {
    let mut reg = REGISTRY.lock().unwrap();
    let id = reg.next_id;
    reg.next_id += 1;
    reg.factories.insert(id, factory);
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
    source: Box<dyn Source>,
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
    let mut source = None;
    let mut streamed = false;

    if let Some(id) = parse_id(uri) {
        if let Some(factory) = REGISTRY.lock().unwrap().factories.get(&id).cloned() {
            match factory.open() {
                Ok(s) => {
                    streamed = factory.is_streamed();
                    source = Some(s);
                }
                Err(_) => {
                    return std::ptr::null_mut();
                }
            }
        }
    }

    if source.is_none() {
        let factory = FileSourceFactory::new(FALLBACK_PATH);
        match factory.open() {
            Ok(s) => {
                streamed = factory.is_streamed();
                source = Some(s);
            }
            Err(_) => return std::ptr::null_mut(),
        }
    }

    if !is_streamed.is_null() {
        unsafe { *is_streamed = if streamed { 1 } else { 0 } };
    }

    let ctx = RsProtoCtx {
        source: source.unwrap(),
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

    match ctx.source.read(slice) {
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
        return ctx.source.size().unwrap_or(-1) as c_longlong;
    }

    match ctx.source.seek(pos as i64, whence) {
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
}

pub fn run_ffmpeg(args: &[String]) -> Result<(), String> {
    let mut cstrings: Vec<CString> = Vec::with_capacity(args.len());
    for arg in args {
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
        Ok(())
    } else {
        Err(format!("ffmpeg_run failed: {}", ret))
    }
}

pub fn run_ffmpeg_with_tempdir<F>(build_args: F) -> Result<tempfile::TempDir, String>
where
    F: FnOnce(&Path) -> Vec<String>,
{
    let dir = tempfile::TempDir::new().map_err(|e| e.to_string())?;
    let args = build_args(dir.path());
    run_ffmpeg(&args)?;
    Ok(dir)
}
