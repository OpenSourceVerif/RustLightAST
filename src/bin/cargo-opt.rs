use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rustlightast::parse_and_print_rust_source;

const OPT_DIR_NAME: &str = "opt";

fn main() -> ExitCode {
    match run() {
        Ok(summary) => {
            println!(
                "opt finished: optimized {} file(s) into {}",
                summary.optimized,
                summary.output_root.display()
            );
            println!("generated {}", summary.opt_manifest.display());
            println!("run optimized code with: cargo run-opt");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("opt failed: {err}");
            ExitCode::FAILURE
        }
    }
}

struct Config {
    target: PathBuf,
    output_root: Option<PathBuf>,
}

struct Summary {
    output_root: PathBuf,
    opt_manifest: PathBuf,
    optimized: usize,
    needs_nightly: bool,
}

fn run() -> Result<Summary, String> {
    let config = parse_args()?;
    let package_root = find_package_root(&config.target)?;
    let source_root = package_root.join("src");

    if !source_root.is_dir() {
        return Err(format!(
            "expected a Cargo package with a src directory, but {} does not exist",
            source_root.display()
        ));
    }

    let cwd = env::current_dir().map_err(|err| err.to_string())?;
    let output_root = config
        .output_root
        .map(|path| absolute_path_from(&path, &cwd))
        .unwrap_or_else(|| package_root.join(OPT_DIR_NAME));

    if output_root == package_root {
        return Err("output directory must be different from the source package".to_string());
    }

    let opt_manifest = output_root.join("Cargo.toml");
    let mut summary = Summary {
        output_root,
        opt_manifest,
        optimized: 0,
        needs_nightly: false,
    };
    write_optimized_sources(&source_root, &mut summary)?;
    write_opt_manifest(&package_root, &summary)?;

    if summary.needs_nightly {
        write_nightly_toolchain_file(&package_root)?;
        write_nightly_toolchain_file(&summary.output_root)?;
    }

    if summary.optimized == 0 {
        return Err(format!(
            "no Rust source files found under {}",
            source_root.display()
        ));
    }

    Ok(summary)
}

fn parse_args() -> Result<Config, String> {
    // Cargo invokes external subcommands as `cargo-opt opt ...`.
    // Local Cargo aliases usually invoke this binary through `cargo run`,
    // which passes no subcommand name. Supporting both forms keeps the tool
    // flexible.
    let mut target = None;
    let mut output_root = None;
    let mut args = env::args_os().skip(1).filter(|arg| arg != "opt").peekable();

    while let Some(arg) = args.next() {
        if arg == "--out-dir" {
            let value = args
                .next()
                .ok_or_else(|| "--out-dir requires a path".to_string())?;
            output_root = Some(PathBuf::from(value));
        } else if let Some(value) = arg.to_str().and_then(|arg| arg.strip_prefix("--out-dir=")) {
            output_root = Some(PathBuf::from(value));
        } else if arg == "-h" || arg == "--help" {
            return Err(help_text());
        } else if target.is_none() {
            target = Some(PathBuf::from(arg));
        } else {
            return Err(format!(
                "unexpected argument {}; use: cargo opt [package-path] [--out-dir path]",
                PathBuf::from(arg).display()
            ));
        }
    }

    Ok(Config {
        target: target.unwrap_or(env::current_dir().map_err(|err| err.to_string())?),
        output_root,
    })
}

fn help_text() -> String {
    "usage: cargo opt [package-path] [--out-dir path]\n\
     writes optimized Rust files to <package-path>/opt/src by default\n\
     and generates <package-path>/opt/Cargo.toml for cargo run-opt"
        .to_string()
}

fn find_package_root(start: &Path) -> Result<PathBuf, String> {
    // Walk upward until a Cargo.toml is found, so the tool can be run from any
    // subdirectory inside the target package.
    let start = if start.is_file() {
        start.parent().unwrap_or(start)
    } else {
        start
    };

    let mut current = start
        .canonicalize()
        .map_err(|err| format!("failed to resolve {}: {err}", start.display()))?;

    loop {
        if current.join("Cargo.toml").is_file() {
            return Ok(current);
        }
        if !current.pop() {
            return Err(format!(
                "could not find Cargo.toml from {} or any parent directory",
                start.display()
            ));
        }
    }
}

