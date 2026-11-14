//! Ordered Multi-paste batches: plain text, rich HTML and image attachment files.
use super::*;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

pub(super) struct Pending {
    rx: Receiver<Result<Payload, String>>,
    copy: bool,
    ids: Vec<i64>,
}

enum Clip {
    Text(String),
    Files(Vec<String>),
    Image(Vec<u8>),
}

impl Clip {
    fn wipe(&mut self) {
        use zeroize::Zeroize;
        match self {
            Clip::Text(text) => text.zeroize(),
            Clip::Files(paths) => paths.iter_mut().for_each(|s| s.zeroize()),
            Clip::Image(bytes) => bytes.zeroize(),
        }
    }
}

impl Drop for Clip {
    fn drop(&mut self) {
        self.wipe();
    }
}

impl Controller {
    pub(super) fn images_chosen(&self, ids: &[i64]) -> bool {
        ids.iter()
            .any(|id| self.store.borrow().kind(*id).ok().flatten() == Some(Kind::Image))
    }

    /// `inline` applies to image-only batches. Mixed batches always offer text, HTML and files.
    pub(super) fn start_image_batch(
        &mut self,
        p: &mut dyn Platform,
        ids: Vec<i64>,
        copy: bool,
        inline: bool,
    ) {
        if self.image_batch.is_some() {
            return;
        }
        if ids.len() > 32 {
            self.flash("Choose at most 32 clips per paste", p.now_ms());
            return;
        }
        let mut clips = Vec::with_capacity(ids.len());
        let mut image_bytes = 0usize;
        let mut any_image = false;
        for id in &ids {
            let clip = {
                let st = self.store.borrow();
                read_clip(&st, *id)
            };
            let clip = match clip {
                Ok(clip) => clip,
                Err(why) => {
                    self.flash(why, p.now_ms());
                    return;
                }
            };
            if let Clip::Image(png) = &clip {
                any_image = true;
                image_bytes = image_bytes.saturating_add(png.len());
                if image_bytes > 24 * 1024 * 1024 {
                    self.flash("Choose fewer or smaller images: the batch exceeds 24 MB", p.now_ms());
                    return;
                }
            }
            clips.push(clip);
        }
        if !any_image {
            self.flash("That selection has no images", p.now_ms());
            return;
        }
        let image_only = clips.iter().all(|c| matches!(c, Clip::Image(_)));
        let folder = match p.image_batch_folder() {
            Ok(f) => Some(f),
            Err(e) => {
                self.flash(&e, p.now_ms());
                return;
            }
        };
        let line_break = p.line_break().to_owned();
        let (tx, rx) = mpsc::channel();
        let wake = self.waker.clone();
        std::thread::spawn(move || {
            let result = prepare(folder, clips, inline, image_only, &line_break);
            let _ = tx.send(result);
            wake.wake();
        });
        self.image_batch = Some(Pending { rx, copy, ids });
        self.flash(
            if image_only { "Preparing images…" } else { "Preparing clips…" },
            p.now_ms(),
        );
    }

    pub(super) fn poll_image_batch(&mut self, p: &mut dyn Platform) {
        let Some(result) = self.image_batch.as_ref().and_then(|b| b.rx.try_recv().ok()) else {
            return;
        };
        let Some(batch) = self.image_batch.take() else {
            return;
        };
        if self.open.is_none() {
            return;
        }
        let payload = match result {
            Ok(pl) => pl,
            Err(e) => {
                self.flash(&e, p.now_ms());
                return;
            }
        };
        if batch.copy {
            let now = p.now_ms();
            self.finish_copy(p, payload, batch.ids.len(), 0, now);
            return;
        }
        let Some(o) = self.open.as_ref() else {
            return;
        };
        let kept = o.kept;
        if o.browse_only {
            self.flash("Click a destination window to paste.", p.now_ms());
            return;
        }
        let target = if kept {
            p.foreground()
        } else {
            Some(o.target.clone())
        };
        let Some(target) = target else {
            return;
        };
        if target.desktop || p.target_privileged(&target) || !p.target_is_current(&target) {
            self.flash("The paste destination is unavailable", p.now_ms());
            return;
        }
        if !kept {
            self.end_session(p);
        }
        let payload = WipeOnDrop(payload);
        if self.deliver(p, &target, &payload.0).is_some() {
            for id in batch.ids {
                let _ = self.store.borrow_mut().mark_pasted(id, p.now_secs(), true);
            }
            if kept {
                self.finish_kept_picks();
            }
        }
    }
}

