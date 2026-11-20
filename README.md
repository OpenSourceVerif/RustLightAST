# RustLightAST

A lightweight Rust subset AST, source parser, and printer.

```toml
[dependencies]
rustlightast = { path = "../RustLightAST" }
```

```rust
use rustlightast::{parse_rust_source, RustCodeGenerator};

fn main() -> Result<(), rustlightast::ParseError> {
    let module = parse_rust_source("fn identity(x: u32) -> u32 { x }", "example")?;
    println!("{}", RustCodeGenerator::new().generate_module_code(&module));
    Ok(())
}
```

- `rustlight_ast` defines the syntax tree.
- `rustlight_parser` converts supported Rust source into that tree.
- `rustlight_print` renders the tree as Rust source.

`ParsedSourceFile::parse` reports a conversion error if the source contains
syntax the model cannot represent. `ParsedSourceFile::declaration_view` exposes
declarations and signatures from
an existing parse. For a standalone source file, use `parse_file_declaration_view`.

Consumers that receive a RustLight model can depend on the model package:

```toml
[dependencies]
rustlightast = { package = "rustlightast-model", path = "../RustLightAST/model" }
```

It provides the syntax tree, printer, and visitors. The parser package re-exports
the same types and can pass them directly to consumers of the model package.

`ParsedSourceFile::visit_original_origins` walks declared occurrences and
their subtrees, including local functions and recognized macro arguments.
An exact model clone keeps the same occurrence identities within its snapshot.

Run tests with `cargo test --workspace --all-targets --locked`.
