use rsproto::{ffprobe, FileSource, Stream};
use std::env;
use std::path::Path;
use std::thread;

const DEFAULT_INPUT: &str =
    "/Users/michelbartels/Documents/personal-projects/backend-torrent/ffmpeg/Big_Buck_Bunny.mp4";

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

#[test]
fn concurrent_ffprobe() {
    let input_path = env::var("INPUT_FILE").unwrap_or_else(|_| DEFAULT_INPUT.to_string());
    if !Path::new(&input_path).exists() {
        panic!("Input file not found: {}", input_path);
    }

    let t1 = {
        let input = input_path.clone();
        thread::spawn(move || block_on(ffprobe(FileSource::new(&input))))
    };
    let t2 = {
        let input = input_path.clone();
        thread::spawn(move || block_on(ffprobe(FileSource::new(&input))))
    };

    let out1 = t1.join().expect("thread 1 join").expect("ffprobe run 1 failed");
    let out2 = t2.join().expect("thread 2 join").expect("ffprobe run 2 failed");

    assert_eq!(out1.format.filename, "input");
    assert_eq!(out1.format.filename, out2.format.filename);
    assert!(!out1.streams.is_empty());
    assert!(matches!(
        out1.streams[0],
        Stream::Video(_) | Stream::Audio(_) | Stream::Subtitle(_) | Stream::Other(_)
    ));
}
