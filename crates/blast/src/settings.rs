//! Wiring the hook into a harness, without touching anything that is not ours.
//!
//! The settings file belongs to the developer. Other tools are in it, their
//! own hooks are in it, and a tool that rewrites the file wholesale will
//! eventually delete somebody's work. So this merges: it reads what is
//! there, adds only entries whose command is ours, removes only entries
//! whose command is ours, and writes the rest back untouched.
//!
//! Our commands are recognisable by the `.blast/` path in them and nothing
//! else. No other tool's entry can match that, and ours cannot match any
//! other tool's test for its own.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// Events worth waking for, and what each is for.
const EVENTS: &[&str] = &[
    "SessionStart",     // announce, index if this repo is new
    "UserPromptSubmit", // who else is here, and any verdict waiting
    "PostToolUse",      // an edit landed: work out the radius, run its tests
    "Stop",             // last chance to deliver a verdict before the turn ends
    "SessionEnd",       // leave the room
];

pub fn is_ours(command: &str) -> bool {
    command.contains(".blast/") || command.contains("blast hook")
}

fn settings_path(root: &Path, harness: &str) -> PathBuf {
    match harness {
        "cursor" => root.join(".cursor/hooks.json"),
        "codex" => root.join(".codex/hooks.json"),
        _ => root.join(".claude/settings.json"),
    }
}

fn read(path: &Path) -> Value {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}))
}

/// Strip every entry of ours, leaving everything else exactly as it was.
fn without_ours(node: &mut Value) {
    match node {
        Value::Object(map) => {
            for value in map.values_mut() {
                without_ours(value);
            }
        }
        Value::Array(items) => {
            items.retain(|item| {
                let ours = |v: &Value| v.as_str().map(is_ours).unwrap_or(false);
                if ours(item) {
                    return false;
                }
                if let Some(command) = item.get("command") {
                    if ours(command) {
                        return false;
                    }
                }
                if let Some(hooks) = item.get("hooks").and_then(Value::as_array) {
                    if !hooks.is_empty()
                        && hooks.iter().all(|h| h.get("command").map(ours).unwrap_or(false))
                    {
                        return false;
                    }
                }
                true
            });
            for item in items.iter_mut() {
                without_ours(item);
            }
        }
        _ => {}
    }
}

fn command_for(harness: &str) -> String {
    format!("BLAST_HARNESS={harness} .blast/bin/blast hook")
}

/// Install (or refresh) our entries. Returns the file written.
pub fn install(root: &Path, harness: &str) -> std::io::Result<PathBuf> {
    let path = settings_path(root, harness);
    let mut blob = read(&path);
    without_ours(&mut blob);

    let command = command_for(harness);
    let map = blob.as_object_mut().expect("settings root is an object");
    let hooks = map.entry("hooks").or_insert_with(|| json!({}));
    let hooks = hooks.as_object_mut().expect("hooks is an object");
    for event in EVENTS {
        let entry = json!({"hooks": [{"type": "command", "command": command, "timeout": 10}]});
        let list = hooks.entry((*event).to_string()).or_insert_with(|| json!([]));
        if let Some(items) = list.as_array_mut() {
            items.push(entry);
        }
    }

    let text = serde_json::to_string_pretty(&blob).unwrap_or_default();
    crate::atomic::write(&path, format!("{text}\n").as_bytes())?;
    Ok(path)
}

/// Take our entries back out. Everything else in the file survives.
pub fn uninstall(root: &Path, harness: &str) -> std::io::Result<Option<PathBuf>> {
    let path = settings_path(root, harness);
    if !path.exists() {
        return Ok(None);
    }
    let mut blob = read(&path);
    without_ours(&mut blob);
    let text = serde_json::to_string_pretty(&blob).unwrap_or_default();
    crate::atomic::write(&path, format!("{text}\n").as_bytes())?;
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("blast-settings-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".claude")).unwrap();
        dir
    }

    /// The guarantee: another tool's hooks, and the developer's own, come
    /// through install and uninstall untouched.
    #[test]
    fn leaves_every_other_tool_alone() {
        let root = temp("foreign");
        let path = root.join(".claude/settings.json");
        let theirs = json!({
            "model": "opus",
            "hooks": {
                "PostToolUse": [
                    {"hooks": [{"type": "command", "command": "./scripts/mine.sh"}]},
                    {"hooks": [{"type": "command", "command": ".collide/bin/collide-hook report"}]}
                ]
            }
        });
        fs::write(&path, serde_json::to_string_pretty(&theirs).unwrap()).unwrap();

        install(&root, "claude").unwrap();
        let after: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let commands = |blob: &Value| -> Vec<String> {
            blob["hooks"]["PostToolUse"]
                .as_array()
                .unwrap()
                .iter()
                .map(|entry| entry["hooks"][0]["command"].as_str().unwrap().to_string())
                .collect()
        };
        let installed = commands(&after);
        assert!(installed.iter().any(|c| c.contains("./scripts/mine.sh")), "{installed:?}");
        assert!(installed.iter().any(|c| c.contains("collide-hook")), "{installed:?}");
        assert!(installed.iter().any(|c| c.contains(".blast/bin/blast")), "{installed:?}");
        assert_eq!(after["model"], "opus", "unrelated settings survive");

        uninstall(&root, "claude").unwrap();
        let after: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let left = commands(&after);
        assert_eq!(left.len(), 2, "only ours is removed: {left:?}");
        assert!(left.iter().any(|c| c.contains("collide-hook")), "{left:?}");
        assert_eq!(after["model"], "opus");
        let _ = fs::remove_dir_all(&root);
    }

    /// Installing twice must not leave two copies wired to the same event.
    #[test]
    fn installing_twice_is_installing_once() {
        let root = temp("twice");
        install(&root, "claude").unwrap();
        install(&root, "claude").unwrap();
        let blob: Value =
            serde_json::from_str(&fs::read_to_string(root.join(".claude/settings.json")).unwrap())
                .unwrap();
        let ours = blob["hooks"]["PostToolUse"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| is_ours(e["hooks"][0]["command"].as_str().unwrap()))
            .count();
        assert_eq!(ours, 1);
        let _ = fs::remove_dir_all(&root);
    }

    /// Our own test for "is this ours" must not claim another tool's hook.
    #[test]
    fn only_claims_its_own_commands() {
        assert!(is_ours("BLAST_HARNESS=claude .blast/bin/blast hook"));
        assert!(!is_ours(".collide/bin/collide-hook report"));
        assert!(!is_ours(".collide/report_hook.py"));
        assert!(!is_ours("./scripts/mine.sh"));
    }
}
