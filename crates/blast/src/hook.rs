//! The hook: what runs on the harness's events, and what it says back.
//!
//! Three rules hold everywhere in here.
//!
//! It never blocks. Every event returns immediately; the only slow work,
//! running tests, happens in a detached process and is collected later.
//!
//! It never costs model tokens to produce. Nothing in this file asks a model
//! anything. The facts are parsed, the tests are run by a test runner, and
//! the only tokens spent are the handful of words we inject.
//!
//! It stays quiet unless it has something. Silence is the common case and
//! the correct one; an agent that learns to skim this channel has lost it.

use std::io::Read;
use std::path::Path;

use serde_json::Value;

fn text_at(value: &Value, key: &str) -> String {
    value.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// What the harness reads back as context, or nothing.
fn render(harness: &str, event: &str, text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    match harness {
        "cursor" => serde_json::json!({"additional_context": text}).to_string(),
        _ if event == "SessionStart" || event == "UserPromptSubmit" => text.to_string(),
        _ => serde_json::json!({
            "hookSpecificOutput": {"hookEventName": event, "additionalContext": text}
        })
        .to_string(),
    }
}

/// Which harness is calling. Set by the settings block we install, because
/// the payloads differ enough that guessing is worse than being told.
fn harness() -> String {
    std::env::var("BLAST_HARNESS").unwrap_or_else(|_| "claude".into())
}

/// The files a tool call touched, repo-relative.
fn touched(root: &Path, call: &Value) -> Vec<String> {
    let tool = text_at(call, "tool_name");
    if !matches!(tool.as_str(), "Write" | "Edit" | "MultiEdit" | "NotebookEdit" | "apply_patch") {
        return Vec::new();
    }
    let input = call.get("tool_input").cloned().unwrap_or(Value::Null);
    let mut out = Vec::new();
    for key in ["file_path", "path", "notebook_path"] {
        let raw = text_at(&input, key);
        if raw.is_empty() {
            continue;
        }
        if let Some(rel) = relative(root, &raw) {
            out.push(rel);
        }
    }
    out
}

fn relative(root: &Path, raw: &str) -> Option<String> {
    let path = Path::new(raw);
    let rel = if path.is_absolute() { path.strip_prefix(root).ok()? } else { path };
    let text = rel.to_string_lossy().replace('\\', "/");
    if text.is_empty() || text.starts_with("..") {
        None
    } else {
        Some(text)
    }
}

/// Every tool call in this event: one for PostToolUse, many for a batch.
fn calls(event: &Value) -> Vec<Value> {
    if let Some(list) = event.get("tool_calls").and_then(Value::as_array) {
        return list.clone();
    }
    if event.get("tool_name").is_some() {
        return vec![event.clone()];
    }
    Vec::new()
}

fn git_branch(root: &Path) -> String {
    let head = std::fs::read_to_string(root.join(".git/HEAD")).unwrap_or_default();
    head.trim().strip_prefix("ref: refs/heads/").unwrap_or("").to_string()
}

/// A line naming who else is in this repository right now.
fn room(root: &Path, me: &str) -> String {
    let peers = crate::peers::others(root, me);
    if peers.is_empty() {
        return String::new();
    }
    let mut lines = vec!["Other agents in this repo right now:".to_string()];
    for peer in peers.iter().take(5) {
        let what = if peer.path.is_empty() {
            peer.action.clone()
        } else {
            format!("{} {}", peer.action, peer.path)
        };
        let who = if peer.harness.is_empty() { "an agent".into() } else { peer.harness.clone() };
        lines.push(format!("  {who} ({}): {what}, {}", &peer.agent[..6.min(peer.agent.len())], crate::peers::ago(peer.age_s)));
    }
    lines.push(
        "Say so to the user if any of that changes what you are about to do.".to_string(),
    );
    lines.join("\n")
}

/// The verdict a detached test run left behind, as one short block.
fn verdict_note(root: &Path) -> String {
    let Some(verdict) = crate::verify::take(root) else { return String::new() };
    let failing: Vec<String> = verdict
        .get("failing")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let changed: Vec<String> = verdict
        .get("changed")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let what = changed.first().cloned().unwrap_or_else(|| "your change".into());
    if verdict.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        return format!("Blast: the tests covering {what} passed after your change.");
    }
    if failing.is_empty() {
        return format!("Blast: the tests covering {what} did not pass after your change.");
    }
    let shown = failing.iter().take(6).cloned().collect::<Vec<_>>().join(", ");
    let more = if failing.len() > 6 { format!(" and {} more", failing.len() - 6) } else { String::new() };
    format!(
        "Blast: your change to {what} breaks {} test{} in files you did not edit: {shown}{more}.\n\
         Fix them or tell the user what you are leaving broken, before you move on.",
        failing.len(),
        if failing.len() == 1 { "" } else { "s" }
    )
}

