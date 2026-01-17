use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::raw::{c_char, c_int, c_longlong, c_uchar, c_void};

const BBB_PATH: &str = "/Users/michelbartels/Documents/personal-projects/backend-torrent/ffmpeg/Big_Buck_Bunny.mp4";
const AVSEEK_SIZE: i32 = 0x10000;

struct RsProtoCtx {
    file: File,
    size: i64,
    pos: i64,
}

#[no_mangle]
pub extern "C" fn rsproto_open(_uri: *const c_char, _flags: c_int, is_streamed: *mut c_int) -> *mut c_void {
    unsafe {
        if !is_streamed.is_null() {
            *is_streamed = 0; // seekable
        }
    }

    let file = match File::open(BBB_PATH) {
        Ok(f) => f,
        Err(_) => return std::ptr::null_mut(),
    };

    let size = match file.metadata() {
        Ok(m) => m.len() as i64,
        Err(_) => return std::ptr::null_mut(),
    };

    let ctx = RsProtoCtx { file, size, pos: 0 };
    Box::into_raw(Box::new(ctx)) as *mut c_void
}

#[no_mangle]
pub extern "C" fn rsproto_read(ctx: *mut c_void, buf: *mut c_uchar, size: c_int) -> c_int {
    if ctx.is_null() || buf.is_null() || size <= 0 {
        return -1;
    }

    let ctx = unsafe { &mut *(ctx as *mut RsProtoCtx) };
    let mut slice = unsafe { std::slice::from_raw_parts_mut(buf, size as usize) };

    // Ensure file cursor is at ctx.pos
    if let Err(_) = ctx.file.seek(SeekFrom::Start(ctx.pos as u64)) {
        return -1;
    }

    match ctx.file.read(&mut slice) {
        Ok(0) => 0,
        Ok(n) => {
            ctx.pos += n as i64;
            n as c_int
        }
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
        0 => pos as i64, // SEEK_SET
        1 => ctx.pos + pos as i64, // SEEK_CUR
        2 => ctx.size + pos as i64, // SEEK_END
        _ => return -1,
    };

    if new_pos < 0 {
        return -1;
    }

    if let Err(_) = ctx.file.seek(SeekFrom::Start(new_pos as u64)) {
        return -1;
    }

    ctx.pos = new_pos;
    ctx.pos as c_longlong
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

// Prevent Rust LTO from stripping symbols in some builds.
#[no_mangle]
pub extern "C" fn rsproto_version() -> c_int {
    1
}
