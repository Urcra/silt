//! `silt credential`: credentials Silt's gateway adds on the way out for a
//! workspace's sandboxed agents (ADR 0093). The agents never see them.

use std::io::Read as _;

use clap::Subcommand;
use exn::{Exn, ResultExt as _};
use silt_daemon::credentials::{self, Source, StoredCredential};
use silt_daemon::paths;

use crate::CliError;
use crate::commands::fail;

#[derive(Debug, Subcommand)]
pub enum CredentialCommand {
    /// Add one; the secret is read from stdin, unless `--command` prints it
    Add {
        /// Its name, such as `work-github`
        name: String,
        /// The workspace whose agents it's for, by id
        #[arg(long)]
        workspace: String,
        /// The host it's sent to, or `*.example.com` for a domain
        #[arg(long)]
        host: String,
        /// Only requests whose path starts with it
        #[arg(long, default_value = "/")]
        path: String,
        /// The header it goes in
        #[arg(long, default_value = "authorization")]
        header: String,
        /// Put before the secret in the header, such as `Bearer` or `token`
        #[arg(long, default_value = "")]
        scheme: String,
        /// A command run on this machine, outside the sandbox, that prints the secret when a run starts, such as `gh auth token`
        #[arg(long)]
        command: Option<String>,
        /// A variable the sandbox gets a placeholder in, for tools that want one before they send anything, such as `GH_TOKEN`
        #[arg(long)]
        placeholder: Option<String>,
    },
    /// List them, without their secrets
    List,
    /// Remove one, and its secret from the keyring
    Remove { name: String },
}

pub fn run(command: CredentialCommand) -> Result<(), Exn<CliError>> {
    let data = paths::data_dir();
    let mut all = credentials::load(&data);
    match command {
        CredentialCommand::Add { name, workspace, host, path, header, scheme, command, placeholder } => {
            if all.iter().any(|stored| stored.name == name) {
                return Err(fail(format!("there's already a credential called {name}; remove it first")));
            }
            let source = match command {
                Some(command) => Source::Command { argv: command.split_whitespace().map(str::to_owned).collect() },
                None => {
                    let mut secret = String::new();
                    std::io::stdin().read_to_string(&mut secret).or_raise(|| CliError("cannot read the secret".into()))?;
                    let secret = secret.trim();
                    if secret.is_empty() {
                        return Err(fail("give the secret on stdin, or `--command` that prints it"));
                    }
                    credentials::set_secret(&name, Some(secret)).map_err(|error| fail(error.to_string()))?;
                    Source::Keyring
                }
            };
            all.push(StoredCredential {
                name: name.clone(),
                workspace,
                host: host.clone(),
                path,
                header,
                scheme,
                source,
                placeholder,
            });
            credentials::save(&data, &all).map_err(|error| fail(error.to_string()))?;
            println!("added {name}: Silt's gateway adds it to requests for {host} from the workspace's sandboxed agents");
        }
        CredentialCommand::List => {
            if all.is_empty() {
                println!("no credentials; `silt credential add` adds one");
            }
            for stored in &all {
                let from = match &stored.source {
                    Source::Keyring => "the keyring".to_owned(),
                    Source::Command { argv } => format!("`{}`", argv.join(" ")),
                };
                let placeholder = stored
                    .placeholder
                    .as_ref()
                    .map(|name| format!(" · sandbox sees {name}=silt-placeholder"))
                    .unwrap_or_default();
                println!(
                    "{}  {} {}{} · {} · from {from}{placeholder}",
                    stored.name, stored.workspace, stored.host, stored.path, stored.header
                );
            }
        }
        CredentialCommand::Remove { name } => {
            let before = all.len();
            all.retain(|stored| stored.name != name);
            if all.len() == before {
                return Err(fail(format!("no credential called {name}")));
            }
            credentials::save(&data, &all).map_err(|error| fail(error.to_string()))?;
            let _ = credentials::set_secret(&name, None);
            println!("removed {name}");
        }
    }
    Ok(())
}
