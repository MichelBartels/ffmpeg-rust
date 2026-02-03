use rsproto::{ffprobe, FileSource, Stream};
use std::env;
use std::path::Path;

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
fn parse_ffprobe_json_reference() {
    let input_path = env::var("INPUT_FILE").unwrap_or_else(|_| DEFAULT_INPUT.to_string());
    if !Path::new(&input_path).exists() {
        panic!("Input file not found: {}", input_path);
    }

    let parsed = block_on(ffprobe(FileSource::new(&input_path))).expect("ffprobe run failed");
    assert!(!parsed.format.format_name.is_empty());
    assert!(parsed.format.nb_streams > 0);
    assert!(!parsed.streams.is_empty());
    assert!(matches!(
        parsed.streams[0],
        Stream::Video(_) | Stream::Audio(_) | Stream::Subtitle(_) | Stream::Other(_)
    ));
}
