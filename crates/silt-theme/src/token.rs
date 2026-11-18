//! What a theme can set: the UI's colour tokens, by their CSS names, and the
//! highlight tags Lezer's class highlighter marks code with.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

macro_rules! names {
    ($(#[$meta:meta])* $name:ident { $($variant:ident = $text:literal: $about:literal,)* }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum $name {
            $($variant,)*
        }

        impl $name {
            pub const ALL: &[Self] = &[$(Self::$variant,)*];

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text,)*
                }
            }

            /// What it colours, for the schema and Settings.
            pub fn about(self) -> &'static str {
                match self {
                    $(Self::$variant => $about,)*
                }
            }

            pub fn from_name(name: &str) -> Option<Self> {
                match name {
                    $($text => Some(Self::$variant),)*
                    _ => None,
                }
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let name = String::deserialize(deserializer)?;
                Self::from_name(&name).ok_or_else(|| serde::de::Error::custom(format!("no such name: {name}")))
            }
        }
    };
}

names! {
    /// A colour token of the UI, as `web/src/nous.css` names it.
    ColorToken {
        Bg = "bg": "Page background.",
        BgElevated = "bg-elevated": "Fields, menus, code and the composer, raised from bg.",
        Surface1 = "surface-1": "Cards and panels on bg; secondary buttons.",
        Surface2 = "surface-2": "Hovered rows and the chosen tab.",
        Surface3 = "surface-3": "Pressed and active surfaces.",
        Fg1 = "fg-1": "Body text. Must reach 4.5:1 on bg and the surfaces.",
        Fg2 = "fg-2": "Secondary text and labels. Must reach 4.5:1 on bg.",
        Fg3 = "fg-3": "Metadata and placeholders. Must reach 4.5:1 on bg.",
        FgOnAccent = "fg-on-accent": "Text on accent, as in primary buttons.",
        Border = "border": "Hairlines around cards and fields.",
        BorderStrong = "border-strong": "Borders of chosen and focused things.",
        BorderSubtle = "border-subtle": "Dividers inside lists; often translucent.",
        Accent = "accent": "The primary action and the focus ring.",
        AccentHover = "accent-hover": "The primary action, hovered.",
        AccentTint = "accent-tint": "Chosen rows and the selection; usually translucent accent.",
        AccentFg = "accent-fg": "Links and accent text. Must reach 4.5:1 on bg.",
        Ok = "ok": "Done, added lines. Must reach 4.5:1 on bg.",
        Warn = "warn": "Warnings as icons, dots and badges. Must reach 3:1 on bg.",
        WarnFg = "warn-fg": "Warnings set in words. Must reach 4.5:1 on bg.",
        Danger = "danger": "Errors and removed lines. Must reach 4.5:1 on bg.",
        Info = "info": "Information and types. Must reach 4.5:1 on bg.",
        OkTint = "ok-tint": "Behind added lines; usually translucent ok.",
        WarnTint = "warn-tint": "Behind warnings; usually translucent warn.",
        DangerTint = "danger-tint": "Behind removed lines and errors; usually translucent danger.",
        InfoTint = "info-tint": "Behind information; usually translucent info.",
        Backdrop = "backdrop": "Behind dialogs; translucent.",
    }
}

names! {
    /// A highlight tag, for the `tok-*` classes Lezer's class highlighter
    /// marks code with.
    SyntaxTag {
        Keyword = "keyword": "Keywords: fn, let, if, return.",
        String = "string": "String literals.",
        StringSpecial = "string-special": "Regular expressions, escapes and special strings.",
        Number = "number": "Numbers.",
        Constant = "constant": "Booleans, atoms and other literals.",
        Comment = "comment": "Comments.",
        Type = "type": "Type, class and namespace names.",
        Macro = "macro": "Macro names.",
        Variable = "variable": "Variable names.",
        Definition = "definition": "A name where it's defined.",
        Property = "property": "Property and field names.",
        Operator = "operator": "Operators.",
        Punctuation = "punctuation": "Brackets, commas and other punctuation.",
        Meta = "meta": "Attributes, annotations and preprocessor lines.",
        Heading = "heading": "Headings in Markdown.",
        Link = "link": "Links and URLs.",
        Emphasis = "emphasis": "Emphasised text in Markdown.",
        Strong = "strong": "Strong text in Markdown.",
    }
}

impl ColorToken {
    /// Text that must be readable, as opposed to fills and lines.
    pub fn is_text(self) -> bool {
        matches!(
            self,
            Self::Fg1
                | Self::Fg2
                | Self::Fg3
                | Self::FgOnAccent
                | Self::AccentFg
                | Self::WarnFg
                | Self::Ok
                | Self::Danger
                | Self::Info
        )
    }

    /// Required in every theme; the rest can be derived from them.
    pub fn required(self) -> bool {
        matches!(self, Self::Bg | Self::Fg1 | Self::Accent)
    }
}
