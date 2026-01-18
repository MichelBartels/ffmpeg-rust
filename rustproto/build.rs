use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn add_link_arg(token: &str) {
    if token == "-pthread" {
        println!("cargo:rustc-link-lib=pthread");
    } else if token == "-lm" {
        println!("cargo:rustc-link-lib=m");
    } else if token == "-lz" {
        println!("cargo:rustc-link-lib=z");
    } else if token == "-lbz2" {
        println!("cargo:rustc-link-lib=bz2");
    } else if token == "-liconv" {
        println!("cargo:rustc-link-lib=iconv");
    } else if token.starts_with("-l") {
        println!("cargo:rustc-link-lib={}", &token[2..]);
    } else if token.starts_with("-L") {
        println!("cargo:rustc-link-search=native={}", &token[2..]);
    }
}

fn parse_extralibs(config: &Path) -> Vec<String> {
    let contents = fs::read_to_string(config).unwrap_or_default();
    let mut tokens = Vec::new();
    for line in contents.lines() {
        if line.starts_with("EXTRALIBS-") {
            if let Some((_, value)) = line.split_once('=') {
                tokens.extend(value.split_whitespace().map(|s| s.to_string()));
            }
        }
    }
    tokens
}

fn extract_macos_minos(obj: &Path) -> Option<String> {
    let output = Command::new("otool")
        .arg("-l")
        .arg(obj)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with("minos") {
            let mut parts = line.split_whitespace();
            let _ = parts.next();
            if let Some(ver) = parts.next() {
                return Some(ver.to_string());
            }
        }
    }
    None
}

fn main() {
    println!("cargo:rerun-if-env-changed=FFMPEG_ROOT");
    println!("cargo:rerun-if-env-changed=FFMPEG_CONFIGURE_ARGS");
    println!("cargo:rerun-if-changed=rustproto/build.rs");

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = env::var("FFMPEG_ROOT").map(PathBuf::from).unwrap_or(manifest_dir);
    let config = root.join("ffbuild").join("config.mak");
    println!("cargo:rerun-if-changed={}", config.display());

    if !config.exists() {
        let args = env::var("FFMPEG_CONFIGURE_ARGS").unwrap_or_else(|_| {
            "--disable-debug --disable-doc".to_string()
        });
        let mut cmd = Command::new("./configure");
        cmd.current_dir(&root);
        for tok in args.split_whitespace() {
            cmd.arg(tok);
        }
        let status = cmd.status().expect("failed to run ffmpeg configure");
        if !status.success() {
            panic!(
                "ffmpeg configure failed. Set FFMPEG_CONFIGURE_ARGS or pre-configure the tree at {}",
                root.display()
            );
        }
    }

    let lib_targets = [
        "libavutil/libavutil.a",
        "libavcodec/libavcodec.a",
        "libavformat/libavformat.a",
        "libavfilter/libavfilter.a",
        "libavdevice/libavdevice.a",
        "libswscale/libswscale.a",
        "libswresample/libswresample.a",
        "libpostproc/libpostproc.a",
    ];

    let mut make = Command::new("make");
    make.arg("-C").arg(&root);
    for target in &lib_targets {
        make.arg(target);
    }
    make.arg("fftools/libffmpeg_runner.a");

    let status = make.status().expect("failed to run make for ffmpeg artifacts");
    if !status.success() {
        panic!("ffmpeg build failed");
    }

    let runner_dir = root.join("fftools");
    let nomain_obj = runner_dir.join("ffmpeg_nomain.o");
    let runner_lib = runner_dir.join("libffmpeg_runner.a");
    println!("cargo:rustc-link-search=native={}", runner_dir.display());
    println!("cargo:rustc-link-lib=static=ffmpeg_runner");
    println!("cargo:rustc-link-arg={}", runner_lib.display());

    let ff_libs = [
        "avformat",
        "avcodec",
        "avfilter",
        "avdevice",
        "avutil",
        "swscale",
        "swresample",
        "postproc",
    ];

    for lib in ff_libs {
        let dir = root.join(format!("lib{}", lib));
        let path = dir.join(format!("lib{}.a", lib));
        if path.exists() {
            println!("cargo:rustc-link-search=native={}", dir.display());
            println!("cargo:rustc-link-lib=static={}", lib);
        }
    }

    let extralibs = parse_extralibs(&config);
    let mut it = extralibs.iter().peekable();
    while let Some(tok) = it.next() {
        if tok == "-framework" {
            if let Some(name) = it.next() {
                println!("cargo:rustc-link-lib=framework={}", name);
            }
            continue;
        }
        add_link_arg(tok);
    }

    let target = env::var("TARGET").unwrap_or_default();
    if target.contains("apple-darwin") {
        if let Some(minos) = extract_macos_minos(&nomain_obj) {
            println!("cargo:rustc-link-arg=-Wl,-platform_version");
            println!("cargo:rustc-link-arg=-Wl,macos");
            println!("cargo:rustc-link-arg=-Wl,{}", minos);
            println!("cargo:rustc-link-arg=-Wl,{}", minos);
        }
    }
}
