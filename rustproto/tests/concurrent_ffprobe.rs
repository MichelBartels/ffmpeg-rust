use rsproto::{run_ffprobe, FileSource};
use std::env;
use std::path::Path;
use std::thread;

const DEFAULT_INPUT: &str =
    "/Users/michelbartels/Documents/personal-projects/backend-torrent/ffmpeg/Big_Buck_Bunny.mp4";

fn run_args() -> Vec<String> {
    vec![
        "ffprobe".to_string(),
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-of".to_string(),
        "json".to_string(),
        "-show_format".to_string(),
        "-show_streams".to_string(),
        "-print_filename".to_string(),
        "input".to_string(),
        "-i".to_string(),
        "{input}".to_string(),
    ]
}

#[test]
fn concurrent_ffprobe() {
    let input_path = env::var("INPUT_FILE").unwrap_or_else(|_| DEFAULT_INPUT.to_string());
    if !Path::new(&input_path).exists() {
        panic!("Input file not found: {}", input_path);
    }

    let t1 = {
        let input = input_path.clone();
        thread::spawn(move || run_ffprobe(FileSource::new(&input), &run_args()))
    };
    let t2 = {
        let input = input_path.clone();
        thread::spawn(move || run_ffprobe(FileSource::new(&input), &run_args()))
    };

    let r1 = t1.join().expect("thread 1 join");
    let r2 = t2.join().expect("thread 2 join");
    let handle1 = r1.expect("ffprobe run 1 failed");
    let handle2 = r2.expect("ffprobe run 2 failed");
    let out1 = handle1
        .wait_with_output()
        .expect("ffprobe wait 1 failed");
    let out2 = handle2
        .wait_with_output()
        .expect("ffprobe wait 2 failed");

    assert_eq!(out1.stdout, out2.stdout, "stdout mismatch");

    let parsed = out1.parse_json().expect("parse json");
    assert_eq!(parsed.format.filename, "input");
    assert!(!parsed.streams.is_empty());
}
