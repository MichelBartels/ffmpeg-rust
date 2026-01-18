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
fn concurrent_parity_dash() {
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
    let direct_handle = r1.expect("direct run failed");
    let proto_handle = r2.expect("proto run failed");
    let direct_dir = direct_handle.wait().expect("direct wait failed");
    let proto_dir = proto_handle.wait().expect("proto wait failed");

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
    let mpd_path = format!("{}/manifest.mpd", outdir);
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
        "dash".to_string(),
        "-seg_duration".to_string(),
        "4".to_string(),
        "-use_template".to_string(),
        "1".to_string(),
        "-use_timeline".to_string(),
        "1".to_string(),
        "-init_seg_name".to_string(),
        "init-$RepresentationID$.mp4".to_string(),
        "-media_seg_name".to_string(),
        "chunk-$RepresentationID$-$Number%05d$.m4s".to_string(),
        "-adaptation_sets".to_string(),
        "id=0,streams=v id=1,streams=a".to_string(),
        "-t".to_string(),
        "600".to_string(),
        mpd_path,
    ]
}
