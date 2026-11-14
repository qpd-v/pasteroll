//! Image attachments and inline HTML are two representations of the same ordered batch.
use super::*;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver};

pub(super) struct Pending {
    rx: Receiver<Result<Payload, String>>,
    copy: bool,
    ids: Vec<i64>,
}

impl Controller {
    pub(super) fn images_chosen(&self, ids: &[i64]) -> bool {
        ids.iter()
            .any(|id| self.store.borrow().kind(*id).ok().flatten() == Some(Kind::Image))
    }
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
            self.flash("Choose at most 32 images per paste", p.now_ms());
            return;
        }
        let mut images = Vec::new();
        let mut total = 0usize;
        for id in &ids {
            if self.store.borrow().kind(*id).ok().flatten() != Some(Kind::Image) {
                self.flash("Choose images together, or text and files together. Mixed image/text batches are not supported.",p.now_ms());
                return;
            }
            let Some(png) = self.store.borrow().image_png(*id).ok().flatten() else {
                self.flash("An image could not be read", p.now_ms());
                return;
            };
            total = total.saturating_add(png.len());
            if total > 24 * 1024 * 1024 {
                self.flash("Choose fewer images: this batch exceeds 24 MB", p.now_ms());
                return;
            }
            images.push(png);
        }
        let folder = match p.image_batch_folder() {
            Ok(f) => f,
            Err(e) => {
                self.flash(&e, p.now_ms());
                return;
            }
        };
        let (tx, rx) = mpsc::channel();
        let wake = self.waker.clone();
        std::thread::spawn(move || {
            let result = prepare(folder, images, inline);
            let _ = tx.send(result);
            wake.wake();
        });
        self.image_batch = Some(Pending { rx, copy, ids });
        self.flash("Preparing images…", p.now_ms());
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

fn prepare(root: PathBuf, images: Vec<Vec<u8>>, inline: bool) -> Result<Payload, String> {
    use zeroize::Zeroize;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    // Retain files for delayed readers and later manual paste. Reap only our expired batches.
    if let Ok(entries) = std::fs::read_dir(&root) {
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
    let mut images = zeroize::Zeroizing::new(images);
    let result = (|| {
        let mut paths = Vec::new();
        let mut html = String::from("<div>");
        let mut total = 0usize;
        for (i, bytes) in images.iter_mut().enumerate() {
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
            let path = crate::files::save_unique_png(&folder, &format!("Image {:03}", i + 1), &png)
                .map_err(|_| "Could not save image attachment")?;
            html.push_str(&format!(
                "<img alt=\"Image {}\" src=\"data:image/png;base64,{}\"><br>",
                i + 1,
                base64(&png)
            ));
            png.zeroize();
            paths.push(path.to_string_lossy().into_owned());
        }
        html.push_str("</div>");
        Ok(Payload::Images {
            paths,
            html,
            inline,
        })
    })();
    if result.is_err() {
        let _ = std::fs::remove_dir_all(folder);
    }
    result
}
