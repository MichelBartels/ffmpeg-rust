use rsproto::{run_ffmpeg, FileSource};
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::thread;

const DEFAULT_INPUT: &str =
    "/Users/michelbartels/Documents/personal-projects/backend-torrent/ffmpeg/Big_Buck_Bunny.mp4";

fn list_files(base: &Path, rel: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    let path = base.join(rel);
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let name = entry.file_name();
        let rel_path = rel.join(name);
        if file_type.is_dir() {
            list_files(base, &rel_path, out)?;
        } else if file_type.is_file() {
            out.push(rel_path);
        }
    }
    Ok(())
}

fn sha256_file(path: &Path) -> std::io::Result<Vec<u8>> {
    let mut f = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 16384];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_vec())
}

#[test]
fn concurrent_parity() {
    let input_path = env::var("INPUT_FILE").unwrap_or_else(|_| DEFAULT_INPUT.to_string());
    if !Path::new(&input_path).exists() {
        panic!("Input file not found: {}", input_path);
    }

    let t1 = {
        let input = input_path.clone();
        thread::spawn(move || run_ffmpeg(FileSource::new(&input), &run_args("{input}", "{outdir}")))
    };
    let t2 = {
        let input = input_path.clone();
        thread::spawn(move || run_ffmpeg(FileSource::new(&input), &run_args("{input}", "{outdir}")))
    };

    let r1 = t1.join().expect("thread 1 join");
    let r2 = t2.join().expect("thread 2 join");
    let direct_dir = r1.expect("direct run failed");
    let proto_dir = r2.expect("proto run failed");

    let mut direct_files = Vec::new();
    let mut proto_files = Vec::new();
    list_files(direct_dir.path(), Path::new("."), &mut direct_files).expect("list direct");
    list_files(proto_dir.path(), Path::new("."), &mut proto_files).expect("list proto");
    direct_files.sort();
    proto_files.sort();

    assert_eq!(direct_files, proto_files, "file lists differ");

    for rel in direct_files {
        let a = sha256_file(&direct_dir.path().join(&rel)).expect("sha direct");
        let b = sha256_file(&proto_dir.path().join(&rel)).expect("sha proto");
        assert_eq!(a, b, "hash mismatch for {:?}", rel);
    }
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
        "-tag:v".to_string(),
        "hvc1".to_string(),
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
