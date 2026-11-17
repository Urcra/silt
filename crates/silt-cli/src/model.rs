//! `silt model`: the local models on this machine (ADR 0092).

use std::io::{IsTerminal, Write};
use std::path::Path;
use std::time::Duration;

use clap::Subcommand;
use exn::{Exn, ResultExt};
use silt_daemon::protocol::{FoundModel, LocalModels, ModelFiles, ModelFit, Request};

use crate::CliError;
use crate::commands::{call, ensure_daemon, fail};

#[derive(Debug, Subcommand)]
pub enum ModelCommand {
    /// List the models here and what this machine can run
    List,
    /// Find GGUF models on Hugging Face, the most downloaded first; with no words, the most downloaded of all
    Search { words: Vec<String> },
    /// List a Hugging Face model's quantizations, with whether each fits here
    Files { repo: String },
    /// Download a model: `owner/repo:file.gguf` from Hugging Face, or a GGUF file you have
    Pull { model: String },
    /// Remove a model; a GGUF file of your own stays where it is
    Rm { model: String },
    /// Download llama.cpp's latest build of llama-server for this machine
    Runtime,
}

pub fn run(command: ModelCommand) -> Result<(), Exn<CliError>> {
    ensure_daemon()?;
    match command {
        ModelCommand::List => {
            list(&call(&Request::Models)?);
            Ok(())
        }
        ModelCommand::Search { words } => {
            let found: Vec<FoundModel> = call(&Request::ModelSearch { query: words.join(" ") })?;
            if found.is_empty() {
                println!("Nothing found.");
            }
            for model in &found {
                println!("{}  {} downloads", model.repo, model.downloads);
            }
            if !found.is_empty() {
                println!("\n`silt model files <owner/repo>` lists one's quantizations.");
            }
            Ok(())
        }
        ModelCommand::Files { repo } => {
            files(&call(&Request::ModelFiles { repo })?);
            Ok(())
        }
        ModelCommand::Pull { model } => pull(&model),
        ModelCommand::Runtime => {
            let id: String = call(&Request::RuntimePull)?;
            wait_for(&id)
        }
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
    wait_for(&id)
}

/// Shows a download's progress until it ends, and how it ended.
fn wait_for(id: &str) -> Result<(), Exn<CliError>> {
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
                if id == silt_daemon::models::RUNTIME {
                    match &view.server {
                        Some(server) => println!("llama.cpp is ready: {server}"),
                        None => return Err(fail("llama.cpp didn't finish downloading")),
                    }
                    return Ok(());
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
        None => println!("llama-server: not found; `silt model runtime` downloads llama.cpp's, or install one on the PATH"),
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
    if view.models.is_empty() {
        println!();
        println!("Find one with `silt model search <words>`, then `silt model files <owner/repo>`.");
    }
}

fn files(model: &ModelFiles) {
    let about =
        [model.architecture.clone(), model.context_length.map(|length| format!("trained for {length} tokens of context"))];
    let about: Vec<String> = about.into_iter().flatten().collect();
    println!("{}{}", model.repo, if about.is_empty() { String::new() } else { format!(" ({})", about.join(", ")) });
    if model.quantizations.is_empty() {
        println!("  No GGUF files Silt can check: each needs the SHA-256 Hugging Face keeps for it.");
        return;
    }
    for quantization in &model.quantizations {
        let parts = if quantization.parts > 1 { format!(", {} parts", quantization.parts) } else { String::new() };
        println!(
            "  {:<14} {:>8}{parts}  {}, {} context",
            quantization.label,
            size(quantization.size),
            fit(quantization.fit),
            quantization.context
        );
        println!("      silt model pull {}", quantization.source);
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
