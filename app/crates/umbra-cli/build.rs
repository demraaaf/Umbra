use std::{env, fs, path::PathBuf};

// Copies the FFmpeg DLLs next to the built executable so it runs without PATH changes.
fn main() {
    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");
    let Ok(ffmpeg_dir) = env::var("FFMPEG_DIR") else {
        return;
    };
    let bin = PathBuf::from(ffmpeg_dir).join("bin");
    // OUT_DIR is target/<profile>/build/<pkg>-<hash>/out
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let Some(profile_dir) = out_dir.ancestors().nth(3) else {
        return;
    };
    let Ok(entries) = fs::read_dir(&bin) else {
        println!(
            "cargo:warning=FFmpeg bin folder not found at {}",
            bin.display()
        );
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("dll"))
        {
            let dest = profile_dir.join(entry.file_name());
            let stale = match (fs::metadata(&path), fs::metadata(&dest)) {
                (Ok(src), Ok(dst)) => src.len() != dst.len(),
                _ => true,
            };
            if stale {
                let _ = fs::copy(&path, &dest);
            }
        }
    }
}