fn read_clip(store: &pasteroll_core::Store, id: i64) -> Result<Clip, &'static str> {
    match store.kind(id).ok().flatten() {
        Some(Kind::Text) => store.text(id).ok().flatten().map(Clip::Text).ok_or("A clip could not be read"),
        Some(Kind::Files) => store.files(id).ok().flatten().map(Clip::Files).ok_or("A file list could not be read"),
        Some(Kind::Image) => store.image_png(id).ok().flatten().map(Clip::Image).ok_or("An image could not be read"),
        None => Err("A clip could not be read"),
    }
}

fn html_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

fn base64(bytes: &[u8]) -> String {
    const ALPH: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16
            | (c.get(1).copied().unwrap_or(0) as u32) << 8
            | c.get(2).copied().unwrap_or(0) as u32;
        for i in 0..4 {
            out.push(if i > c.len() {
                '='
            } else {
                ALPH[((n >> (18 - 6 * i)) & 63) as usize] as char
            });
        }
    }
    out
}

/// Shared by the batch editor when it builds inline HTML for edited images.
pub(super) fn base64_for_editor(bytes: &[u8]) -> String {
    base64(bytes)
}

fn batch_folder(root: &PathBuf) -> Result<PathBuf, String> {
    use std::sync::atomic::AtomicU64;
    static NEXT: AtomicU64 = AtomicU64::new(0);
    if let Ok(entries) = std::fs::read_dir(root) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with("batch-")
                && e.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink())
                && e.metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|d| d.as_secs() > 7 * 86400)
            {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let folder = root.join(format!(
        "batch-{stamp}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir(&folder).map_err(|_| "Could not create image attachment files")?;
    Ok(folder)
}

fn prepare(
    root: Option<PathBuf>,
    mut clips: Vec<Clip>,
    inline: bool,
    image_only: bool,
    line_break: &str,
) -> Result<Payload, String> {
    use zeroize::Zeroize;
    let folder = batch_folder(root.as_ref().ok_or("No image folder")?)?;
    let mut text_parts: Vec<String> = Vec::new();
    let mut html = String::from("<div>");
    let mut paths = Vec::new();
    let mut image_number = 0usize;
    let mut total = 0usize;
    let result = (|| {
        for clip in clips.iter_mut() {
            match clip {
                Clip::Text(text) => {
                    let text = text.trim_end_matches(['\r', '\n']).to_owned();
                    html.push_str(&format!("<p>{}</p>", html_escape(&text).replace('\n', "<br>")));
                    text_parts.push(text);
                }
                Clip::Files(files) => {
                    let joined = files.join(line_break);
                    for file in files.iter() {
                        html.push_str(&format!("<p>{}</p>", html_escape(file)));
                    }
                    text_parts.push(joined);
                }
                Clip::Image(bytes) => {
                    image_number += 1;
                    let decoded = pasteroll_core::image::decode_png(bytes);
                    bytes.zeroize();
                    let Some(image) = decoded else {
                        return Err("An image could not be decoded".into());
                    };
                    let Some(mut png) = pasteroll_core::image::encode_png(&image) else {
                        return Err("An image could not be encoded".into());
                    };
                    total += png.len();
                    if total > 24 * 1024 * 1024 {
                        png.zeroize();
                        return Err("Image batch exceeds 24 MB".into());
                    }
                    let path = crate::files::save_unique_png(
                        &folder,
                        &format!("Image {:03}", image_number),
                        &png,
                    )
                    .map_err(|_| "Could not save image attachment")?;
                    let path_text = path.to_string_lossy().into_owned();
                    html.push_str(&format!(
                        "<p><img alt=\"Image {}\" src=\"data:image/png;base64,{}\"></p>",
                        image_number,
                        base64(&png)
                    ));
                    text_parts.push(path_text.clone());
                    paths.push(path_text);
                    png.zeroize();
                }
            }
        }
        html.push_str("</div>");
        if image_only {
            Ok(Payload::Images { paths, html, inline })
        } else {
            Ok(Payload::Mixed {
                text: text_parts.join(line_break),
                html,
                paths,
            })
        }
    })();
    for clip in clips.iter_mut() {
        clip.wipe();
    }
    if result.is_err() {
        let _ = std::fs::remove_dir_all(folder);
    }
    result
}
