//! Clipboard round-trip self-test, using only synthetic items.
//!
//! This one touches the REAL system clipboard (it replaces whatever is there and leaves it empty),
//! so it is a separate, explicit subcommand. It simulates "another app copied something" by writing
//! content without our do-not-record markers, then checks what `capture` makes of it.

use crate::clipboard::{self, Captured, Formats, Skip};
use pasteroll_core::image::{self, Rgba};
use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

struct Check {
    name: String,
    ok: bool,
    detail: String,
}

fn owner() -> HWND {
    unsafe {
        let hinst = GetModuleHandleW(None).expect("module handle");
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!(""),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(hinst.into()),
            None,
        )
        .expect("owner window")
    }
}

fn gradient(w: u32, h: u32) -> Rgba {
    let mut px = Vec::new();
    for y in 0..h {
        for x in 0..w {
            px.extend_from_slice(&[(x * 255 / w) as u8, (y * 255 / h) as u8, 200, 255]);
        }
    }
    Rgba { width: w, height: h, pixels: px }
}

pub fn run() -> i32 {
    let hwnd = owner();
    let f = Formats::register();
    let mut out: Vec<Check> = Vec::new();
    let mut check = |name: &str, ok: bool, detail: String| {
        out.push(Check { name: name.to_owned(), ok, detail });
    };

    // 1. text another app copied is captured
    let w1 = clipboard::write_unmarked(hwnd, || clipboard::put_text_raw("héllo wörld 😀 synthetic"));
    check("write_plain_text", w1.is_ok(), format!("{w1:?}"));
    match clipboard::capture(&f, clipboard::MAX_CLIP_BYTES) {
        Ok(Captured::Text(t)) => check("capture_text", t == "héllo wörld 😀 synthetic", format!("{} chars", t.chars().count())),
        other => check("capture_text", false, format!("{:?}", other.map(|_| "non-text"))),
    }

    // 2. our own marked write is NOT captured
    let w2 = clipboard::set_text(hwnd, &f, "private paste");
    check("set_text_marked", w2.is_ok(), format!("{w2:?}"));
    check(
        "own_paste_not_recorded",
        matches!(clipboard::capture(&f, clipboard::MAX_CLIP_BYTES), Err(Skip::Excluded)),
        "capture skipped our marked write".into(),
    );

    // 3. a password manager's CanIncludeInClipboardHistory = 0 is honoured; 1 is not an opt-out
    let _ = clipboard::write_unmarked(hwnd, || {
        clipboard::put_text_raw("hunter2")?;
        clipboard::put_raw(f.can_include, &0u32.to_le_bytes())
    });
    check(
        "can_include_zero_is_excluded",
        matches!(clipboard::capture(&f, clipboard::MAX_CLIP_BYTES), Err(Skip::Excluded)),
        String::new(),
    );
    let _ = clipboard::write_unmarked(hwnd, || {
        clipboard::put_text_raw("allowed")?;
        clipboard::put_raw(f.can_include, &1u32.to_le_bytes())
    });
    check(
        "can_include_one_is_captured",
        matches!(clipboard::capture(&f, clipboard::MAX_CLIP_BYTES), Ok(Captured::Text(ref t)) if t == "allowed"),
        String::new(),
    );
    // present but malformed (too short) counts as "exclude"
    let _ = clipboard::write_unmarked(hwnd, || {
        clipboard::put_text_raw("odd")?;
        clipboard::put_raw(f.can_include, &[1u8])
    });
    check(
        "malformed_marker_is_excluded",
        matches!(clipboard::capture(&f, clipboard::MAX_CLIP_BYTES), Err(Skip::Excluded)),
        String::new(),
    );
    // the exclude-from-monitor marker
    let _ = clipboard::write_unmarked(hwnd, || {
        clipboard::put_text_raw("kp")?;
        clipboard::put_raw(f.exclude, &1u32.to_le_bytes())
    });
    check(
        "exclude_from_monitor_is_excluded",
        matches!(clipboard::capture(&f, clipboard::MAX_CLIP_BYTES), Err(Skip::Excluded)),
        String::new(),
    );

    // 4. files
    let paths = vec!["C:\\synthetic\\a.txt".to_owned(), "C:\\synthetic\\b c.txt".to_owned()];
    let _ = clipboard::write_unmarked(hwnd, || clipboard::put_files_raw(&paths));
    match clipboard::capture(&f, clipboard::MAX_CLIP_BYTES) {
        Ok(Captured::Files(got)) => check("capture_files", got == paths, format!("{got:?}")),
        other => check("capture_files", false, format!("{:?}", other.map(|_| "non-files"))),
    }

    // 5. an image copied as a DIB becomes PNG
    let img = gradient(64, 40);
    let _ = clipboard::write_unmarked(hwnd, || {
        clipboard::put_raw(clipboard::dib_format(), &image::rgba_to_dib(&img))
    });
    match clipboard::capture(&f, clipboard::MAX_CLIP_BYTES) {
        Ok(Captured::Image(png)) => {
            let back = image::decode_png(&png);
            check(
                "capture_dib_as_png",
                back.as_ref() == Some(&img),
                format!("{} byte PNG, dims {:?}", png.len(), back.map(|b| (b.width, b.height))),
            );
        }
        other => check("capture_dib_as_png", false, format!("{:?}", other.map(|_| "non-image"))),
    }

    // 6. an image copied as registered PNG is kept as-is
    let png = image::encode_png(&img).unwrap();
    let _ = clipboard::write_unmarked(hwnd, || clipboard::put_raw(f.png, &png));
    check(
        "capture_png_as_is",
        matches!(clipboard::capture(&f, clipboard::MAX_CLIP_BYTES), Ok(Captured::Image(ref b)) if *b == png),
        String::new(),
    );

    // 7. what we paste for an image carries both formats
    let w7 = clipboard::set_image(hwnd, &f, &png, &img);
    check("set_image", w7.is_ok(), format!("{w7:?}"));
    check(
        "image_has_png_and_dib",
        clipboard::has_format(f.png) && clipboard::has_format(clipboard::dib_format()),
        String::new(),
    );

    // 8. an enormous clip is refused rather than copied
    let huge = "x".repeat(clipboard::MAX_CLIP_BYTES + 10); // UTF-16 doubles it past the raw cap
    let _ = clipboard::write_unmarked(hwnd, || clipboard::put_text_raw(&huge));
    check(
        "oversized_clip_refused",
        matches!(clipboard::capture(&f, clipboard::MAX_CLIP_BYTES), Err(Skip::TooLarge)),
        format!("{} chars", huge.len()),
    );

    clipboard::clear(hwnd);
    unsafe {
        let _ = DestroyWindow(hwnd);
    }

    let failed = out.iter().filter(|c| !c.ok).count();
    for c in &out {
        println!("{:<4} {:<32} {}", if c.ok { "PASS" } else { "FAIL" }, c.name, c.detail);
    }
    println!("{} checks, {} failed", out.len(), failed);
    i32::from(failed > 0)
}
