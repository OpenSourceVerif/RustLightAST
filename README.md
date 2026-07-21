# RustLightAST

A lightweight Rust subset AST crate for Isabelle2Rust.

This crate contains only the RustLight AST definitions and the Rust source
printer. The Isabelle2Rust optimizer, parser, command-line tool, and tests live
under `../Isabelle2Rust/optimize` and use this crate as a path dependency.
