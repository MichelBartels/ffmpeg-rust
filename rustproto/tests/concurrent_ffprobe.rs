use rsproto::{run_ffprobe, FileSource};
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
        "-o".to_string(),
        "{outdir}/out.json".to_string(),
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
    let dir1 = handle1.wait().expect("ffprobe wait 1 failed");
    let dir2 = handle2.wait().expect("ffprobe wait 2 failed");

    let mut files1 = Vec::new();
    let mut files2 = Vec::new();
    list_files(dir1.path(), Path::new("."), &mut files1).expect("list 1");
    list_files(dir2.path(), Path::new("."), &mut files2).expect("list 2");
    files1.sort();
    files2.sort();
    assert_eq!(files1, files2, "file lists differ");

    for rel in files1 {
        let a = sha256_file(&dir1.path().join(&rel)).expect("sha 1");
        let b = sha256_file(&dir2.path().join(&rel)).expect("sha 2");
        assert_eq!(a, b, "hash mismatch for {:?}", rel);
    }
}
