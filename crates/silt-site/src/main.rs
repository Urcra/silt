//! `silt-site [docs] [out]`: builds the docs site, by default from `docs`
//! into `target/site`.

use std::path::PathBuf;

use exn::{Exn, ResultExt};

#[derive(Debug, thiserror::Error)]
#[error("could not build the docs site")]
struct BuildFailed;

fn main() -> Result<(), Exn<BuildFailed>> {
    let mut args = std::env::args_os().skip(1);
    let docs = args.next().map_or_else(|| PathBuf::from("docs"), PathBuf::from);
    let out = args.next().map_or_else(|| PathBuf::from("target/site"), PathBuf::from);
    let written = silt_site::build(&docs, &out).or_raise(|| BuildFailed)?;
    println!("wrote {} pages to {}", written.len(), out.display());
    Ok(())
}