/// Freshen the index when it is missing or the tree has moved past it.
fn ensure_index(root: &Path) -> Option<Value> {
    if let Some(index) = crate::index::read(root) {
        return Some(index);
    }
    let (index, _, _) = crate::index::build(root);
    let _ = crate::index::write(root, &index);
    Some(index)
}

pub fn run() -> i32 {
    let mut raw = String::new();
    if std::io::stdin().read_to_string(&mut raw).is_err() {
        return 0;
    }
    let event: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    let name = text_at(&event, "hook_event_name");
    let cwd = text_at(&event, "cwd");
    let here = if cwd.is_empty() {
        std::env::current_dir().unwrap_or_default()
    } else {
        Path::new(&cwd).to_path_buf()
    };
    let root = crate::config::repo_root(&here);

    // a hosted coordinator owns this repo: say nothing, do nothing
    if !crate::config::forced_alone() && crate::config::coordinator_present(&root).is_some() {
        return 0;
    }

    let session = text_at(&event, "session_id");
    let worker = text_at(&event, "agent_id");
    let me = crate::peers::agent_id(&session, &worker);
    let harness = harness();
    let branch = git_branch(&root);

    let mut said: Vec<String> = Vec::new();

    match name.as_str() {
        "SessionStart" => {
            let _ = crate::peers::announce(
                &root,
                &crate::peers::Announce {
                    agent: &me,
                    harness: &harness,
                    action: "joined",
                    path: "",
                    symbols: &[],
                    branch: &branch,
                },
            );
            // build the index if this repo has never been indexed, so the
            // first edit of the session already has a graph to ask
            if crate::index::read(&root).is_none() {
                let (index, files, symbols) = crate::index::build(&root);
                let _ = crate::index::write(&root, &index);
                if files > 0 {
                    said.push(format!(
                        "Blast indexed {files} file{} and {symbols} symbol{}: edits here are checked against what calls them.",
                        if files == 1 { "" } else { "s" },
                        if symbols == 1 { "" } else { "s" }
                    ));
                }
            }
            let peers = room(&root, &me);
            if !peers.is_empty() {
                said.push(peers);
            }
        }
        "SessionEnd" => {
            crate::peers::withdraw(&root, &me);
        }
        "UserPromptSubmit" => {
            let peers = room(&root, &me);
            if !peers.is_empty() {
                said.push(peers);
            }
            let verdict = verdict_note(&root);
            if !verdict.is_empty() {
                said.push(verdict);
            }
        }
        "PostToolUse" | "PostToolBatch" | "Stop" => {
            let mut changed: Vec<String> = Vec::new();
            for call in calls(&event) {
                changed.extend(touched(&root, &call));
            }
            changed.sort();
            changed.dedup();

            if !changed.is_empty() {
                let _ = crate::peers::announce(
                    &root,
                    &crate::peers::Announce {
                        agent: &me,
                        harness: &harness,
                        action: "editing",
                        path: changed.first().map(String::as_str).unwrap_or(""),
                        symbols: &[],
                        branch: &branch,
                    },
                );
                if let Some(index) = ensure_index(&root) {
                    let tests = crate::verify::tests_for(&index, &changed);
                    crate::verify::spawn(&root, &tests, &changed);
                }
            }
            // whatever a previous run finished saying
            let verdict = verdict_note(&root);
            if !verdict.is_empty() {
                said.push(verdict);
            }
        }
        _ => {}
    }

    let out = render(&harness, &name, &said.join("\n\n"));
    if !out.is_empty() {
        println!("{out}");
    }
    0
}
