//! Run the tests of everything the edit could have broken, off the turn.
//!
//! The rule this module exists to keep: the agent never waits. A write
//! finishes, we work out which tests cover the blast radius, and we hand
//! that to a detached process. The turn ends immediately. Whatever the tests
//! said is waiting in `.blast/verdict.json` when the next hook event fires,
//! and rides into the model's context then, costing nothing to produce
//! because no model was involved in producing it.

use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

/// How deep to walk callers before deciding which tests matter. Two hops
/// covers "my caller's test" and "my caller's caller's test", which is where
/// the useful signal stops and the noise starts.
pub const DEPTH: usize = 3;
pub const MAX_DEPENDENTS: usize = 200;
pub const MAX_TESTS: usize = 12;

fn looks_like_test(path: &str) -> bool {
    let lower = path.to_lowercase();
    let base = lower.rsplit('/').next().unwrap_or(&lower);
    base.starts_with("test_")
        || base.ends_with("_test.py")
        || base.contains(".test.")
        || base.contains(".spec.")
        || lower.contains("/tests/")
        || lower.contains("/test/")
        || lower.starts_with("tests/")
        || lower.starts_with("test/")
}

/// The test command for this repo: whatever `.blast/config.json` says, else
/// a guess from what is on disk. A wrong guess is why the config key exists.
pub fn command_for(root: &Path) -> Option<String> {
    if let Ok(text) = std::fs::read_to_string(crate::config::dir_of(root).join("config.json")) {
        if let Ok(cfg) = serde_json::from_str::<Value>(&text) {
            if let Some(cmd) = cfg.get("test").and_then(Value::as_str) {
                if !cmd.trim().is_empty() {
                    return Some(cmd.to_string());
                }
            }
        }
    }
    let has = |name: &str| root.join(name).exists();
    if has("pytest.ini") || has("pyproject.toml") || has("setup.cfg") || has("tox.ini") {
        return Some("python -m pytest -q".into());
    }
    if has("Cargo.toml") {
        return Some("cargo test --quiet".into());
    }
    if has("go.mod") {
        return Some("go test ./...".into());
    }
    if has("package.json") {
        return Some("npm test --silent".into());
    }
    None
}

/// Which test files cover the symbols that just changed.
pub fn tests_for(index: &Value, changed: &[String]) -> Vec<String> {
    let mut roots: Vec<String> = Vec::new();
    for path in changed {
        roots.extend(crate::radius::symbols_in(index, path));
    }
    if roots.is_empty() {
        return Vec::new();
    }
    let dependents = crate::radius::of(index, &roots, DEPTH, MAX_DEPENDENTS);
    let mut out: Vec<String> = Vec::new();
    let mut seen = BTreeSet::new();
    // a changed test file is its own test
    for path in changed {
        if looks_like_test(path) && seen.insert(path.clone()) {
            out.push(path.clone());
        }
    }
    for file in crate::radius::files_of(&dependents) {
        if looks_like_test(&file) && seen.insert(file.clone()) {
            out.push(file);
        }
        if out.len() >= MAX_TESTS {
            break;
        }
    }
    out
}

fn verdict_path(root: &Path) -> std::path::PathBuf {
    crate::config::dir_of(root).join("verdict.json")
}

/// Hand the run to a detached copy of ourselves and return at once.
pub fn spawn(root: &Path, tests: &[String], changed: &[String]) {
    if tests.is_empty() {
        return;
    }
    let Ok(exe) = std::env::current_exe() else { return };
    let mut command = Command::new(exe);
    command
        .arg("run-tests")
        .arg(root)
        .arg(tests.join(","))
        .arg(changed.join(","))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // its own session, so the harness reaping the turn does not reap the
        // test run with it
        unsafe {
            command.pre_exec(|| {
                libc_setsid();
                Ok(())
            });
        }
    }
    let _ = command.spawn();
}

#[cfg(unix)]
fn libc_setsid() {
    // avoiding a libc dependency for one call
    extern "C" {
        fn setsid() -> i32;
    }
    unsafe {
        setsid();
    }
}

/// The detached half: run the tests, write what happened.
pub fn run(root: &Path, tests: &[String], changed: &[String]) {
    let Some(base) = command_for(root) else { return };
    let started = crate::peers::now();
    let command = format!("{base} {}", tests.join(" "));
    let output = Command::new("sh")
        .arg("-c")
        .arg(&command)
        .current_dir(root)
        .stdin(Stdio::null())
        .output();
    let Ok(output) = output else { return };
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let failed = failing_lines(&text);
    let record = json!({
        "v": 1,
        "ok": output.status.success(),
        "command": command,
        "changed": changed,
        "tests": tests,
        "failing": failed,
        "elapsed_s": (crate::peers::now() - started * 1.0).max(0.0),
        "ts": crate::peers::now(),
    });
    let _ = crate::atomic::write(
        &verdict_path(root),
        serde_json::to_string(&record).unwrap_or_default().as_bytes(),
    );
}

/// The failing test names a runner printed, in the shapes the common ones
/// use. Names, not tracebacks: the agent needs to know what broke, and can
/// run the test itself if it wants the detail.
fn failing_lines(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let name = if let Some(rest) = line.strip_prefix("FAILED ") {
            Some(rest.split(' ').next().unwrap_or(rest).to_string())
        } else if let Some(rest) = line.strip_prefix("ERROR ") {
            Some(rest.split(' ').next().unwrap_or(rest).to_string())
        } else if line.starts_with("test ") && line.ends_with("... FAILED") {
            Some(line.trim_start_matches("test ").trim_end_matches("... FAILED").trim().to_string())
        } else if line.starts_with("--- FAIL:") {
            Some(line.trim_start_matches("--- FAIL:").trim().split(' ').next().unwrap_or("").to_string())
        } else {
            line.strip_prefix("✕ ")
                .or_else(|| line.strip_prefix("× "))
                .map(|rest| rest.trim().to_string())
        };
        if let Some(name) = name {
            if !name.is_empty() && !out.contains(&name) {
                out.push(name);
            }
        }
        if out.len() >= 20 {
            break;
        }
    }
    out
}

/// Read the verdict and clear it, so one run is reported once.
pub fn take(root: &Path) -> Option<Value> {
    let path = verdict_path(root);
    let text = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_the_usual_test_layouts() {
        for path in ["tests/test_api.py", "src/api.test.ts", "pkg/thing_test.py", "test/unit.rb", "src/a.spec.js"] {
            assert!(looks_like_test(path), "{path}");
        }
        for path in ["booking/api.py", "src/latest.ts", "contest/entry.go"] {
            assert!(!looks_like_test(path), "{path}");
        }
    }

    #[test]
    fn reads_failures_out_of_each_runner() {
        let pytest = failing_lines("FAILED tests/test_api.py::test_create - AssertionError");
        assert_eq!(pytest, vec!["tests/test_api.py::test_create"]);
        let go = failing_lines("--- FAIL: TestCharge (0.00s)");
        assert_eq!(go, vec!["TestCharge"]);
        let vitest = failing_lines("✕ src/pay.test.ts > charges the card");
        assert_eq!(vitest, vec!["src/pay.test.ts > charges the card"]);
        assert!(failing_lines("everything passed").is_empty());
    }

    #[test]
    fn the_same_failure_is_not_listed_twice() {
        let out = failing_lines("FAILED a.py::x\nFAILED a.py::x\nFAILED a.py::y");
        assert_eq!(out, vec!["a.py::x", "a.py::y"]);
    }
}
