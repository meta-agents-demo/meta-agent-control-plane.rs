//! Deterministic, explainable metacognition derived from visible retained state.
//!
//! The engine intentionally consumes only the bounded public snapshot projection:
//! task status, progress, dependencies, concise reflections, evidence references,
//! artifacts, and retained event identifiers. It never requests or reconstructs
//! hidden model reasoning.

include!("metacognition/types.rs");
include!("metacognition/analysis.rs");
include!("metacognition/graph.rs");
