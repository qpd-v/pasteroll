//! Synthetic integration driver, never included in release packaging.
#[cfg(not(unix))]
fn main() {}
#[cfg(unix)]
mod wayland;

#[cfg(unix)]
fn main() {
    use pasteroll_app::{Captured, Payload, Skip};
    use pasteroll_render::{Body, Renderer, ViewModel};
    use std::io::{self, BufRead, Write};
    use wayland::{
        bridge::Client,
        protocol::{self, Error},
    };
    if !std::path::Path::new("/run/pasteroll-wayland-spike").is_file()
        || !std::env::var("HOME")
            .unwrap_or_default()
            .starts_with("/tmp/pasteroll-wayland-")
    {
        eprintln!("The integration probe requires its isolated launcher");
        std::process::exit(2);
    }
    let mut client = Some(Client::connect().expect("bridge handshake"));
    let mut generation = 0;
    println!("ready");
    for line in io::stdin().lock().lines() {
        let command = line.expect("test command");
        if command == "quit" {
            break;
        }
        if command == "drop" {
            client = None;
            println!("ok drop");
            continue;
        }
        if command == "reconnect" {
            client = Some(Client::connect().expect("bridge reconnect"));
            println!("ok reconnect");
            continue;
        }
        let c = client.as_ref().expect("connected test client");
        match command.as_str() {
            "caps" => {
                assert!(!c.capabilities.mouse_activation());
                assert!(c.capabilities.keyboard_activation());
                assert!(c.capabilities.paste());
                assert!(c.capabilities.viewer());
                assert!(c.capabilities.settings());
            }
            "context" => {
                let context = c.context().expect("context");
                assert!(context.scale > 0.0);
                assert!(context.stage_scale > 0.0);
                assert!(context.monitor.2 > 0 && context.monitor.3 > 0);
                assert!(context.work.2 > 0 && context.work.3 > 0);
            }
            "text" => assert!(
                matches!(c.capture(4096), Ok(Captured::Text(s)) if s == "Native Wayland fixture")
            ),
            "files" => assert!(
                matches!(c.capture(4096), Ok(Captured::Files(p)) if p == ["/tmp/synthetic clip.png", "/tmp/idea.txt"])
            ),
            "image" => assert!(
                matches!(c.capture(4096), Ok(Captured::Image(p)) if pasteroll_core::image::decode_png(&p).is_some())
            ),
            "sensitive" => assert!(matches!(c.capture(4096), Err(Skip::Excluded))),
            "large" => assert!(matches!(c.capture(32), Err(Skip::TooLarge))),
            "write-text" => {
                c.write(&Payload::Text("Native Wayland fixture".into()))
                    .unwrap();
            }
            "write-files" => {
                c.write(&Payload::Files(vec![
                    "/tmp/synthetic clip.png".into(),
                    "/tmp/idea.txt".into(),
                ]))
                .unwrap();
            }
            "write-image" => {
                let img = pasteroll_core::image::Rgba {
                    width: 2,
                    height: 2,
                    pixels: vec![31, 80, 60, 255].repeat(4),
                };
                c.write(&Payload::Image(
                    pasteroll_core::image::encode_png(&img).unwrap(),
                ))
                .unwrap();
            }
            "own" => assert!(matches!(c.capture(4096), Err(Skip::Excluded))),
            "stale-read" => {
                let before = c.snapshot().unwrap();
                c.write(&Payload::Text("Changed fixture".into())).unwrap();
                assert_eq!(
                    c.read(before.sequence, protocol::TEXT, 4096),
                    Err(Error::Stale)
                );
            }
            "no-opt-in" => assert!(matches!(
                c.arm_keyboard(true, false),
                Err(Error::KeyboardOptInRequired)
            )),
            "show" => {
                generation = c.input_active(true).expect("input generation");
                let shown = c.show(500, 250, 400, 300, true).unwrap();
                assert_eq!(shown.0, generation, "Show must keep the active generation");
                generation = shown.0;
                assert!(generation > 0);
                let mut renderer = Renderer::new();
                let vm = ViewModel {
                    placeholder: "Wayland bridge test".into(),
                    body: Body::Message("Real Rust renderer, native GNOME surface".into()),
                    footer: "Synthetic fixture — controller integration follows".into(),
                    ..Default::default()
                };
                c.present(generation, &renderer.render(&vm, (400.0, 300.0), 1.0))
                    .unwrap();
            }
            "stale-frame" => {
                let frame = pasteroll_render::Frame {
                    width: 1,
                    height: 1,
                    bgra: vec![1, 2, 3, 255],
                };
                assert_eq!(c.present(generation - 1, &frame), Err(Error::Stale));
            }
            "viewer" => {
                assert!(generation > 0);
                c.viewer_show(generation, 620, 222, 320, 240, true)
                    .expect("viewer show");
                let frame = pasteroll_render::Frame {
                    width: 2,
                    height: 2,
                    bgra: vec![31, 80, 60, 255].repeat(4),
                };
                c.viewer_present(generation, &frame)
                    .expect("viewer present");
                c.viewer_hide(generation).expect("viewer hide");
            }
            "viewer-4k" => {
                c.viewer_show(generation, 0, 0, 3840, 2160, false).unwrap();
                let frame = pasteroll_render::Frame {
                    width: 3840, height: 2160,
                    bgra: vec![31, 80, 60, 255].repeat(3840 * 2160),
                };
                c.viewer_present(generation, &frame).expect("lossless 4K viewer");
            }
            "viewer-hide" => c.viewer_hide(generation).expect("viewer hide"),
            "suspend" => c.suspend(generation, true).expect("suspend"),
            "resume" => c.suspend(generation, false).expect("resume"),
            "poll" => {
                let events = c.poll().unwrap();
                for event in &events {
                    if !(20..=26).contains(&event.0) {
                        let _ = protocol::event(*event, generation).unwrap();
                    }
                }
                println!(
                    "events {}",
                    events
                        .iter()
                        .map(|e| e.0.to_string())
                        .collect::<Vec<_>>()
                        .join(",")
                );
                io::stdout().flush().unwrap();
                continue;
            }
            "hide" => c.hide(generation).unwrap(),
            "arm" => c.arm_keyboard(true, true).unwrap(),
            "disarm" => {
                if c.input_active(false).is_ok() {}
                c.arm_keyboard(false, true).unwrap()
            }
            "lost" => assert!(c.snapshot().is_err()),
            _ => panic!("unknown integration command"),
        }
        println!("ok {command}");
        io::stdout().flush().unwrap();
    }
}
