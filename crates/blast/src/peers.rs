//! The protocol: one small file per live agent, in a directory they share.
//!
//! There is no daemon, no port and no broker. An agent announces itself by
//! writing `.blast/peers/<agent>.json` and withdraws by deleting it; if it
//! dies without withdrawing, the record ages out on its own stamp. Reading
//! the room is reading the directory.
//!
//! The filesystem is the transport on purpose. Every harness can already
//! write a file, nothing needs installing or binding, records survive a
//! crash, and two agents that never heard of each other interoperate as
//! long as they agree on this shape. Point the directory at a synced or
//! mounted folder and the same protocol spans machines unchanged.
//!
//! See PROTOCOL.md for the wire format and the compatibility rules.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

/// A record older than this is nobody: the agent exited, crashed, or moved
/// on without saying so. Every writer refreshes well inside it.
pub const STALE_AFTER_S: f64 = 180.0;

pub fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

pub fn dir(root: &Path) -> PathBuf {
    crate::config::dir_of(root).join("peers")
}

/// A stable id for this agent: its session if the harness gave it one, plus
/// the subagent id when the call came from inside one, so a conversation
/// running three workers shows as three hands rather than one.
pub fn agent_id(session: &str, worker: &str) -> String {
    let seed = format!("{session}|{worker}");
    let digest = <sha2::Sha256 as sha2::Digest>::digest(seed.as_bytes());
    format!("{:x}", digest)[..12].to_string()
}

pub struct Announce<'a> {
    pub agent: &'a str,
    pub harness: &'a str,
    pub action: &'a str,
    pub path: &'a str,
    pub symbols: &'a [String],
    pub branch: &'a str,
}

/// Say what this agent is doing. Overwrites its own record and nobody
/// else's, which is why no lock is needed: one writer per file, always.
pub fn announce(root: &Path, what: &Announce) -> std::io::Result<()> {
    let record = json!({
        "v": 1,
        "agent": what.agent,
        "harness": what.harness,
        "action": what.action,
        "path": what.path,
        "symbols": what.symbols,
        "branch": what.branch,
        "pid": std::process::id(),
        "ts": now(),
    });
    let file = dir(root).join(format!("{}.json", what.agent));
    crate::atomic::write(&file, serde_json::to_string(&record).unwrap_or_default().as_bytes())
}

/// Leave the room.
pub fn withdraw(root: &Path, agent: &str) {
    let _ = fs::remove_file(dir(root).join(format!("{agent}.json")));
}

pub struct Peer {
    pub agent: String,
    pub harness: String,
    pub action: String,
    pub path: String,
    pub symbols: Vec<String>,
    pub branch: String,
    pub age_s: f64,
}

/// Everyone else who is live here. Stale records are swept as they are
/// found, so the directory cleans itself without anything scheduled.
pub fn others(root: &Path, me: &str) -> Vec<Peer> {
    let stamp = now();
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir(root)) else { return out };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else { continue };
        let Ok(record) = serde_json::from_str::<Value>(&text) else { continue };
        let ts = record.get("ts").and_then(Value::as_f64).unwrap_or(0.0);
        let age = stamp - ts;
        if age > STALE_AFTER_S {
            let _ = fs::remove_file(&path);
            continue;
        }
        let text_at = |key: &str| record.get(key).and_then(Value::as_str).unwrap_or("").to_string();
        let agent = text_at("agent");
        if agent.is_empty() || agent == me {
            continue;
        }
        out.push(Peer {
            agent,
            harness: text_at("harness"),
            action: text_at("action"),
            path: text_at("path"),
            symbols: record
                .get("symbols")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or_default(),
            branch: text_at("branch"),
            age_s: age.max(0.0),
        });
    }
    out.sort_by(|a, b| a.age_s.partial_cmp(&b.age_s).unwrap_or(std::cmp::Ordering::Equal));
    out
}

pub fn ago(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    if s < 10 {
        "just now".to_string()
    } else if s < 90 {
        format!("{s}s ago")
    } else {
        format!("{}m ago", s / 60)
    }
}
