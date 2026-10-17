//! Session detection must not mistake DISPLAY from XWayland for an X11 desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    X11,
    Gnome,
    UnsupportedWayland,
    Unavailable,
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
}
