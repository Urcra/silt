//! Tasks for working on Silt rather than using it, kept out of the `silt`
//! binary: `cargo xtask wasm-bindgen <wasm> <out-dir>` writes the browser
//! replica's JavaScript bindings, as `wasm-bindgen --target web` would,
//! with the version silt-wasm links, so nobody installs a matching CLI
//! (ADR 0097).

use std::env;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["wasm-bindgen", wasm, out_dir] => wasm_bindgen(Path::new(wasm), Path::new(out_dir)),
        _ => Err("usage: cargo xtask wasm-bindgen <wasm> <out-dir>".to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The settings `wasm-bindgen --target web` runs with where they differ
/// from the library's own defaults.
fn wasm_bindgen(wasm: &Path, out_dir: &Path) -> Result<(), String> {
    let mut bindgen = wasm_bindgen_cli_support::Bindgen::new();
    bindgen.input_path(wasm).typescript(true).omit_default_module_path(false);
    bindgen
        .web(true)
        .and_then(|bindgen| bindgen.generate(out_dir))
        .map_err(|error| format!("wasm-bindgen on {}: {error:#}", wasm.display()))
}
