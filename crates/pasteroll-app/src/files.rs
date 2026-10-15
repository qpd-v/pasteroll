//! Saving an image next to the user's files without ever overwriting or leaving half a file.

use std::io::Write;
use std::path::{Path, PathBuf};

/// Writes `data` to a new file `<stem>.png` in `folder`, or `<stem> (2).png`, `(3)`... if that name
/// is taken. The file is created exclusively, flushed, and removed again if the write fails.
pub fn save_unique_png(folder: &Path, stem: &str, data: &[u8]) -> std::io::Result<PathBuf> {
    for n in 0..1000 {
        let name = if n == 0 { format!("{stem}.png") } else { format!("{stem} ({}).png", n + 1) };
        let path = folder.join(name);
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut f) => {
                let r = f.write_all(data).and_then(|_| f.sync_all());
                drop(f);
                return match r {
                    Ok(()) => Ok(path),
                    Err(e) => {
                        let _ = std::fs::remove_file(&path);
                        Err(e)
                    }
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "too many files with that name"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pasteroll-files-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn writes_the_bytes() {
        let d = temp("a");
        let p = save_unique_png(&d, "shot", b"PNGDATA").unwrap();
        assert_eq!(p.file_name().unwrap(), "shot.png");
        assert_eq!(std::fs::read(&p).unwrap(), b"PNGDATA");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn never_overwrites_an_existing_file() {
        let d = temp("b");
        std::fs::write(d.join("shot.png"), b"mine").unwrap();
        let p = save_unique_png(&d, "shot", b"new").unwrap();
        assert_eq!(p.file_name().unwrap(), "shot (2).png");
        assert_eq!(std::fs::read(d.join("shot.png")).unwrap(), b"mine");
        let p3 = save_unique_png(&d, "shot", b"newer").unwrap();
        assert_eq!(p3.file_name().unwrap(), "shot (3).png");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_missing_folder_is_an_error_not_a_panic() {
        let d = temp("c").join("does-not-exist");
        assert!(save_unique_png(&d, "x", b"1").is_err());
        let _ = std::fs::remove_dir_all(d.parent().unwrap());
    }
}
