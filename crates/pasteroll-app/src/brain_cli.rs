//! Shared agent-brain commands, compiled with the platform keystore adapter.
use crate::keystore;
use pasteroll_core::config;
use std::path::Path;
fn fail(message: impl AsRef<str>) -> i32 { eprintln!("{}", message.as_ref()); 1 }
/// `pasteroll brain ...`: the rolls a coding agent may read, and the master switch.
pub fn brain_cmd(db: &Path, args: &[String]) -> i32 {
    let sub = args.get(1).map(String::as_str).unwrap_or("status");
    let cfg = match keystore::load_config(db) {
        Ok(c) => c,
        Err(problems) => return fail(format!("config.toml is not valid:\n{}", problems.join("\n"))),
    };
    let write = |text: &str, what: &str| -> i32 {
        match keystore::write_config_text(db, text) {
            Ok(()) => {
                println!("{what} Agents see the change on their next call.");
                0
            }
            Err(e) => fail(e),
        }
    };
    match sub {
        "status" | "show" | "list" | "" => {
            println!("Agent brain: {}", if cfg.brain.enabled { "ON" } else { "OFF" });
            println!("  config: {}", keystore::config_path(db).display());
            match keystore::open_store(db) {
                Ok(store) => {
                    let choices = match pasteroll_core::brain::choices(&store) {
                        Ok(c) => c,
                        Err(e) => return fail(e),
                    };
                    let selected = pasteroll_core::brain::selected(&store, &cfg.brain).unwrap_or_default();
                    println!("  shared rolls:");
                    if selected.is_empty() {
                        println!("    (none)");
                    }
                    for c in &selected {
                        println!("    {} ({} clips)", c.name, c.count);
                    }
                    println!("  all rolls:");
                    for c in &choices {
                        let mark = if selected.iter().any(|s| s.source == c.source) { "*" } else { " " };
                        println!("   {mark} {} ({} clips)", c.name, c.count);
                    }
                }
                Err(e) => println!("  (could not open history: {e})"),
            }
            let ranges = format!(
                "  limits: {} clips per call, {} characters per text clip, images {}, {} KB each",
                cfg.brain.max_clips,
                cfg.brain.max_clip_chars,
                if cfg.brain.include_images { "on" } else { "off" },
                cfg.brain.max_image_kb
            );
            println!("{ranges}");
            0
        }
        "on" | "enable" => match keystore::read_config_text(db).and_then(|t| config::set_value(&t, "brain.enabled", "true").map_err(|e| e.join("; "))) {
            Ok(text) => write(&text, "Agent brain is on."),
            Err(e) => fail(e),
        },
        "off" | "disable" => match keystore::read_config_text(db).and_then(|t| config::set_value(&t, "brain.enabled", "false").map_err(|e| e.join("; "))) {
            Ok(text) => write(&text, "Agent brain is off."),
            Err(e) => fail(e),
        },
        "add" | "remove" => {
            // A roll name may contain spaces; --db is the global database option, not part of it.
            let mut words = Vec::new();
            let mut rest = args.iter().skip(2);
            while let Some(word) = rest.next() {
                if word == "--db" {
                    rest.next();
                    continue;
                }
                words.push(word.clone());
            }
            let name = words.join(" ");
            let name = name.trim();
            if name.is_empty() {
                return fail(format!("usage: pasteroll brain {sub} ROLL"));
            }
            let store = match keystore::open_store(db) {
                Ok(s) => s,
                Err(e) => return fail(e),
            };
            let choices = match pasteroll_core::brain::choices(&store) {
                Ok(c) => c,
                Err(e) => return fail(e),
            };
            let found = choices.iter().find(|c| c.name.to_lowercase() == name.to_lowercase());
            if sub == "remove" && found.is_none() {
                // A roll may have been renamed or deleted: still let the person clear the stale name.
                let mut rolls = cfg.brain.rolls.clone();
                let before = rolls.len();
                rolls.retain(|r| r.to_lowercase() != name.to_lowercase());
                if rolls.len() == before {
                    return fail(format!("{name:?} is not shared with agents."));
                }
                let text = match keystore::read_config_text(db) {
                    Ok(t) => t,
                    Err(e) => return fail(e),
                };
                return match config::set_brain_rolls(&text, &rolls) {
                    Ok(text) => write(&text, &format!("Shared rolls: {}.", rolls.join(", "))),
                    Err(e) => fail(e.join("; ")),
                };
            }
            let Some(found) = found else {
                let names: Vec<&str> = choices.iter().map(|c| c.name.as_str()).collect();
                return fail(format!("There is no roll called {name:?}. Rolls: {}.", names.join(", ")));
            };
            let mut rolls = cfg.brain.rolls.clone();
            let present = rolls.iter().any(|r| r.to_lowercase() == found.name.to_lowercase());
            let changed = if sub == "add" {
                if present {
                    println!("{} is already shared with agents.", found.name);
                    return 0;
                }
                rolls.push(found.name.clone());
                true
            } else if present {
                rolls.retain(|r| r.to_lowercase() != found.name.to_lowercase());
                true
            } else {
                println!("{} is not shared with agents.", found.name);
                false
            };
            if !changed {
                return 0;
            }
            let text = match keystore::read_config_text(db) {
                Ok(t) => t,
                Err(e) => return fail(e),
            };
            match config::set_brain_rolls(&text, &rolls) {
                Ok(text) => write(&text, &format!("Shared rolls: {}.", rolls.join(", "))),
                Err(e) => fail(e.join("; ")),
            }
        }
        other => fail(format!("usage: pasteroll brain [status|on|off|add ROLL|remove ROLL] (got {other:?})")),
    }
}

