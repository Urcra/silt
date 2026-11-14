//! `silt model`: the local models on this machine (ADR 0066).

use std::io::{IsTerminal, Write};
use std::path::Path;
use std::time::Duration;

use clap::Subcommand;
use exn::{Exn, ResultExt};
use silt_daemon::protocol::{LocalModels, ModelFit, Request};

use crate::CliError;
use crate::commands::{call, ensure_daemon, fail};

#[derive(Debug, Subcommand)]
pub enum ModelCommand {
    /// List the models here, the catalog's others, and what this machine can run
    List,
    /// Download a model: a catalog name, `owner/repo:file.gguf` from Hugging Face, or a GGUF file you have
    Pull { model: String },
    /// Remove a model; a GGUF file of your own stays where it is
    Rm { model: String },
}

pub fn run(command: ModelCommand) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    match command {
        ModelCommand::List => {
            list(&call(&Request::Models)?);
            Ok(())
        }
        ModelCommand::Pull { model } => pull(&model),
        ModelCommand::Rm { model } => {
            let said: String = call(&Request::ModelRemove { model })?;
            println!("{said}");
            Ok(())
        }
    }
}

fn pull(model: &str) -> Result<(), Exn<CliError>> {
    let path = Path::new(model);
    let source = if path.exists() {
        std::path::absolute(path).or_raise(|| CliError(format!("can't find {model}")))?.display().to_string()
    } else {
        model.to_owned()
    };
    let id: String = call(&Request::ModelPull { source })?;
    let interactive = std::io::stderr().is_terminal();
    loop {
        let view: LocalModels = call(&Request::Models)?;
        match &view.pull {
            Some(pull) if pull.id == id => {
                if interactive && pull.total > 0 {
                    eprint!("\r{id}: {} of {}  ", size(pull.done), size(pull.total));
                    let _ = std::io::stderr().flush();
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            _ => {
                if interactive {
                    eprintln!();
                }
                if let Some(failed) = view.failed.filter(|failed| failed.id == id) {
                    return Err(fail(format!("couldn't download {id}: {}", failed.reason)));
                }
                let Some(model) = view.models.iter().find(|model| model.id == id) else {
                    return Err(fail(format!("{id} didn't finish downloading")));
                };
                println!("{id} is ready: {} ({}, {} tokens of context)", model.name, size(model.size), model.context);
                if model.fit == ModelFit::TooBig {
                    println!("It's larger than this machine's {} for models, so it may not load.", size(view.room));
                }
                return Ok(());
            }
        }
    }
}

fn list(view: &LocalModels) {
    match &view.server {
        Some(server) => println!("llama-server: {server}"),
        None => println!("llama-server: not found; install llama.cpp, or set SILT_LLAMA_SERVER"),
    }
    for device in &view.devices {
        println!("  {}: {} ({})", device.name, device.description, size(device.total));
    }
    println!("Room for models: {}", size(view.room));
    println!();
    if view.models.is_empty() {
        println!("No models here yet.");
    } else {
        println!("Here:");
        for model in &view.models {
            println!("  {}  {}, {}, {} context, {}", model.id, model.name, size(model.size), model.context, fit(model.fit));
            println!("      from {}", model.source);
        }
    }
    if let Some(pull) = &view.pull {
        println!("  {} is downloading: {} of {}", pull.id, size(pull.done), size(pull.total));
    }
    if let Some(failed) = &view.failed {
        println!("  {} failed to download: {}", failed.id, failed.reason);
    }
    if !view.catalog.is_empty() {
        println!();
        println!("In the catalog (`silt model pull <name>`):");
        for model in &view.catalog {
            let tested = if model.tested { "" } else { ", not tested yet" };
            println!("  {}  {}, {}, {}{tested}", model.id, model.name, size(model.size), fit(model.fit));
            println!("      {}", model.about);
        }
    }
}

fn fit(fit: ModelFit) -> &'static str {
    match fit {
        ModelFit::Fits => "fits",
        ModelFit::Tight => "tight",
        ModelFit::TooBig => "too big here",
    }
}

fn size(bytes: u64) -> String {
    const GB: f64 = 1e9;
    if bytes >= 1_000_000_000 { format!("{:.1} GB", bytes as f64 / GB) } else { format!("{} MB", bytes / 1_000_000) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_in_gigabytes_or_megabytes() {
        assert_eq!(size(18_556_689_568), "18.6 GB");
        assert_eq!(size(12_000_000), "12 MB");
    }
}
