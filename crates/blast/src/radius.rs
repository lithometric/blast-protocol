//! What breaks if this changes.
//!
//! The index stores edges pointing from a caller to what it calls. Blast
//! radius is that read backwards: start at the symbol somebody just edited
//! and walk the callers, then their callers, breadth first. The answer is
//! the set of files whose tests are worth running, and it is computed from
//! the parser's edges rather than from a name search, so a rename does not
//! silently lose half the callers.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde_json::Value;

pub struct Dependent {
    pub path: String,
    pub symbol: String,
    /// hops from the symbol that changed: 1 is a direct caller
    pub depth: usize,
}

/// caller -> callee, reversed into callee -> callers.
pub fn callers_of(index: &Value) -> BTreeMap<String, Vec<String>> {
    let mut back: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if let Some(edges) = index.get("edges").and_then(Value::as_array) {
        for edge in edges {
            let Some(pair) = edge.as_array() else { continue };
            let (Some(from), Some(to)) = (
                pair.first().and_then(Value::as_str),
                pair.get(1).and_then(Value::as_str),
            ) else {
                continue;
            };
            back.entry(to.to_string()).or_default().push(from.to_string());
        }
    }
    for callers in back.values_mut() {
        callers.sort();
        callers.dedup();
    }
    back
}

fn split(id: &str) -> (String, String) {
    match id.find("::") {
        Some(at) => (id[..at].to_string(), id[at + 2..].to_string()),
        None => (id.to_string(), String::new()),
    }
}

/// Everything that reaches `roots`, out to `depth` hops.
pub fn of(index: &Value, roots: &[String], depth: usize, limit: usize) -> Vec<Dependent> {
    let back = callers_of(index);
    let mut seen: BTreeSet<String> = roots.iter().cloned().collect();
    let mut queue: VecDeque<(String, usize)> = roots.iter().map(|r| (r.clone(), 0)).collect();
    let mut out = Vec::new();
    while let Some((node, at)) = queue.pop_front() {
        if at >= depth || out.len() >= limit {
            continue;
        }
        let Some(callers) = back.get(&node) else { continue };
        for caller in callers {
            if !seen.insert(caller.clone()) {
                continue;
            }
            let (path, symbol) = split(caller);
            out.push(Dependent { path, symbol, depth: at + 1 });
            queue.push_back((caller.clone(), at + 1));
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

/// The symbols a file defines, as node ids.
pub fn symbols_in(index: &Value, path: &str) -> Vec<String> {
    index
        .get("files")
        .and_then(|f| f.get(path))
        .and_then(|f| f.get("symbols"))
        .and_then(Value::as_object)
        .map(|symbols| symbols.keys().map(|name| crate::index::id(path, name)).collect())
        .unwrap_or_default()
}

/// The distinct files a set of dependents lives in, nearest hop first.
pub fn files_of(dependents: &[Dependent]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for dep in dependents {
        if seen.insert(dep.path.clone()) {
            out.push(dep.path.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// tests/test_api.py reaches charge only through booking/api.py. Finding
    /// it is the entire point: it is the file the agent never opened.
    fn index() -> Value {
        json!({
            "version": 1,
            "files": {
                "booking/pay.py": {"symbols": {"charge": {}}},
                "booking/api.py": {"symbols": {"create_booking": {}}},
                "tests/test_api.py": {"symbols": {"test_create_booking": {}}}
            },
            "edges": [
                ["booking/api.py::create_booking", "booking/pay.py::charge", "call", "certain"],
                ["tests/test_api.py::test_create_booking", "booking/api.py::create_booking", "call", "certain"]
            ]
        })
    }

    #[test]
    fn walks_callers_outward_by_hop() {
        let index = index();
        let found = of(&index, &["booking/pay.py::charge".into()], 3, 50);
        let names: Vec<(&str, usize)> =
            found.iter().map(|d| (d.symbol.as_str(), d.depth)).collect();
        assert_eq!(names, vec![("create_booking", 1), ("test_create_booking", 2)]);
    }

    #[test]
    fn stops_at_the_depth_it_was_given() {
        let index = index();
        let found = of(&index, &["booking/pay.py::charge".into()], 1, 50);
        assert_eq!(found.len(), 1, "one hop means direct callers only");
    }

    #[test]
    fn a_cycle_does_not_loop_forever() {
        let index = json!({
            "version": 1, "files": {},
            "edges": [["a.py::one", "b.py::two", "call", "certain"],
                      ["b.py::two", "a.py::one", "call", "certain"]]
        });
        let found = of(&index, &["a.py::one".into()], 10, 50);
        assert_eq!(found.len(), 1, "each node is visited once");
    }
}
