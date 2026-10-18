//! Session detection must not mistake DISPLAY from XWayland for an X11 desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    X11,
    Gnome,
    UnsupportedWayland,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    X11,
    GnomeKeyboard,
    GnomeMouse,
}

/// Pick a backend without ever treating a Wayland display's XWayland socket as native X11.
pub fn choose(
    explicit: Option<&str>,
    keyboard_opt_in: bool,
    detected: Backend,
) -> Result<Launch, String> {
    match explicit {
        None => match detected {
            Backend::X11 if !keyboard_opt_in => Ok(Launch::X11),
            Backend::X11 => Err("--wayland-keyboard only applies to --backend gnome".into()),
            Backend::Gnome if keyboard_opt_in => Err(
                "GNOME Wayland needs an explicit backend. Use: pasteroll run --backend gnome --wayland-keyboard".into(),
            ),
            Backend::Gnome => Ok(Launch::GnomeMouse),
            Backend::UnsupportedWayland => Err(
                "This Wayland desktop has no Pasteroll backend. --backend x11 only covers XWayland applications.".into(),
            ),
            Backend::Unavailable => Err("No graphical X11 or GNOME Wayland session was detected".into()),
        },
        Some("x11") if keyboard_opt_in => Err("--wayland-keyboard can only be used with --backend gnome".into()),
        Some("x11") => Ok(Launch::X11),
        Some("gnome") if detected != Backend::Gnome => Err(
            "--backend gnome requires a GNOME Wayland session with the Pasteroll companion enabled".into(),
        ),
        Some("gnome") if !keyboard_opt_in => Ok(Launch::GnomeMouse),
        Some("gnome") => Ok(Launch::GnomeKeyboard),
        Some(other) => Err(format!("unknown backend {other:?}; use x11 or gnome")),
    }
}

pub fn detect(
    session: Option<&str>,
    wayland: Option<&str>,
    display: Option<&str>,
    desktop: Option<&str>,
) -> Backend {
    let present = |v: Option<&str>| v.is_some_and(|s| !s.trim().is_empty());
    if session.is_some_and(|s| s.eq_ignore_ascii_case("wayland")) || present(wayland) {
        if desktop
            .unwrap_or_default()
            .split(':')
            .any(|s| s.eq_ignore_ascii_case("gnome"))
        {
            Backend::Gnome
        } else {
            Backend::UnsupportedWayland
        }
    } else if present(display) {
        Backend::X11
    } else {
        Backend::Unavailable
    }
}

pub fn current() -> Backend {
    let vars: Vec<_> = [
        "XDG_SESSION_TYPE",
        "WAYLAND_DISPLAY",
        "DISPLAY",
        "XDG_CURRENT_DESKTOP",
    ]
    .map(|name| std::env::var(name).ok())
    .into();
    detect(
        vars[0].as_deref(),
        vars[1].as_deref(),
        vars[2].as_deref(),
        vars[3].as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn xwayland_display_never_selects_x11_on_wayland() {
        assert_eq!(
            detect(
                Some("wayland"),
                Some("wayland-0"),
                Some(":0"),
                Some("ubuntu:GNOME")
            ),
            Backend::Gnome
        );
        assert_eq!(
            detect(None, Some("wayland-0"), Some(":0"), Some("KDE")),
            Backend::UnsupportedWayland
        );
    }
    #[test]
    fn no_desktop_and_missing_socket_are_distinct_from_x11() {
        assert_eq!(detect(None, None, None, None), Backend::Unavailable);
        assert_eq!(
            detect(Some("x11"), Some(""), Some(":9"), Some("GNOME")),
            Backend::X11
        );
        assert_eq!(
            detect(Some("wayland"), None, Some(":0"), Some("GNOME")),
            Backend::Gnome
        );
    }

    #[test]
    fn launch_selection_makes_wayland_opt_in_and_xwayland_escape_explicit() {
        assert_eq!(choose(None, false, Backend::X11), Ok(Launch::X11));
        assert_eq!(choose(None, false, Backend::Gnome), Ok(Launch::GnomeMouse));
        assert!(choose(None, true, Backend::Gnome).is_err());
        assert!(choose(None, false, Backend::UnsupportedWayland).is_err());
        assert_eq!(choose(Some("x11"), false, Backend::Gnome), Ok(Launch::X11));
        assert!(choose(Some("x11"), true, Backend::X11).is_err());
        assert_eq!(
            choose(Some("gnome"), false, Backend::Gnome),
            Ok(Launch::GnomeMouse)
        );
        assert_eq!(
            choose(Some("gnome"), true, Backend::Gnome),
            Ok(Launch::GnomeKeyboard)
        );
        assert!(choose(Some("gnome"), true, Backend::X11).is_err());
        assert!(choose(Some("kde"), false, Backend::UnsupportedWayland).is_err());
    }
}
