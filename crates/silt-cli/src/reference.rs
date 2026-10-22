//! The command-line reference in Markdown, from `silt`'s own clap
//! definitions, for the docs site (docs/reference/cli.md).

use std::fmt::Write as _;

use clap::{Arg, Command};

pub fn markdown(root: &Command) -> String {
    let mut out = String::from(
        "# Command-line reference\n\nEvery `silt` command, as `silt <command> --help` describes it. \
         Generated from the commands themselves by `silt reference`, so it matches the version it came with.\n",
    );
    section(&mut out, root, &[], 0);
    let hidden: Vec<&Command> = root.get_subcommands().filter(|sub| sub.is_hide_set()).collect();
    if !hidden.is_empty() {
        out.push_str(
            "\n## Run for you\n\nSilt runs these itself, from agents' hooks and configuration and for the docs site, \
             so they're left out of `silt --help`.\n",
        );
        for sub in hidden {
            section(&mut out, sub, &[root.get_name()], 1);
        }
    }
    out
}

fn section(out: &mut String, command: &Command, parents: &[&str], deeper: usize) {
    let mut path = parents.to_vec();
    path.push(command.get_name());
    if !parents.is_empty() {
        let level = "#".repeat((path.len() + deeper).min(6));
        let _ = write!(out, "\n{level} `{}`\n\n", path.join(" "));
        if let Some(about) = command.get_long_about().or_else(|| command.get_about()) {
            let _ = writeln!(out, "{}", about.to_string().trim());
        }
        let args: Vec<&Arg> = command.get_arguments().filter(|arg| shown(arg)).collect();
        if !args.is_empty() {
            out.push('\n');
            for arg in args {
                let help = arg.get_long_help().or_else(|| arg.get_help()).map(|help| one_line(&help.to_string()));
                let _ = writeln!(out, "- `{}`{}", usage(arg), help.map(|help| format!(": {help}")).unwrap_or_default());
            }
        }
    }
    for sub in command.get_subcommands().filter(|sub| !sub.is_hide_set()) {
        section(out, sub, &path, deeper);
    }
}

fn shown(arg: &Arg) -> bool {
    !arg.is_hide_set() && !matches!(arg.get_id().as_str(), "help" | "version")
}

/// How the argument is written: `<NAME>` for a positional one, `--name
/// <VALUE>` or `--flag` for an option.
fn usage(arg: &Arg) -> String {
    let value = arg
        .get_value_names()
        .and_then(|names| names.first())
        .map_or_else(|| arg.get_id().as_str().to_uppercase(), ToString::to_string);
    match arg.get_long() {
        Some(long) if arg.get_action().takes_values() => format!("--{long} <{value}>"),
        Some(long) => format!("--{long}"),
        None => format!("<{value}>"),
    }
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_options_and_positionals_are_written_and_hidden_commands_last() {
        let root = Command::new("silt")
            .subcommand(
                Command::new("auth").about("Sign in").subcommand(
                    Command::new("login")
                        .about("Sign this machine in")
                        .arg(Arg::new("server").long("server").help("The server's URL").required(true))
                        .arg(Arg::new("no_browser").long("no-browser").action(clap::ArgAction::SetTrue))
                        .arg(Arg::new("secret").long("secret").hide(true)),
                ),
            )
            .subcommand(Command::new("hook").about("Called by hooks").hide(true))
            .subcommand(Command::new("show").arg(Arg::new("thread").help("A thread")));
        let text = markdown(&root);
        assert!(text.contains("\n## `silt auth`\n\nSign in\n"), "{text}");
        assert!(
            text.contains(
                "\n### `silt auth login`\n\nSign this machine in\n\n- `--server <SERVER>`: The server's URL\n- `--no-browser`\n"
            ),
            "{text}"
        );
        assert!(text.contains("- `<THREAD>`: A thread"), "{text}");
        assert!(!text.contains("secret"), "{text}");
        let (shown, run_for_you) = text.split_once("\n## Run for you\n").expect("a section for hidden commands");
        assert!(!shown.contains("hook"), "{text}");
        assert!(run_for_you.contains("\n### `silt hook`\n\nCalled by hooks\n"), "{text}");
    }
}
