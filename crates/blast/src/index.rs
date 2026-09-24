//! The repository as symbols and edges.
//!
//! Source is read, parsed in memory, and dropped. What is written to
//! `.blast/index.json` is structure only: which symbols exist, what they
//! look like, and which of them call which. No file contents, ever — the
//! index of a private repository is safe to read, and that is deliberate.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use blast_core::graph::{node_id, resolve_edges, ImportSpec, SymbolEdges};
use serde_json::{json, Value};

pub const INDEX_VERSION: u32 = 1;

/// Directories never worth walking: build output, dependencies, and our own
/// bookkeeping. Skipping them is most of the speed.
const SKIP_DIRS: &[&str] = &[
    ".git", ".blast", ".collide", "node_modules", "target", "dist", "build", ".next",
    "__pycache__", ".venv", "venv", ".mypy_cache", ".pytest_cache", ".ruff_cache",
    "vendor", ".idea", ".vscode", "coverage", ".tox", ".gradle", "Pods",
];

const MAX_FILE_BYTES: u64 = 512 * 1024;

fn supported(path: &str) -> bool {
    blast_core::langs::specs()
        .iter()
        .any(|spec| spec.exts.iter().any(|ext| path.to_lowercase().ends_with(ext)))
}

/// Every source file under `root`, repo-relative, sorted so an index built
/// twice on the same tree is byte-identical.
pub fn source_files(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else { continue };
            let name = entry.file_name().to_string_lossy().to_string();
            if kind.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) && !name.starts_with('.') {
                    stack.push(path);
                }
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            if entry.metadata().map(|m| m.len() > MAX_FILE_BYTES).unwrap_or(true) {
                continue;
            }
            let Ok(rel) = path.strip_prefix(root) else { continue };
            let rel = rel.to_string_lossy().replace('\\', "/");
            if supported(&rel) {
                out.push(rel);
            }
        }
    }
    out.sort();
    out
}

/// (name, the record we store, the raw edges the parser recorded for it)
type ParsedSymbol = (String, Value, Vec<(String, String, String)>);

struct Parsed {
    language: String,
    doc: String,
    symbols: Vec<ParsedSymbol>,
    imports: Vec<ImportSpec>,
}

fn parse_one(path: &str, content: &str) -> Option<Parsed> {
    let out = blast_core::engine::parse(path, content)?;
    let symbols = out
        .symbols
        .into_iter()
        .map(|s| {
            let record = json!({
                "kind": s.kind,
                "signature": s.signature,
                "span": [s.span.0, s.span.1],
                "params": s.params,
                "doc": s.doc,
                "calls": s.sites.iter().map(|(n, l, a)| json!([n, l, a])).collect::<Vec<_>>(),
            });
            (s.name, record, s.edges)
        })
        .collect();
    Some(Parsed {
        language: out.language.to_string(),
        doc: out.doc,
        symbols,
        imports: out
            .imports
            .into_iter()
            .map(|i| ImportSpec { module: i.module, names: i.names })
            .collect(),
    })
}

/// Build the index. Returns the JSON written and how many files carried
/// symbols, so `blast index` can say something true about what it found.
pub fn build(root: &Path) -> (Value, usize, usize) {
    let files = source_files(root);
    let known: BTreeSet<String> = files.iter().cloned().collect();

    // parse everything first: edge resolution needs to know which names the
    // rest of the repository defines before it can say where a call lands
    let mut parsed: BTreeMap<String, Parsed> = BTreeMap::new();
    for rel in &files {
        let Ok(content) = fs::read_to_string(root.join(rel)) else { continue };
        if let Some(one) = parse_one(rel, &content) {
            parsed.insert(rel.clone(), one);
        }
    }

    let mut name_index: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (rel, one) in &parsed {
        for (name, _, _) in &one.symbols {
            name_index.entry(name.clone()).or_default().push(rel.clone());
        }
    }
    for paths in name_index.values_mut() {
        paths.sort();
        paths.dedup();
    }

    let mut files_json = serde_json::Map::new();
    let mut edges: Vec<Value> = Vec::new();
    let mut symbol_count = 0usize;
    for (rel, one) in &parsed {
        let mut symbols = serde_json::Map::new();
        for (name, record, _) in &one.symbols {
            symbols.insert(name.clone(), record.clone());
            symbol_count += 1;
        }
        files_json.insert(
            rel.clone(),
            json!({"language": one.language, "doc": one.doc, "symbols": Value::Object(symbols)}),
        );

        let views: Vec<SymbolEdges> = one
            .symbols
            .iter()
            .map(|(name, _, edges)| SymbolEdges { name: name.as_str(), edges: edges.as_slice() })
            .collect();
        for edge in resolve_edges(rel, &one.language, &views, &one.imports, &known, &name_index) {
            edges.push(json!([edge.from, edge.to, edge.kind, edge.confidence]));
        }
    }

    let index = json!({
        "version": INDEX_VERSION,
        "files": Value::Object(files_json),
        "edges": edges,
    });
    (index, parsed.len(), symbol_count)
}

pub fn path_of(root: &Path) -> std::path::PathBuf {
    crate::config::dir_of(root).join("index.json")
}

pub fn write(root: &Path, index: &Value) -> std::io::Result<()> {
    let dir = crate::config::dir_of(root);
    fs::create_dir_all(&dir)?;
    crate::atomic::write(&path_of(root), serde_json::to_string(index).unwrap_or_default().as_bytes())
}

pub fn read(root: &Path) -> Option<Value> {
    let text = fs::read_to_string(path_of(root)).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    if value.get("version").and_then(Value::as_u64) == Some(INDEX_VERSION as u64) {
        Some(value)
    } else {
        None
    }
}

/// `path::symbol` for a file's symbol, the id every edge is written in.
pub fn id(path: &str, symbol: &str) -> String {
    node_id(path, symbol)
}
