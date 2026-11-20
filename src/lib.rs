//! Rust source parsing and the RustLight syntax model.

pub mod rustlight_parser;

pub use rustlight_parser::{parse_rust_pattern, parse_rust_source, ParseError};
pub use rustlightast_model::*;
