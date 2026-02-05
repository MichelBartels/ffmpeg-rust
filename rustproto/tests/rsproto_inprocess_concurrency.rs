use rsproto::{ffprobe, run_ffmpeg, Source};
use std::{
    env,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

// NOTE: This test is `#[ignore]` because it can crash the process (SIGABRT) if
// the embedded ffprobe/ffmpeg code is not re-entrant/thread-safe. That's the
// point: keep a minimal repro in the rsproto crate.

fn find_bbb() -> Option<PathBuf> {
    if let Ok(p) = env::var("BBB_PATH") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }

    let candidates = [
        "/Users/michelbartels/Documents.nosync/personal-projects/backend-torrent/ffmpeg/Big_Buck_Bunny.mp4",
        "/Users/michelbartels/Documents/personal-projects/backend-torrent/ffmpeg/Big_Buck_Bunny.mp4",
        "Big_Buck_Bunny.mp4",
    ];

    for c in candidates {
        let p = PathBuf::from(c);
        if p.exists() {
            return Some(p);
        }
    }

    None
}

fn block_on<F: std::future::Future>(mut fut: F) -> F::Output {
    use std::pin::Pin;
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    fn raw_waker() -> RawWaker {
        fn no_op(_: *const ()) {}
        fn clone(_: *const ()) -> RawWaker {
            raw_waker()
        }
        static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, no_op, no_op, no_op);
        RawWaker::new(std::ptr::null(), &VTABLE)
    }

    let waker = unsafe { Waker::from_raw(raw_waker()) };
    let mut cx = Context::from_waker(&waker);
    // Safety: we never move the future after pinning.
    let mut fut = unsafe { Pin::new_unchecked(&mut fut) };
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(val) => return val,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

#[derive(Clone)]
struct SlowFileSource {
    path: PathBuf,
    delay: Duration,
}

impl SlowFileSource {
    fn new(path: impl AsRef<Path>, delay: Duration) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
            delay,
        }
    }
}

struct SlowReadSeek {
    inner: std::fs::File,
    delay: Duration,
}

impl Read for SlowReadSeek {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.delay != Duration::ZERO {
            thread::sleep(self.delay);
        }
        self.inner.read(buf)
    }
}

impl Seek for SlowReadSeek {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

impl Source for SlowFileSource {
    fn open(&self) -> std::io::Result<Box<dyn rsproto::ReadSeek>> {
        let file = std::fs::File::open(&self.path)?;
        Ok(Box::new(SlowReadSeek {
            inner: file,
            delay: self.delay,
        }))
    }

    fn size(&self) -> std::io::Result<i64> {
        Ok(std::fs::metadata(&self.path)?.len() as i64)
    }
}

fn ffmpeg_hls_args(seconds: u32) -> Vec<String> {
    vec![
        "ffmpeg".to_string(),
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-y".to_string(),
        "-fflags".to_string(),
        "+genpts".to_string(),
        "-probesize".to_string(),
        "2M".to_string(),
        "-analyzeduration".to_string(),
        "2M".to_string(),
        "-i".to_string(),
        "{input}".to_string(),
        "-t".to_string(),
        seconds.to_string(),
        "-c:v".to_string(),
        "copy".to_string(),
        "-c:a".to_string(),
        "aac".to_string(),
        "-b:a".to_string(),
        "128k".to_string(),
        "-ac".to_string(),
        "2".to_string(),
        "-f".to_string(),
        "hls".to_string(),
        "-hls_time".to_string(),
        "4".to_string(),
        "-hls_list_size".to_string(),
        "0".to_string(),
        "-hls_flags".to_string(),
        "append_list".to_string(),
        "-hls_playlist_type".to_string(),
        "event".to_string(),
        "-hls_segment_type".to_string(),
        "fmp4".to_string(),
        "-hls_fmp4_init_filename".to_string(),
        "init.mp4".to_string(),
        "-hls_segment_filename".to_string(),
        "{outdir}/seg_%05d.m4s".to_string(),
        "{outdir}/index.m3u8".to_string(),
    ]
}

#[test]
#[ignore]
fn inprocess_ffprobe_and_ffmpeg_hls_concurrently_may_crash() {
    let Some(path) = find_bbb() else {
        eprintln!("Big_Buck_Bunny.mp4 not found. Set BBB_PATH to run this test.");
        return;
    };
    eprintln!("Using BBB at {}", path.display());

    // Simulate slow IO so the overlap window is large.
    let ffprobe_source = SlowFileSource::new(&path, Duration::from_millis(50));
    let ffprobe_thread = thread::spawn(move || block_on(ffprobe(ffprobe_source)));

    // Kick off ffmpeg quickly while ffprobe is still reading.
    thread::sleep(Duration::from_millis(5));
    let ffmpeg_source = SlowFileSource::new(&path, Duration::from_millis(50));
    let args = ffmpeg_hls_args(15);
    let handle = run_ffmpeg(ffmpeg_source, &args).expect("run_ffmpeg start failed");

    // If the underlying in-process ffmpeg/ffprobe are not re-entrant, this can abort.
    let _out_dir = handle.wait().expect("ffmpeg failed");

    let probe = ffprobe_thread.join().expect("ffprobe join").expect("ffprobe failed");
    eprintln!(
        "ffprobe ok: format={} streams={}",
        probe.format.format_name,
        probe.streams.len()
    );
}