fn write_optimized_sources(source_root: &Path, summary: &mut Summary) -> Result<(), String> {
    let mut sources = rust_sources_under(source_root)?;
    sources.sort();

    for source_path in sources {
        let relative_path = relative_to(&source_path, source_root);
        let output_path = summary.output_root.join("src").join(&relative_path);

        if optimize_source_file(&source_path, &output_path)? {
            summary.needs_nightly = true;
        }
        println!("optimized src/{}", relative_path.display());
        summary.optimized += 1;
    }

    Ok(())
}

fn rust_sources_under(root: &Path) -> Result<Vec<PathBuf>, String> {
    // Use an explicit stack instead of recursion to collect every Rust file in
    // src, including nested module directories.
    let mut pending = vec![root.to_path_buf()];
    let mut sources = Vec::new();

    while let Some(path) = pending.pop() {
        let entries = fs::read_dir(&path)
            .map_err(|err| format!("failed to read directory {}: {err}", path.display()))?;

        for entry in entries {
            let entry = entry.map_err(|err| err.to_string())?;
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|err| format!("failed to inspect {}: {err}", path.display()))?;

            if file_type.is_dir() {
                pending.push(path);
            } else if file_type.is_file() && is_input_rust_file(&path) {
                sources.push(path);
            }
        }
    }

    Ok(sources)
}

fn optimize_source_file(source_path: &Path, output_path: &Path) -> Result<bool, String> {
    let source = fs::read_to_string(source_path)
        .map_err(|err| format!("failed to read {}: {err}", source_path.display()))?;
    let module_name = module_name_from_path(source_path);
    let needs_nightly = source.contains("#![feature(");

    // Current opt pipeline: Rust source -> rustlight_ast::RustModule
    // -> rustlight_print::RustCodeGenerator. Real optimization passes
    // should be inserted between parsing and printing.
    let (_, printed) = parse_and_print_rust_source(&source, module_name)
        .map_err(|err| format!("failed to parse {}: {err}", source_path.display()))?;

    write_file(output_path, printed.as_bytes())?;
    Ok(needs_nightly)
}

fn write_opt_manifest(package_root: &Path, summary: &Summary) -> Result<(), String> {
    let manifest_path = package_root.join("Cargo.toml");
    let manifest = fs::read_to_string(&manifest_path)
        .map_err(|err| format!("failed to read {}: {err}", manifest_path.display()))?;
    let main_path = summary.output_root.join("src").join("main.rs");

    if !main_path.is_file() {
        return Err(format!(
            "cannot generate optimized Cargo.toml: optimized binary entry {} does not exist",
            main_path.display()
        ));
    }

    write_file(&summary.opt_manifest, manifest.as_bytes())
}

fn write_file(output_path: &Path, contents: &[u8]) -> Result<(), String> {
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }

    fs::write(output_path, contents)
        .map_err(|err| format!("failed to write {}: {err}", output_path.display()))
}

fn write_nightly_toolchain_file(package_root: &Path) -> Result<(), String> {
    let toolchain_path = package_root.join("rust-toolchain.toml");
    if toolchain_path.exists() {
        return Ok(());
    }

    write_file(&toolchain_path, b"[toolchain]\nchannel = \"nightly\"\n")
}

fn is_input_rust_file(path: &Path) -> bool {
    path.extension() == Some(OsStr::new("rs")) && !is_generated_rust_file(path)
}

fn is_generated_rust_file(path: &Path) -> bool {
    path.extension() == Some(OsStr::new("rs"))
        && path
            .file_stem()
            .and_then(OsStr::to_str)
            .is_some_and(|stem| stem.ends_with("-opt"))
}

fn module_name_from_path(source_path: &Path) -> String {
    let stem = source_path
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("module");
    sanitize_module_name(stem)
}

fn sanitize_module_name(stem: &str) -> String {
    // The AST stores a module name, so derive a conservative Rust identifier
    // from the file stem even if the file name contains punctuation.
    let mut out = String::new();

    for (idx, ch) in stem.chars().enumerate() {
        let valid = ch == '_' || ch.is_ascii_alphanumeric();
        if idx == 0 && ch.is_ascii_digit() {
            out.push('_');
        }
        out.push(if valid { ch } else { '_' });
    }

    if out.is_empty() {
        "module".to_string()
    } else {
        out
    }
}

fn relative_to(path: &Path, base: &Path) -> PathBuf {
    path.strip_prefix(base).unwrap_or(path).to_path_buf()
}

fn absolute_path_from(path: &Path, cwd: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    }
}
