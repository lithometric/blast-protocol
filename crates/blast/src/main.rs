//! blast: the local agent-to-agent protocol.
//!
//! Agents working in one repository can see each other and learn what they
//! broke, with no server, no account, and no model in the loop. See
//! PROTOCOL.md for the wire format and README.md for what it is for.

mod atomic;
mod config;
mod hook;
mod index;
mod peers;
mod radius;
mod settings;
mod verify;

use std::path::{Path, PathBuf};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn here() -> PathBuf {
    config::repo_root(&std::env::current_dir().unwrap_or_default())
}

fn usage() {
    println!(
        "blast {VERSION} — local agent-to-agent protocol

  blast init [--harness claude|cursor|codex]
      Wire the hook into this repo's harness settings and index the tree.

  blast index
      Rebuild the symbol and call graph. Source is parsed in memory and
      dropped; only structure is stored.

  blast radius <path>[::<symbol>]
      What calls this, and which tests cover it. The question every edit
      should have asked.

  blast peers
      Who else is working in this repository right now.

  blast check [<path>...]
      Run the tests covering these files (or the whole radius) and print
      what failed. This is what the hook does for the agent, by hand.

  blast hook
      The harness entry point. Reads one event on stdin. Not for humans.

  blast uninstall
      Remove our hook entries. Nothing else in the settings file is touched.
"
    );
}

fn cmd_init(root: &Path, harness: &str) -> i32 {
    match settings::install(root, harness) {
        Ok(path) => println!("wired the hook into {}", path.display()),
        Err(error) => {
            eprintln!("could not write the settings file: {error}");
            return 1;
        }
    }
    let (built, files, symbols) = index::build(root);
    if let Err(error) = index::write(root, &built) {
        eprintln!("could not write the index: {error}");
        return 1;
    }
    println!("indexed {files} files, {symbols} symbols");
    if let Some(name) = config::coordinator_present(root) {
        println!(
            "note: {name} is configured here and covers the same ground across machines, so blast will stay quiet. Set BLAST_ALONE=1 to run it anyway."
        );
    }
    match verify::command_for(root) {
        Some(command) => println!("tests will run with: {command}"),
        None => println!(
            "no test command detected. Put one in .blast/config.json as {{\"test\": \"...\"}} or the check stays silent."
        ),
    }
    0
}

fn cmd_radius(root: &Path, target: &str) -> i32 {
    let Some(index) = index::read(root) else {
        eprintln!("no index yet: run `blast index`");
        return 1;
    };
    let roots: Vec<String> = if target.contains("::") {
        vec![target.to_string()]
    } else {
        radius::symbols_in(&index, target)
    };
    if roots.is_empty() {
        println!("nothing indexed under {target}");
        return 0;
    }
    let dependents = radius::of(&index, &roots, verify::DEPTH, verify::MAX_DEPENDENTS);
    if dependents.is_empty() {
        println!("{target}: nothing calls this");
    } else {
        println!("{target}: {} dependent(s)", dependents.len());
        for dep in dependents.iter().take(40) {
            let symbol = if dep.symbol.is_empty() { "(file)" } else { dep.symbol.as_str() };
            println!("  hop {}  {:<28} {}", dep.depth, symbol, dep.path);
        }
    }
    let tests = verify::tests_for(&index, &[target.split("::").next().unwrap_or(target).to_string()]);
    if tests.is_empty() {
        println!("no test files cover it");
    } else {
        println!("covered by: {}", tests.join(", "));
    }
    0
}

fn cmd_peers(root: &Path) -> i32 {
    let peers = peers::others(root, "");
    if peers.is_empty() {
        println!("nobody else is working here right now");
        return 0;
    }
    for peer in peers {
        let mut what = if peer.path.is_empty() {
            peer.action.clone()
        } else {
            format!("{} {}", peer.action, peer.path)
        };
        if !peer.symbols.is_empty() {
            what.push_str(&format!(" ({})", peer.symbols.join(", ")));
        }
        let branch = if peer.branch.is_empty() { String::new() } else { format!("  on {}", peer.branch) };
        println!(
            "{}  {}  {}{}  {}",
            &peer.agent[..6.min(peer.agent.len())],
            peer.harness,
            what,
            branch,
            peers::ago(peer.age_s)
        );
    }
    0
}

fn cmd_check(root: &Path, paths: &[String]) -> i32 {
    let Some(index) = index::read(root) else {
        eprintln!("no index yet: run `blast index`");
        return 1;
    };
    let changed: Vec<String> = if paths.is_empty() {
        index.get("files").and_then(|f| f.as_object()).map(|f| f.keys().cloned().collect()).unwrap_or_default()
    } else {
        paths.to_vec()
    };
    let tests = verify::tests_for(&index, &changed);
    if tests.is_empty() {
        println!("no test files cover that");
        return 0;
    }
    println!("running: {}", tests.join(", "));
    verify::run(root, &tests, &changed);
    match verify::take(root) {
        Some(verdict) => {
            let failing = verdict.get("failing").and_then(|f| f.as_array()).cloned().unwrap_or_default();
            if verdict.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                println!("passed");
                0
            } else {
                println!("failed:");
                for name in failing {
                    println!("  {}", name.as_str().unwrap_or(""));
                }
                1
            }
        }
        None => {
            println!("no test command configured");
            0
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = here();
    let code = match args.first().map(String::as_str) {
        Some("hook") => hook::run(),
        Some("init") => {
            let harness = args
                .iter()
                .position(|a| a == "--harness")
                .and_then(|at| args.get(at + 1))
                .cloned()
                .unwrap_or_else(|| "claude".into());
            cmd_init(&root, &harness)
        }
        Some("index") => {
            let (built, files, symbols) = index::build(&root);
            match index::write(&root, &built) {
                Ok(()) => {
                    println!("indexed {files} files, {symbols} symbols");
                    0
                }
                Err(error) => {
                    eprintln!("could not write the index: {error}");
                    1
                }
            }
        }
        Some("radius") => match args.get(1) {
            Some(target) => cmd_radius(&root, target),
            None => {
                eprintln!("usage: blast radius <path>[::<symbol>]");
                1
            }
        },
        Some("peers") => cmd_peers(&root),
        Some("check") => cmd_check(&root, &args[1..]),
        Some("uninstall") => {
            for harness in ["claude", "cursor", "codex"] {
                if let Ok(Some(path)) = settings::uninstall(&root, harness) {
                    println!("removed our entries from {}", path.display());
                }
            }
            0
        }
        // the detached half of the test run; not part of the documented surface
        Some("run-tests") => {
            let root = args.get(1).map(PathBuf::from).unwrap_or_else(|| root.clone());
            let split = |s: Option<&String>| -> Vec<String> {
                s.map(|v| v.split(',').filter(|p| !p.is_empty()).map(str::to_string).collect())
                    .unwrap_or_default()
            };
            verify::run(&root, &split(args.get(2)), &split(args.get(3)));
            0
        }
        Some("--version") | Some("-V") => {
            println!("blast {VERSION}");
            0
        }
        _ => {
            usage();
            0
        }
    };
    std::process::exit(code);
}
