use rsproto::{run_ffmpeg, FileSource};
use std::env;
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const DEFAULT_INPUT: &str =
    "/Users/michelbartels/Documents/personal-projects/backend-torrent/ffmpeg/Big_Buck_Bunny.mp4";

#[test]
fn cancel_terminates() {
    let input_path = env::var("INPUT_FILE").unwrap_or_else(|_| DEFAULT_INPUT.to_string());
    if !Path::new(&input_path).exists() {
        panic!("Input file not found: {}", input_path);
    }

    let handle =
        run_ffmpeg(FileSource::new(&input_path), &run_args("{input}", "{outdir}"))
            .expect("run_ffmpeg start");
    let cancel = handle.cancel_handle();

    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let res = handle.wait();
        let _ = tx.send(res);
    });

    thread::sleep(Duration::from_millis(200));
    if rx.try_recv().is_ok() {
        panic!("ffmpeg finished before cancel was issued");
    }
    cancel.cancel();

    let res = rx
        .recv_timeout(Duration::from_secs(20))
        .expect("ffmpeg did not terminate after cancel");
    let _ = res;
}

fn run_args(input: &str, outdir: &str) -> Vec<String> {
    let seg_path = format!("{}/seg_%05d.m4s", outdir);
    let out_path = format!("{}/out.m3u8", outdir);
    vec![
        "ffmpeg".to_string(),
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-y".to_string(),
        "-fflags".to_string(),
        "+genpts".to_string(),
        "-i".to_string(),
        input.to_string(),
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
        "independent_segments".to_string(),
        "-hls_playlist_type".to_string(),
        "event".to_string(),
        "-hls_segment_type".to_string(),
        "fmp4".to_string(),
        "-hls_fmp4_init_filename".to_string(),
        "init.mp4".to_string(),
        "-hls_segment_filename".to_string(),
        seg_path,
        "-t".to_string(),
        "600".to_string(),
        out_path,
    ]
}
