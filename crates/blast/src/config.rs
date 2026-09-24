//! Where we are, and whether we should be here at all.

use std::path::{Path, PathBuf};

/// The directory Blast keeps its index and its peer records in.
pub const DIR: &str = ".blast";

/// The repository root: the nearest ancestor holding a `.git`, else the
/// nearest holding a `.blast`, else the directory itself. Walking up matters
/// because a hook fires with whatever working directory the agent had.
pub fn repo_root(from: &Path) -> PathBuf {
    let mut here = from.to_path_buf();
    loop {
        if here.join(".git").exists() || here.join(DIR).exists() {
            return here;
        }
        match here.parent() {
            Some(parent) if parent != here => here = parent.to_path_buf(),
            _ => return from.to_path_buf(),
        }
    }
}

/// Is a hosted coordinator already running this repo?
///
/// Blast is the local half of a problem that also has a hosted answer. When
/// one is configured here it is a strict superset of what we do — it has the
/// same graph plus everyone else's machines — so we stand down rather than
/// brief the agent twice and bill it for both. This is the whole conflict
/// story: two tools, one of them yields, and the yielding is automatic.
pub fn coordinator_present(root: &Path) -> Option<String> {
    for (dir, name) in [(".collide", "Collide")] {
        if root.join(dir).join("config.json").is_file() {
            return Some(name.to_string());
        }
    }
    None
}

/// Set BLAST_ALONE=1 to keep Blast working even when a coordinator is
/// configured — useful when you want the local check and nothing else.
pub fn forced_alone() -> bool {
    std::env::var("BLAST_ALONE").map(|v| v == "1").unwrap_or(false)
}

pub fn dir_of(root: &Path) -> PathBuf {
    root.join(DIR)
}
