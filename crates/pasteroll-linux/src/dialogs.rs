//! The desktop's Save dialog and text recognition for the expanded viewer, through the programs
//! most desktops have: `zenity` (GNOME and most others) or `kdialog` (KDE) for the dialog,
//! `tesseract` for recognising text. Nothing is sent anywhere; when a program is missing the
//! viewer says so (and saves to the Desktop instead of asking).

use pasteroll_app::{SaveChoice, TextRecognizer};
use pasteroll_core::image::{encode_png, Rgba};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What a dialog program printed: the chosen path, or nothing when cancelled.
fn chosen(out: std::io::Result<std::process::Output>) -> Option<SaveChoice> {
    let out = match out {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(_) => return Some(SaveChoice::Cancelled),
    };
    let path = String::from_utf8_lossy(&out.stdout).trim_end_matches(['\n', '\r']).to_owned();
    Some(if out.status.success() && !path.is_empty() { SaveChoice::Path(PathBuf::from(path)) } else { SaveChoice::Cancelled })
}

pub fn save_file(suggested: &Path) -> SaveChoice {
    let s = suggested.to_string_lossy().into_owned();
    let zenity = Command::new("zenity")
        .args(["--file-selection", "--save", "--confirm-overwrite", "--title=Save clip as", &format!("--filename={s}")])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    if let Some(c) = chosen(zenity) {
        return c;
    }
    let kdialog = Command::new("kdialog")
        .args(["--title", "Save clip as", "--getsavefilename", &s])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    chosen(kdialog).unwrap_or(SaveChoice::Unavailable)
}

/// Asks which folder to save into, opening at `start`.
pub fn pick_folder(start: Option<&Path>) -> SaveChoice {
    let mut dir = start.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    if !dir.is_empty() && !dir.ends_with('/') {
        dir.push('/');
    }
    let zenity = Command::new("zenity")
        .args(["--file-selection", "--directory", "--title=Save the picture in this folder", &format!("--filename={dir}")])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    if let Some(c) = chosen(zenity) {
        return c;
    }
    let kdialog = Command::new("kdialog")
        .args(["--title", "Save the picture in this folder", "--getexistingdirectory", &dir])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    chosen(kdialog).unwrap_or(SaveChoice::Unavailable)
}

/// Tesseract, if it is installed.
pub struct Tesseract;

impl TextRecognizer for Tesseract {
    fn recognize(&self, img: &Rgba) -> Result<String, String> {
        let png = encode_png(img).ok_or("The picture could not be prepared for text recognition.")?;
        let child = Command::new("tesseract")
            .args(["stdin", "stdout"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err("Extract text needs Tesseract (for example: sudo apt install tesseract-ocr).".into())
            }
            Err(e) => return Err(format!("Text recognition could not start ({e}).")),
        };
        // the picture goes through a pipe: it is never written to a file
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(&png);
        }
        let out = child.wait_with_output().map_err(|e| format!("Text recognition failed ({e})."))?;
        if !out.status.success() {
            return Err("Text recognition failed.".into());
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_program_is_not_a_cancel() {
        let missing = Command::new("pasteroll-no-such-dialog-program").output();
        assert_eq!(chosen(missing), None);
    }

    #[test]
    fn the_printed_path_is_the_choice() {
        let out = Command::new("sh").args(["-c", "echo /tmp/a\\ b.txt"]).output();
        assert_eq!(chosen(out), Some(SaveChoice::Path(PathBuf::from("/tmp/a b.txt"))));
        let cancelled = Command::new("sh").args(["-c", "exit 1"]).output();
        assert_eq!(chosen(cancelled), Some(SaveChoice::Cancelled));
    }
}
