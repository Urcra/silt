//! Checks over the docs as a whole: every page can be reached from the
//! guide, and every `silt` command they name is in the reference.

use std::collections::{BTreeSet, VecDeque};

use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

/// The pages not reachable by following links from `start`, given each
/// page's links (all relative to `docs/`).
pub fn unreachable<'a>(pages: &[(&'a str, &[String])], start: &str) -> Vec<&'a str> {
    let mut reached = BTreeSet::from([start.to_owned()]);
    let mut queue = VecDeque::from([start.to_owned()]);
    while let Some(page) = queue.pop_front() {
        let links = pages.iter().find(|(name, _)| *name == page).map_or(&[][..], |(_, links)| links);
        for link in links {
            if reached.insert(link.clone()) {
                queue.push_back(link.clone());
            }
        }
    }
    pages.iter().map(|(page, _)| *page).filter(|page| !reached.contains(*page)).collect()
}

/// The commands the reference documents, from its headings: `silt mr
/// create` as `["mr", "create"]`.
pub fn reference_commands(reference: &str) -> BTreeSet<Vec<String>> {
    let mut commands = BTreeSet::new();
    let mut in_heading = false;
    for event in Parser::new(reference) {
        match event {
            Event::Start(Tag::Heading { level, .. }) => in_heading = level != HeadingLevel::H1,
            Event::End(TagEnd::Heading(_)) => in_heading = false,
            Event::Code(code) if in_heading => {
                if let Some(rest) = code.strip_prefix("silt ") {
                    commands.insert(rest.split(' ').map(str::to_owned).collect());
                }
            }
            _ => {}
        }
    }
    commands
}

/// Each `silt` command line a page shows, in inline code or a code block,
/// as the words after `silt`, up to its first argument or option.
pub fn named_commands(markdown: &str) -> Vec<Vec<String>> {
    let mut named = Vec::new();
    let mut in_code_block = false;
    for event in Parser::new(markdown) {
        match event {
            Event::Start(Tag::CodeBlock(_)) => in_code_block = true,
            Event::End(TagEnd::CodeBlock) => in_code_block = false,
            Event::Code(code) => named.extend(commands_in(&code)),
            Event::Text(text) if in_code_block => named.extend(commands_in(&text)),
            _ => {}
        }
    }
    named
}

fn commands_in(text: &str) -> Vec<Vec<String>> {
    text.lines().flat_map(commands_in_line).collect()
}

fn commands_in_line(line: &str) -> Vec<Vec<String>> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| is_silt(token))
        .map(|(at, _)| command_words(&tokens[at + 1..]))
        .filter(|words| !words.is_empty())
        .collect()
}

/// `silt` itself or a path to it, such as `target/debug/silt`, but not a
/// URL that ends in it.
fn is_silt(token: &str) -> bool {
    token == "silt" || (token.ends_with("/silt") && !token.contains("://"))
}

/// The words that name the command, up to its first argument or option,
/// or the punctuation of the sentence it's in.
fn command_words(tokens: &[&str]) -> Vec<String> {
    let mut words = Vec::new();
    for token in tokens {
        let word = token.trim_end_matches([',', '.', ';', ':', ')']);
        if !is_word(word) {
            break;
        }
        words.push(word.to_owned());
        if word.len() != token.len() {
            break;
        }
    }
    words
}

fn is_word(token: &str) -> bool {
    token.starts_with(|c: char| c.is_ascii_lowercase()) && token.chars().all(|c| c.is_ascii_lowercase() || c == '-')
}

/// Whether `words` name a documented command: their longest documented
/// prefix is a command, and the word after it is one of its subcommands
/// when it has any, rather than an argument.
pub fn is_documented(words: &[String], known: &BTreeSet<Vec<String>>) -> bool {
    let Some(length) = (1..=words.len()).rev().find(|length| known.contains(&words[..*length])) else {
        return false;
    };
    let command = &words[..length];
    let has_subcommands = known.iter().any(|other| other.len() > length && other.starts_with(command));
    !(has_subcommands && words.len() > length)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(line: &str) -> Vec<String> {
        line.split(' ').map(str::to_owned).collect()
    }

    #[test]
    fn a_page_is_unreachable_when_no_chain_of_links_leads_to_it() {
        let index = vec!["a.md".to_owned()];
        let a = vec!["b.md".to_owned(), "adr/0001.md".to_owned()];
        let pages: Vec<(&str, &[String])> = vec![("index.md", &index), ("a.md", &a), ("b.md", &[]), ("lost.md", &[])];
        assert_eq!(unreachable(&pages, "index.md"), ["lost.md"]);
    }

    #[test]
    fn the_reference_documents_the_commands_its_headings_name() {
        let reference = "# Command-line reference\n\nEvery `silt` command.\n\n## `silt mr`\n\n### `silt mr create`\n\n- `--title <TITLE>`\n\n## `silt log`\n";
        assert_eq!(reference_commands(reference), BTreeSet::from([words("mr"), words("mr create"), words("log")]));
    }

    #[test]
    fn commands_are_read_from_code_up_to_their_first_argument() {
        let page = "Run `silt fork --turn 3 <thread>`, then `silt mr create`. Not silt prose.\n\n\
                    ```sh\nsilt claude   # or silt codex, silt gemini\nsilt inbox\ntarget/debug/silt log\n```\n\n\
                    `silt <command> --help` and `silt-site` name none.";
        assert_eq!(
            named_commands(page),
            [words("fork"), words("mr create"), words("claude"), words("codex"), words("gemini"), words("inbox"), words("log")]
        );
    }

    #[test]
    fn a_command_is_documented_when_the_reference_has_it_and_its_subcommand() {
        let known = BTreeSet::from([words("mr"), words("mr create"), words("search"), words("server user"), words("server")]);
        assert!(is_documented(&words("mr create"), &known));
        assert!(is_documented(&words("mr"), &known));
        assert!(is_documented(&words("search retry"), &known), "a word after a command without subcommands is its argument");
        assert!(!is_documented(&words("mr frobnicate"), &known));
        assert!(!is_documented(&words("frobnicate"), &known));
        assert!(is_documented(&words("server user"), &known));
    }
}