/// The exact MCP entries for the three harnesses, with this program's path filled in.
pub fn mcp_config(db: &Path) -> i32 {
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => return fail(format!("could not find this program's path: {e}")),
    };
    let exe = exe.display().to_string();
    let json_exe = pasteroll_core::json::stringify(&pasteroll_core::json::string(exe.clone()));
    let toml_exe = exe.replace('\\', "\\\\").replace('"', "\\\"");
    // A custom --db must be passed on, or the harness would read the default history instead.
    let default_db = keystore::default_db().unwrap_or_default();
    let custom_db = db != default_db;
    let db_path = db.display().to_string();
    let json_db = pasteroll_core::json::stringify(&pasteroll_core::json::string(db_path.clone()));
    let toml_db = db_path.replace('\\', "\\\\").replace('"', "\\\"");
    let claude_args = if custom_db { format!(" mcp --db \"{db_path}\"") } else { " mcp".to_owned() };
    let codex_args = if custom_db {
        format!("[\"mcp\", \"--db\", \"{toml_db}\"]")
    } else {
        "[\"mcp\"]".to_owned()
    };
    let opencode_args = if custom_db {
        format!("[{json_exe}, \"mcp\", \"--db\", {json_db}]")
    } else {
        format!("[{json_exe}, \"mcp\"]")
    };
    println!("Pasteroll agent brain (MCP)");
    println!("1. Turn it on and choose rolls:");
    println!("   pasteroll brain on");
    println!("   pasteroll brain add Main");
    println!();
    println!("2. Add the server once to each harness.");
    println!();
    println!("Claude Code:");
    println!("   claude mcp add --scope user pasteroll -- \"{exe}\"{claude_args}");
    println!();
    println!("Codex (~/.codex/config.toml):");
    println!("   [mcp_servers.pasteroll]");
    println!("   command = \"{toml_exe}\"");
    println!("   args = {codex_args}");
    println!();
    println!("OpenCode (opencode.json):");
    println!("   {{");
    println!("     \"mcp\": {{");
    println!("       \"pasteroll\": {{");
    println!("         \"type\": \"local\",");
    println!("         \"command\": {opencode_args},");
    println!("         \"enabled\": true");
    println!("       }}");
    println!("     }}");
    println!("   }}");
    println!();
    println!("Restart a harness after adding its entry: MCP servers are read when it starts.");
    println!("Each harness starts its own copy of the server; all read the same history and the same shared rolls.");
    0
}
