//! An approval's card: what a plugin says its call will do, in blocks Silt
//! draws itself. A plugin describes; it never supplies markup.

use serde::{Deserialize, Serialize};

/// The most a card may hold, so a plugin can't bury what matters.
pub const MAX_BLOCKS: usize = 32;
pub const MAX_TEXT: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Card {
    pub title: String,
    pub blocks: Vec<Block>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    Neutral,
    Good,
    Warn,
    Danger,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Item {
    /// Set apart, such as a commit's id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct FileChange {
    pub path: String,
    pub added: u32,
    pub removed: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Block {
    /// Name and value pairs, such as a branch and the commits it moves
    /// between.
    Fields {
        pairs: Vec<(String, String)>,
    },
    List {
        label: String,
        items: Vec<Item>,
    },
    Diff {
        label: String,
        files: Vec<FileChange>,
    },
    /// Shown as it is, in a fixed-width face.
    Code {
        label: String,
        text: String,
    },
    Note {
        tone: Tone,
        text: String,
    },
}

impl Card {
    /// The card with every hidden character made visible, as Silt shows it.
    pub fn shown(&self) -> Self {
        let show = |text: &str| silt_access::shown(text);
        Self {
            title: show(&self.title),
            blocks: self
                .blocks
                .iter()
                .map(|block| match block {
                    Block::Fields { pairs } => {
                        Block::Fields { pairs: pairs.iter().map(|(name, value)| (show(name), show(value))).collect() }
                    }
                    Block::List { label, items } => Block::List {
                        label: show(label),
                        items: items
                            .iter()
                            .map(|item| Item { key: item.key.as_deref().map(show), text: show(&item.text) })
                            .collect(),
                    },
                    Block::Diff { label, files } => Block::Diff {
                        label: show(label),
                        files: files.iter().map(|file| FileChange { path: show(&file.path), ..file.clone() }).collect(),
                    },
                    Block::Code { label, text } => Block::Code { label: show(label), text: show(text) },
                    Block::Note { tone, text } => Block::Note { tone: *tone, text: show(text) },
                })
                .collect(),
        }
    }

    /// Why Silt won't draw it, if it won't.
    pub fn check(&self) -> Result<(), String> {
        if self.title.trim().is_empty() {
            return Err("a card needs a title".into());
        }
        if self.blocks.len() > MAX_BLOCKS {
            return Err(format!("a card holds at most {MAX_BLOCKS} blocks"));
        }
        let mut texts = vec![self.title.len()];
        for block in &self.blocks {
            texts.push(match block {
                Block::Fields { pairs } => pairs.iter().map(|(name, value)| name.len() + value.len()).sum(),
                Block::List { label, items } => {
                    label.len()
                        + items.iter().map(|item| item.text.len() + item.key.as_ref().map_or(0, String::len)).sum::<usize>()
                }
                Block::Diff { label, files } => label.len() + files.iter().map(|file| file.path.len()).sum::<usize>(),
                Block::Code { label, text } => label.len() + text.len(),
                Block::Note { text, .. } => text.len(),
            });
        }
        if texts.iter().sum::<usize>() > MAX_TEXT * 4 || texts.iter().any(|&length| length > MAX_TEXT) {
            return Err("a card's text is too long to read; put the detail in a code block of the call's result".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_characters_are_made_visible_everywhere() {
        let card = Card {
            title: "Open a pull request\u{202E}".into(),
            blocks: vec![
                Block::Fields { pairs: vec![("title".into(), "Retry\u{200B} uploads".into())] },
                Block::Code { label: "body".into(), text: "a\u{0007}b".into() },
            ],
        };
        let shown = card.shown();
        assert_eq!(shown.title, "Open a pull request⟨U+202E⟩");
        assert_eq!(shown.blocks[0], Block::Fields { pairs: vec![("title".into(), "Retry⟨U+200B⟩ uploads".into())] });
        assert_eq!(shown.blocks[1], Block::Code { label: "body".into(), text: "a⟨U+0007⟩b".into() });
    }

    #[test]
    fn a_card_without_a_title_or_too_long_is_refused() {
        assert!(Card { title: " ".into(), blocks: vec![] }.check().is_err());
        let long = Card { title: "x".into(), blocks: vec![Block::Code { label: String::new(), text: "y".repeat(MAX_TEXT + 1) }] };
        assert!(long.check().is_err());
        let many =
            Card { title: "x".into(), blocks: vec![Block::Note { tone: Tone::Neutral, text: "n".into() }; MAX_BLOCKS + 1] };
        assert!(many.check().is_err());
        assert!(Card { title: "x".into(), blocks: vec![] }.check().is_ok());
    }
}
