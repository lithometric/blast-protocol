//! The parse and graph engine: source in, symbols and edges out.
//!
//! Nothing here touches the network, the filesystem, or any state. You hand
//! it a path and the file's text; it hands back the symbols that file
//! defines, what they call, and what it imports. Every grammar is compiled
//! in, so the binary that carries this has no runtime dependency at all.
//!
//! Derived from Collide's parse hot path (MIT) and kept deliberately boring:
//! this is the part that has to be right for everything above it to mean
//! anything.

// This crate is vendored from Collide's parse hot path and is kept
// line-comparable with it, so a fix in either tree can be read across
// without a diff full of restyling. Clippy's style lints are therefore
// silenced here rather than applied; correctness lints still apply.
#![allow(
    clippy::manual_pattern_char_comparison,
    clippy::unnecessary_sort_by,
    clippy::type_complexity
)]

pub mod community;
pub mod edit;
pub mod engine;
pub mod graph;
pub mod langs;

pub use engine::{parse, ParseOutput, Symbol};
pub use graph::{node_id, resolve_edges, Edge, ImportSpec, SymbolEdges};
