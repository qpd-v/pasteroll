//! Validation and serialization for settings proposed by the native GNOME panel.

use super::bridge::SettingsResult;
use pasteroll_core::config::{self, Config};
use pasteroll_core::limits::Limits;

const MAX_APPS: usize = 128;
const MAX_APP_NAME: usize = 128;

#[derive(Clone, Debug, PartialEq)]
pub struct Proposal {
    pub config: Config,
    pub autostart: bool,
}

fn app_name(value: &str) -> Result<String, String> {
    let name = value.trim().to_ascii_lowercase();
    if name.is_empty() || name.len() > MAX_APP_NAME {
        return Err("an application name is 1 to 128 characters".into());
    }
    if name
        .chars()
        .any(|c| c.is_control() || c == '/' || c == '\\' || c.is_whitespace())
    {
        return Err("application names must be executable names without paths".into());
    }
    Ok(name)
}

fn app_list(values: &[String]) -> Result<Vec<String>, String> {
    if values.len() > MAX_APPS {
        return Err(format!("at most {MAX_APPS} applications can be listed"));
    }
    let mut out = Vec::new();
    for value in values {
        let name = app_name(value)?;
        if !out.contains(&name) {
            out.push(name);
        }
    }
    Ok(out)
}

pub fn proposal(previous: &Config, result: &SettingsResult) -> Result<Proposal, String> {
    if !(config::SIZE_MIN..=config::SIZE_MAX).contains(&result.size) {
        return Err(format!(
            "carousel size must be {} to {} percent",
            config::SIZE_MIN,
            config::SIZE_MAX
        ));
    }
    if result.total_bytes % pasteroll_core::limits::MB != 0
        || result.clip_bytes % pasteroll_core::limits::MB != 0
    {
        return Err("storage limits must use whole megabytes".into());
    }
    let mut cfg = previous.clone();
    cfg.paused = result.paused;
    cfg.size_percent = result.size;
    cfg.limits = Limits {
        max_clips: u64::from(result.clips),
        max_bytes: result.total_bytes,
        max_age_days: (result.age_days != 0).then_some(result.age_days),
        max_clip_bytes: result.clip_bytes,
        max_pins: result.pins,
    };
    let problems = cfg.limits.problems();
    if !problems.is_empty() {
        return Err(problems.join("; "));
    }
    cfg.exclude_apps = app_list(&result.exclude_apps)?;
    cfg.terminal_apps = app_list(&result.terminal_apps)?;
    cfg.warn_multiline_in_terminals = result.warn_multiline;
    Ok(Proposal {
        config: cfg,
        autostart: result.autostart,
    })
}

fn quote(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                use std::fmt::Write;
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn strings(values: &[String]) -> String {
    let mut out = String::from("[");
    for (i, value) in values.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&quote(value));
    }
    out.push(']');
    out
}

/// Rewrites all recognized settings. Core parsing is the final validation gate before this text
/// is written, and custom filters plus unrelated editor options come from the previous config.
pub fn render_config(cfg: &Config) -> Result<String, String> {
    let age = cfg
        .limits
        .max_age_days
        .map_or_else(|| "\"never\"".to_owned(), |days| days.to_string());
    let mut text = String::from("[limits]\n");
    text.push_str(&format!("max_clips = {}\n", cfg.limits.max_clips));
    text.push_str(&format!(
        "max_total_mb = {}\n",
        cfg.limits.max_bytes / pasteroll_core::limits::MB
    ));
    text.push_str(&format!("max_age_days = {age}\n"));
    text.push_str(&format!(
        "max_clip_mb = {}\n",
        cfg.limits.max_clip_bytes / pasteroll_core::limits::MB
    ));
    text.push_str(&format!("max_pins = {}\n\n", cfg.limits.max_pins));
    text.push_str("[capture]\n");
    text.push_str(&format!("paused = {}\n", cfg.paused));
    text.push_str(&format!(
        "exclude_apps = {}\n\n",
        strings(&cfg.exclude_apps)
    ));
    text.push_str("[paste]\n");
    text.push_str(&format!(
        "warn_multiline_in_terminals = {}\n",
        cfg.warn_multiline_in_terminals
    ));
    text.push_str(&format!(
        "terminal_apps = {}\n",
        strings(&cfg.terminal_apps)
    ));
    text.push_str(&format!(
        "advanced_image_modifier = \"{}\"\n\n",
        cfg.advanced_image_modifier.label().to_ascii_lowercase()
    ));
    text.push_str("[appearance]\n");
    text.push_str(&format!("size = {}\n", cfg.size_percent));
    for filter in &cfg.custom_filters {
        text.push_str("\n[[filters]]\n");
        text.push_str(&format!("name = {}\n", quote(&filter.name)));
        text.push_str(&format!("extensions = {}\n", strings(&filter.extensions)));
    }
    config::parse(&text).map_err(|problems| problems.join("; "))?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pasteroll_core::config::CustomFilter;

    fn result() -> SettingsResult {
        SettingsResult {
            changed: true,
            confirmed: false,
            paused: true,
            size: 125,
            clips: 250,
            total_bytes: 32 * pasteroll_core::limits::MB,
            age_days: 0,
            clip_bytes: 4 * pasteroll_core::limits::MB,
            pins: 12,
            exclude_apps: vec![" KeePassXC ".into(), "keepassxc".into()],
            warn_multiline: false,
            terminal_apps: vec!["kitty".into()],
            autostart: true,
        }
    }

    #[test]
    fn accepted_settings_keep_unrelated_options_and_filters() {
        let previous = Config {
            advanced_image_modifier: pasteroll_core::config::ClickModifier::Alt,
            mode: pasteroll_core::config::UiMode::NoBloat,
            custom_filters: vec![CustomFilter {
                name: "Design".into(),
                extensions: vec!["psd".into(), "ai".into()],
            }],
            ..Config::default()
        };
        let proposal = proposal(&previous, &result()).unwrap();
        assert_eq!(proposal.config.exclude_apps, vec!["keepassxc"]);
        assert_eq!(proposal.config.limits.max_age_days, None);
        assert_eq!(proposal.config.custom_filters, previous.custom_filters);
        assert_eq!(proposal.config.mode, previous.mode, "a Linux save keeps the chosen mode");
        let text = render_config(&proposal.config).unwrap();
        let parsed = config::parse(&text).unwrap();
        assert_eq!(parsed, proposal.config);
        assert!(proposal.autostart);
    }

    #[test]
    fn unsafe_or_out_of_range_values_are_refused_before_writing() {
        let previous = Config::default();
        let mut r = result();
        r.size = 10;
        assert!(proposal(&previous, &r).is_err());
        let mut r = result();
        r.total_bytes = 0;
        assert!(proposal(&previous, &r).is_err());
        let mut r = result();
        r.total_bytes += 1;
        assert!(proposal(&previous, &r).is_err());
        let mut r = result();
        r.exclude_apps = vec!["../../secret".into()];
        assert!(proposal(&previous, &r).is_err());
        let mut r = result();
        r.terminal_apps = vec!["bad\nname".into()];
        assert!(proposal(&previous, &r).is_err());
    }
}
