//! Run `.tish` with `tish:process`, `tish:fs`, and `tish:shadertoy`.

use std::env;
use std::fs;
use std::path::Path;

use tish_shadertoy::ShadertoyModule;
use tishlang_eval::Evaluator;
use tishlang_parser;

fn main() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    let script_path = args
        .get(1)
        .map(|s| s.as_str())
        .unwrap_or("./src/main.tish");

    let path = Path::new(script_path);
    let source = fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", script_path, e))?;

    let program = tishlang_parser::parse(&source)?;
    let shadertoy = ShadertoyModule;
    let mut eval = Evaluator::with_modules(&[&shadertoy]);
    eval.set_current_dir(path.parent());
    eval.eval_program(&program)?;
    Ok(())
}
