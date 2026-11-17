//! Design systems: the tokens an artifact is styled with, and the CSS its
//! frame is given (ADR 0071). A system is a `README.md` for the agent and a
//! `tokens.json`, compiled here into custom properties for every theme.

pub mod frame;
pub mod palette;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// A font file, an inlined face, may be at most this large.
pub const FONT_BYTES: usize = 1 << 20;
const FONTS_BYTES: usize = 4 << 20;

/// A system's name: what `silt_artifact`'s `design_system` takes and its
/// folder is called.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SystemName(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} isn't a design system's name: lowercase letters, digits and hyphens, at most 64")]
pub struct BadSystemName(String);

impl SystemName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SystemName {
    type Error = BadSystemName;
    fn try_from(name: String) -> Result<Self, Self::Error> {
        let ok = (1..=64).contains(&name.len())
            && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !name.starts_with('-');
        if ok { Ok(Self(name)) } else { Err(BadSystemName(name)) }
    }
}

impl std::str::FromStr for SystemName {
    type Err = BadSystemName;
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::try_from(name.to_owned())
    }
}

impl From<SystemName> for String {
    fn from(name: SystemName) -> Self {
        name.0
    }
}

impl std::fmt::Display for SystemName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// `tokens.json`, as written. Families other than colour and type are kept
/// as JSON and read leniently, so one bad family doesn't lose the rest.
#[derive(Debug, Clone, Deserialize)]
pub struct Tokens {
    pub name: String,
    #[serde(default)]
    pub color: Option<Colors>,
    #[serde(default, rename = "type")]
    pub typography: Option<Typography>,
    /// More names for tokens, such as the ones MCP Apps hosts give their
    /// views (`color-background-primary`), so a page written for those
    /// reads this system too.
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
    #[serde(flatten)]
    pub rest: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Colors {
    pub themes: Vec<Theme>,
    pub tokens: Vec<Token>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Theme {
    pub id: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Token {
    pub name: String,
    pub value: TokenValue,
    #[serde(default)]
    pub usage: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum TokenValue {
    One(String),
    ByTheme(BTreeMap<String, String>),
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Typography {
    #[serde(default)]
    pub fonts: Vec<Font>,
    #[serde(default)]
    pub families: BTreeMap<String, String>,
    #[serde(default)]
    pub groups: Vec<TypeGroup>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Font {
    pub family: String,
    /// Under the system's `fonts/`; none loads the face from Google Fonts.
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub weight: Option<String>,
    #[serde(default)]
    pub style: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TypeGroup {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub styles: Vec<TypeStyle>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeStyle {
    pub name: String,
    #[serde(default)]
    pub family: Option<String>,
    pub font_size: String,
    #[serde(default)]
    pub line_height: Option<serde_json::Value>,
    #[serde(default)]
    pub font_weight: Option<serde_json::Value>,
    #[serde(default)]
    pub letter_spacing: Option<String>,
    #[serde(default)]
    pub usage: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Family {
    tokens: Vec<Token>,
}

#[derive(Debug, thiserror::Error)]
pub enum InvalidTokens {
    #[error("tokens.json isn't valid: {0}")]
    Json(#[from] serde_json::Error),
}

pub fn parse(json: &str) -> Result<Tokens, InvalidTokens> {
    Ok(serde_json::from_str(json)?)
}

/// The CSS a frame is given, and what was left out of it and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiled {
    pub css: String,
    pub dropped: Vec<String>,
}

/// `components.css` may be at most this large (ADR 0086).
pub const COMPONENTS_CSS_BYTES: usize = 128 << 10;
/// One component's example, or its guidelines, may be at most this large.
pub const COMPONENT_BYTES: usize = 16 << 10;
/// A system holds at most this many components...
pub const MAX_COMPONENTS: usize = 48;
/// ...and their examples and guidelines at most this much together.
pub const COMPONENTS_BYTES: usize = 256 << 10;
/// `cover.html` may be at most this large (ADR 0088).
pub const COVER_BYTES: usize = 64 << 10;
/// One asset under `assets/` may be at most this large...
pub const ASSET_BYTES: usize = 1 << 20;
/// ...a system holds at most this many...
pub const MAX_ASSETS: usize = 64;
/// ...and at most this much together.
pub const ASSETS_BYTES: usize = 4 << 20;

impl Compiled {
    /// Adds a system's `components.css` after its tokens. It's unlayered,
    /// so a library's layered reset (Tailwind's preflight) doesn't undo it,
    /// and it comes before the page, whose own rules win at equal
    /// specificity. A file that could load anything or close the frame's
    /// `<style>` is left out whole.
    pub fn add_components(&mut self, css: &str) {
        if css.trim().is_empty() {
            return;
        }
        match components_css_problem(css) {
            Some(why) => self.dropped.push(format!("components.css {why}")),
            None => {
                let _ = writeln!(self.css, "/* components */\n{}", css.trim_end());
            }
        }
    }
}

fn components_css_problem(css: &str) -> Option<&'static str> {
    if css.len() > COMPONENTS_CSS_BYTES {
        return Some("is larger than 128 KiB");
    }
    let lower = css.to_ascii_lowercase();
    if lower.contains("@import") {
        Some("holds an @import")
    } else if lower.contains("</") {
        Some("holds `</`")
    } else if lower
        .match_indices("url(")
        .any(|(at, _)| !lower[at + 4..].trim_start().trim_start_matches(['"', '\'']).starts_with("data:"))
    {
        Some("loads a url() that isn't a data: URI")
    } else {
        None
    }
}

/// One of a system's components: its example markup, the note that heads
/// it and its guidelines (ADR 0086, ADR 0088).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Component {
    pub name: String,
    pub about: String,
    pub html: String,
    pub guide: Option<String>,
}

/// A component's files as read: `components/<name>.html` and, when there
/// is one, `components/<name>.md`.
#[derive(Debug, Clone, Copy)]
pub struct ComponentFiles<'a> {
    pub name: &'a str,
    pub html: &'a str,
    pub guide: Option<&'a str>,
}

/// Whether `name` can name a component: what names a system.
pub fn valid_component_name(name: &str) -> bool {
    name.parse::<SystemName>().is_ok()
}

/// A system's components from their files, by name, in name order, within
/// the limits, and what was left out and why. Guidelines too large are
/// left out alone; the example stays.
pub fn components<'a>(files: impl IntoIterator<Item = ComponentFiles<'a>>) -> (Vec<Component>, Vec<String>) {
    let mut files: Vec<ComponentFiles<'a>> = files.into_iter().collect();
    files.sort_by_key(|files| files.name);
    let mut out = Vec::new();
    let mut dropped = Vec::new();
    let mut total = 0;
    for ComponentFiles { name, html, guide } in files {
        let guide = match guide {
            Some(guide) if guide.len() > COMPONENT_BYTES => {
                dropped.push(format!("the component {name}'s guidelines are larger than 16 KiB"));
                None
            }
            guide => guide.map(str::trim).filter(|guide| !guide.is_empty()),
        };
        let size = html.len() + guide.map_or(0, str::len);
        if !valid_component_name(name) {
            dropped.push(format!("{name:?} isn't a component's name"));
        } else if html.len() > COMPONENT_BYTES {
            dropped.push(format!("the component {name} is larger than 16 KiB"));
        } else if out.len() == MAX_COMPONENTS {
            dropped.push(format!("the component {name} is past the {MAX_COMPONENTS} a system may hold"));
        } else if total + size > COMPONENTS_BYTES {
            dropped.push(format!("the component {name} is past the 256 KiB a system's components may hold together"));
        } else {
            total += size;
            let about = Some(component_note(html))
                .filter(|note| !note.is_empty())
                .or_else(|| guide.map(first_sentence))
                .unwrap_or_default();
            out.push(Component { name: name.to_owned(), about, html: html.trim().to_owned(), guide: guide.map(str::to_owned) });
        }
    }
    (out, dropped)
}

/// The `<!-- … -->` comment an example opens with, on one line.
fn component_note(html: &str) -> String {
    html.trim_start()
        .strip_prefix("<!--")
        .and_then(|rest| rest.split_once("-->"))
        .map(|(note, _)| note.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default()
}

/// A guideline file's summary: its first sentence, past any heading.
fn first_sentence(guide: &str) -> String {
    let prose =
        guide.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')).collect::<Vec<_>>().join(" ");
    let end = prose.find(". ").map_or(prose.len(), |at| at + 1);
    prose[..end].to_owned()
}

/// A system's cover, or why it can't be drawn.
pub fn cover(html: &str) -> Result<String, String> {
    let html = html.trim();
    if html.len() > COVER_BYTES { Err("cover.html is larger than 64 KiB".into()) } else { Ok(html.to_owned()) }
}

/// What an asset is, by its path under `assets/`: `<Group>/<file>` with
/// plain names and a picture's, a PDF's or a group README's extension; none
/// for anything else, which a system doesn't keep.
pub fn asset_type(path: &str) -> Option<&'static str> {
    let (group, file) = path.split_once('/')?;
    let plain = |part: &str| {
        (1..=64).contains(&part.len())
            && part.as_bytes()[0].is_ascii_alphanumeric()
            && part.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
            && !part.contains("..")
    };
    if !plain(group) || !plain(file) {
        return None;
    }
    if file == "README.md" {
        return Some("text/markdown");
    }
    let extension = file.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match extension.as_str() {
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        _ => return None,
    })
}

/// The assets a system keeps of those found, by path and size, in path
/// order, within the limits, and what was left out and why.
pub fn assets(found: impl IntoIterator<Item = (String, usize)>) -> (Vec<(String, &'static str, usize)>, Vec<String>) {
    let mut found: Vec<(String, usize)> = found.into_iter().collect();
    found.sort();
    let mut out = Vec::new();
    let mut dropped = Vec::new();
    let mut total = 0;
    for (path, size) in found {
        match asset_type(&path) {
            None => dropped.push(format!("assets/{path} isn't a picture, a PDF or a group's README")),
            Some(_) if size > ASSET_BYTES => dropped.push(format!("assets/{path} is larger than 1 MiB")),
            Some(_) if out.len() == MAX_ASSETS => {
                dropped.push(format!("assets/{path} is past the {MAX_ASSETS} a system may hold"))
            }
            Some(_) if total + size > ASSETS_BYTES => {
                dropped.push(format!("assets/{path} is past the 4 MiB a system's assets may hold together"))
            }
            Some(media_type) => {
                total += size;
                out.push((path, media_type, size));
            }
        }
    }
    (out, dropped)
}

/// Compiles `tokens` into custom properties: the first theme's on `:root`,
/// `light` and `dark` following the viewer through `prefers-color-scheme`
/// and `data-theme`, and any other theme under its own `data-theme`.
/// `fonts` holds the system's font files by their path in `tokens.json`.
pub fn compile(tokens: &Tokens, fonts: &BTreeMap<String, Vec<u8>>) -> Compiled {
    let mut dropped = Vec::new();
    let mut names = BTreeSet::new();
    let mut claim = |name: &str, dropped: &mut Vec<String>| {
        if !valid_name(name) {
            dropped.push(format!("{name:?} isn't a token name"));
            false
        } else if !names.insert(name.to_owned()) {
            dropped.push(format!("{name} is named twice"));
            false
        } else {
            true
        }
    };

    let themes: Vec<String> = tokens
        .color
        .as_ref()
        .map(|colors| colors.themes.iter().map(|theme| theme.id.clone()).filter(|id| valid_name(id)).collect())
        .unwrap_or_default();
    let themes = if themes.is_empty() { vec!["light".to_owned()] } else { themes };
    let mut by_theme: BTreeMap<&str, Vec<(String, String)>> = BTreeMap::new();

    if let Some(colors) = &tokens.color {
        let known: BTreeSet<&str> = colors.tokens.iter().map(|token| token.name.as_str()).collect();
        for token in &colors.tokens {
            if !claim(&token.name, &mut dropped) {
                continue;
            }
            let value_in = |theme: &str| match &token.value {
                TokenValue::One(value) => Some(value.as_str()),
                TokenValue::ByTheme(values) => values.get(theme).or_else(|| values.get(&themes[0])).map(String::as_str),
            };
            let mut values = Vec::new();
            let before = dropped.len();
            for theme in &themes {
                match value_in(theme).map(|value| color_value(value, &token.name, &known)) {
                    Some(Ok(value)) => values.push((theme.as_str(), value)),
                    Some(Err(why)) => dropped.push(format!("{} in {theme}: {why}", token.name)),
                    None => {}
                }
            }
            if values.is_empty() {
                if dropped.len() == before {
                    dropped.push(format!("{} has no value it can use", token.name));
                }
                continue;
            }
            for (theme, value) in values {
                by_theme.entry(theme).or_default().push((token.name.clone(), value));
            }
        }
    }

    let mut plain: Vec<(String, String)> = Vec::new();
    for (family, value) in &tokens.rest {
        if matches!(family.as_str(), "version" | "meta" | "description") {
            continue;
        }
        let Ok(family_tokens) = serde_json::from_value::<Family>(value.clone()) else {
            dropped.push(format!("{family} isn't a list of tokens"));
            continue;
        };
        for token in family_tokens.tokens {
            if !claim(&token.name, &mut dropped) {
                continue;
            }
            match &token.value {
                TokenValue::One(value) if plain_value(value) => plain.push((token.name, value.clone())),
                TokenValue::ByTheme(values) => {
                    for theme in &themes {
                        let Some(value) = values.get(theme).or_else(|| values.get(&themes[0])) else {
                            continue;
                        };
                        if plain_value(value) {
                            by_theme.entry(theme.as_str()).or_default().push((token.name.clone(), value.clone()));
                        } else {
                            dropped.push(format!("{} in {theme}: {value:?} isn't a plain CSS value", token.name));
                        }
                    }
                }
                TokenValue::One(value) => dropped.push(format!("{}: {value:?} isn't a plain CSS value", token.name)),
            }
        }
    }

    let mut css = String::new();
    let typography = tokens.typography.clone().unwrap_or_default();
    if let Some(import) = google_fonts(&typography.fonts, &mut dropped) {
        let _ = writeln!(css, "@import url(\"{import}\");");
    }
    font_faces(&mut css, &typography.fonts, fonts, &mut dropped);

    let first = themes[0].as_str();
    let scheme = |theme: &str| if theme == "dark" { "dark" } else { "light" };
    let block = |css: &mut String, selector: &str, theme: &str, declarations: &[(String, String)]| {
        let _ = writeln!(css, "{selector} {{");
        let _ = writeln!(css, "  color-scheme: {};", scheme(theme));
        for (name, value) in declarations {
            let _ = writeln!(css, "  --{name}: {value};");
        }
        css.push_str("}\n");
    };
    let empty = Vec::new();
    block(&mut css, &format!(":root, :root[data-theme=\"{first}\"]"), first, by_theme.get(first).unwrap_or(&empty));
    for theme in themes.iter().skip(1) {
        let declarations = by_theme.get(theme.as_str()).unwrap_or(&empty);
        let other = match (first, theme.as_str()) {
            ("light", "dark") => Some(("dark", "light")),
            ("dark", "light") => Some(("light", "dark")),
            _ => None,
        };
        if let Some((preference, guard)) = other {
            let _ = writeln!(css, "@media (prefers-color-scheme: {preference}) {{");
            let mut inner = String::new();
            block(&mut inner, &format!(":root:not([data-theme=\"{guard}\"])"), theme, declarations);
            for line in inner.lines() {
                let _ = writeln!(css, "  {line}");
            }
            css.push_str("}\n");
        }
        block(&mut css, &format!(":root[data-theme=\"{theme}\"]"), theme, declarations);
    }

    let mut root = Vec::new();
    for (key, stack) in &typography.families {
        if valid_name(key) && font_stack(stack) {
            root.push((format!("font-{key}"), stack.clone()));
        } else {
            dropped.push(format!("the font family {key:?} isn't a name and a font stack"));
        }
    }
    root.extend(plain);
    for (alias, target) in &tokens.aliases {
        if !valid_name(alias) || !valid_name(target) || !names.contains(target.as_str()) {
            dropped.push(format!("the alias {alias} names no token"));
        } else if names.contains(alias.as_str()) || root.iter().any(|(name, _)| name == alias) {
            dropped.push(format!("the alias {alias} is a token's own name"));
        } else {
            root.push((alias.clone(), format!("var(--{target})")));
        }
    }
    if !root.is_empty() {
        css.push_str(":root {\n");
        for (name, value) in &root {
            let _ = writeln!(css, "  --{name}: {value};");
        }
        css.push_str("}\n");
    }

    for group in &typography.groups {
        for style in &group.styles {
            match type_class(style, group.family.as_deref(), &typography.families) {
                Ok(rule) => css.push_str(&rule),
                Err(why) => dropped.push(format!("the type style {}: {why}", style.name)),
            }
        }
    }
    Compiled { css, dropped }
}

fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes.iter().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

const COLOR_FUNCTIONS: [&str; 9] = ["rgb", "rgba", "hsl", "hsla", "oklch", "oklab", "lab", "lch", "color"];

fn color_value(value: &str, name: &str, known: &BTreeSet<&str>) -> Result<String, String> {
    let value = value.trim();
    if let Some(alias) = value.strip_prefix('{').and_then(|rest| rest.strip_suffix('}')) {
        return if alias == name {
            Err("it names itself".into())
        } else if known.contains(alias) && valid_name(alias) {
            Ok(format!("var(--{alias})"))
        } else {
            Err(format!("{{{alias}}} names no colour token"))
        };
    }
    if let Some(hex) = value.strip_prefix('#') {
        return if matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            Ok(value.to_ascii_lowercase())
        } else {
            Err(format!("{value:?} isn't a hex colour"))
        };
    }
    let function = value.split('(').next().unwrap_or_default();
    let arguments = value.strip_prefix(function).and_then(|rest| rest.strip_prefix('(')).and_then(|rest| rest.strip_suffix(')'));
    match arguments {
        Some(arguments)
            if COLOR_FUNCTIONS.contains(&function)
                && arguments.bytes().all(|b| b.is_ascii_alphanumeric() || b" .,%/+-".contains(&b)) =>
        {
            Ok(value.to_owned())
        }
        _ => Err(format!("{value:?} isn't a hex colour, a colour function of plain numbers or an alias")),
    }
}

/// A single CSS value with nothing that reaches outside it: no `url()`,
/// `var()` or anything that could close the rule.
fn plain_value(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    let mut depth = 0i32;
    let balanced = value.chars().all(|c| {
        depth += match c {
            '(' => 1,
            ')' => -1,
            _ => 0,
        };
        depth >= 0
    }) && depth == 0;
    (1..=400).contains(&value.len())
        && value.bytes().all(|b| b.is_ascii_alphanumeric() || b" #%(),./+_-".contains(&b))
        && balanced
        && !lower.contains("url(")
        && !lower.contains("var(")
        && !lower.contains("expression(")
        && !lower.contains("image(")
}

fn font_stack(stack: &str) -> bool {
    (1..=200).contains(&stack.len())
        && !stack.bytes().any(|b| b";{}<>\\()".contains(&b) || b.is_ascii_control())
        && stack.matches('"').count().is_multiple_of(2)
        && stack.matches('\'').count().is_multiple_of(2)
}

fn family_name(family: &str) -> bool {
    (1..=64).contains(&family.len()) && family.bytes().all(|b| b.is_ascii_alphanumeric() || b == b' ')
}

fn weights(weight: Option<&str>) -> Option<(u16, u16)> {
    let weight = weight.unwrap_or("400").trim();
    let parse = |w: &str| w.trim().parse::<u16>().ok().filter(|w| (1..=1000).contains(w));
    match weight.split_once(' ') {
        Some((low, high)) => Some((parse(low)?, parse(high)?)),
        None => parse(weight).map(|w| (w, w)),
    }
}

fn italic(style: Option<&str>) -> Result<bool, String> {
    match style.unwrap_or("normal") {
        "normal" => Ok(false),
        "italic" => Ok(true),
        other => Err(format!("{other:?} isn't normal or italic")),
    }
}

/// One `css2` URL for every face without a file.
fn google_fonts(fonts: &[Font], dropped: &mut Vec<String>) -> Option<String> {
    let mut families: BTreeMap<&str, BTreeSet<(bool, u16)>> = BTreeMap::new();
    for font in fonts.iter().filter(|font| font.file.is_none()) {
        let (Some((low, high)), Ok(italic)) = (weights(font.weight.as_deref()), italic(font.style.as_deref())) else {
            dropped.push(format!("the font {} has a weight or style that can't be loaded", font.family));
            continue;
        };
        if !family_name(&font.family) || low != high {
            dropped.push(format!("{:?} can't be loaded from Google Fonts by name", font.family));
            continue;
        }
        families.entry(font.family.as_str()).or_default().insert((italic, low));
    }
    if families.is_empty() {
        return None;
    }
    let mut url = String::from("https://fonts.googleapis.com/css2?");
    for (family, faces) in families {
        let tuples: Vec<String> = faces.iter().map(|(italic, weight)| format!("{},{weight}", u8::from(*italic))).collect();
        let _ = write!(url, "family={}:ital,wght@{}&", family.replace(' ', "+"), tuples.join(";"));
    }
    url.push_str("display=swap");
    Some(url)
}

fn font_faces(css: &mut String, fonts: &[Font], files: &BTreeMap<String, Vec<u8>>, dropped: &mut Vec<String>) {
    let mut total = 0;
    for font in fonts {
        let Some(file) = &font.file else {
            continue;
        };
        let Some(bytes) = files.get(file) else {
            dropped.push(format!("the font file {file} isn't in the system"));
            continue;
        };
        let format = match file.rsplit('.').next() {
            Some("woff2") => "woff2",
            Some("woff") => "woff",
            Some("ttf") => "truetype",
            Some("otf") => "opentype",
            _ => {
                dropped.push(format!("{file} isn't a woff2, woff, ttf or otf file"));
                continue;
            }
        };
        let (Some((low, high)), Ok(italic)) = (weights(font.weight.as_deref()), italic(font.style.as_deref())) else {
            dropped.push(format!("the font {} has a weight or style that can't be used", font.family));
            continue;
        };
        if !family_name(&font.family) {
            dropped.push(format!("{:?} isn't a font family's name", font.family));
            continue;
        }
        total += bytes.len();
        if bytes.len() > FONT_BYTES || total > FONTS_BYTES {
            dropped.push(format!("{file} is too large to inline"));
            continue;
        }
        let data = base64::engine::general_purpose::STANDARD.encode(bytes);
        let weight = if low == high { low.to_string() } else { format!("{low} {high}") };
        let style = if italic { "italic" } else { "normal" };
        let _ = writeln!(
            css,
            "@font-face {{ font-family: \"{}\"; src: url(data:font/{format};base64,{data}) format(\"{format}\"); font-weight: {weight}; font-style: {style}; font-display: swap; }}",
            font.family
        );
    }
}

fn length(value: &str) -> bool {
    let value = value.trim();
    let number_end = value.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).unwrap_or(value.len());
    let (number, unit) = value.split_at(number_end);
    number.parse::<f32>().is_ok() && matches!(unit, "" | "px" | "rem" | "em" | "%")
}

fn type_class(style: &TypeStyle, group_family: Option<&str>, families: &BTreeMap<String, String>) -> Result<String, String> {
    if !valid_name(&style.name) {
        return Err("its name isn't a token name".into());
    }
    if !length(&style.font_size) {
        return Err(format!("{:?} isn't a font size", style.font_size));
    }
    let mut rule = format!(".ds-{} {{", style.name);
    if let Some(family) = style.family.as_deref().or(group_family).filter(|family| families.contains_key(*family)) {
        let _ = write!(rule, " font-family: var(--font-{family});");
    }
    let size = if style.font_size.trim().bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        format!("{}px", style.font_size.trim())
    } else {
        style.font_size.trim().to_owned()
    };
    let _ = write!(rule, " font-size: {size};");
    match &style.line_height {
        Some(serde_json::Value::Number(number)) if number.as_f64().is_some_and(|n| n > 0.0 && n < 10.0) => {
            let _ = write!(rule, " line-height: {number};");
        }
        Some(serde_json::Value::String(value)) if length(value) => {
            let _ = write!(rule, " line-height: {};", value.trim());
        }
        None => {}
        Some(other) => return Err(format!("{other} isn't a line height")),
    }
    match &style.font_weight {
        Some(serde_json::Value::Number(number)) if number.as_u64().is_some_and(|w| (1..=1000).contains(&w)) => {
            let _ = write!(rule, " font-weight: {number};");
        }
        Some(serde_json::Value::String(value)) if weights(Some(value)).is_some() => {
            let _ = write!(rule, " font-weight: {};", value.trim());
        }
        None => {}
        Some(other) => return Err(format!("{other} isn't a font weight")),
    }
    if let Some(spacing) = &style.letter_spacing {
        if !length(spacing) {
            return Err(format!("{spacing:?} isn't a letter spacing"));
        }
        let _ = write!(rule, " letter-spacing: {};", spacing.trim());
    }
    rule.push_str(" }\n");
    Ok(rule)
}

/// A token as a page shows it: its family, name, value per theme with
/// colour aliases followed to a value, and usage note (ADR 0088). A value
/// that holds in every theme is under `*`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRow {
    pub family: String,
    pub name: String,
    pub values: BTreeMap<String, String>,
    pub usage: String,
}

/// A type style as a page shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeRow {
    pub name: String,
    pub group: String,
    pub family: String,
    pub font_size: String,
    pub line_height: Option<String>,
    pub font_weight: Option<String>,
    pub letter_spacing: Option<String>,
    pub usage: String,
}

/// A system's themes by id and name, the first being its primary.
pub fn theme_list(tokens: &Tokens) -> Vec<(String, String)> {
    tokens
        .color
        .as_ref()
        .map(|colors| {
            colors
                .themes
                .iter()
                .map(|theme| (theme.id.clone(), if theme.name.is_empty() { theme.id.clone() } else { theme.name.clone() }))
                .collect()
        })
        .unwrap_or_default()
}

/// Every token, colours first then each other family in file order, with
/// values for display. Nothing here is compiled or trusted: a page shows
/// these as text.
pub fn token_rows(tokens: &Tokens) -> Vec<TokenRow> {
    let mut out = Vec::new();
    if let Some(colors) = &tokens.color {
        let themes: Vec<&str> = colors.themes.iter().map(|theme| theme.id.as_str()).collect();
        let by_name: BTreeMap<&str, &Token> = colors.tokens.iter().map(|token| (token.name.as_str(), token)).collect();
        let raw = |token: &Token, theme: Option<&str>| -> Option<String> {
            match &token.value {
                TokenValue::One(value) => Some(value.clone()),
                TokenValue::ByTheme(values) => theme
                    .and_then(|theme| values.get(theme))
                    .or_else(|| themes.first().and_then(|first| values.get(*first)))
                    .or_else(|| values.values().next())
                    .cloned(),
            }
        };
        let resolve = |mut value: String, theme: Option<&str>| {
            for _ in 0..16 {
                let Some(alias) = value.trim().strip_prefix('{').and_then(|rest| rest.strip_suffix('}')).map(str::to_owned)
                else {
                    break;
                };
                match by_name.get(alias.as_str()).and_then(|target| raw(target, theme)) {
                    Some(next) => value = next,
                    None => break,
                }
            }
            value
        };
        for token in &colors.tokens {
            let values: BTreeMap<String, String> = match &token.value {
                TokenValue::One(value) if !value.trim().starts_with('{') => [("*".to_owned(), value.clone())].into(),
                _ if themes.is_empty() => {
                    raw(token, None).map(|value| [("*".to_owned(), resolve(value, None))].into()).unwrap_or_default()
                }
                _ => themes
                    .iter()
                    .filter_map(|theme| raw(token, Some(theme)).map(|value| ((*theme).to_owned(), resolve(value, Some(theme)))))
                    .collect(),
            };
            out.push(TokenRow { family: "color".into(), name: token.name.clone(), values, usage: token.usage.clone() });
        }
    }
    for (family, value) in &tokens.rest {
        if matches!(family.as_str(), "version" | "meta" | "description") {
            continue;
        }
        let Ok(family_tokens) = serde_json::from_value::<Family>(value.clone()) else {
            continue;
        };
        for token in family_tokens.tokens {
            let values = match token.value {
                TokenValue::One(value) => [("*".to_owned(), value)].into(),
                TokenValue::ByTheme(values) => values,
            };
            out.push(TokenRow { family: family.clone(), name: token.name, values, usage: token.usage });
        }
    }
    out
}

/// Every type style, with its family's stack.
pub fn type_rows(tokens: &Tokens) -> Vec<TypeRow> {
    let Some(typography) = &tokens.typography else {
        return Vec::new();
    };
    let text = |value: &Option<serde_json::Value>| {
        value.as_ref().map(|value| match value {
            serde_json::Value::String(text) => text.clone(),
            other => other.to_string(),
        })
    };
    typography
        .groups
        .iter()
        .flat_map(|group| {
            group.styles.iter().map(move |style| {
                let key = style.family.as_deref().or(group.family.as_deref());
                TypeRow {
                    name: style.name.clone(),
                    group: group.name.clone(),
                    family: key.and_then(|key| typography.families.get(key)).cloned().unwrap_or_default(),
                    font_size: style.font_size.clone(),
                    line_height: text(&style.line_height),
                    font_weight: text(&style.font_weight),
                    letter_spacing: style.letter_spacing.clone(),
                    usage: style.usage.clone(),
                }
            })
        })
        .collect()
}

/// What `tokens.json`'s `meta.source` says the system was made from.
pub fn made_from(tokens: &Tokens) -> Option<String> {
    tokens.rest.get("meta")?.get("source")?.as_str().map(str::to_owned)
}

/// The token names a system defines, by family, for the agent's guide.
pub fn token_names(tokens: &Tokens) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    if let Some(colors) = &tokens.color {
        out.insert("color".into(), colors.tokens.iter().map(|token| format!("--{}", token.name)).collect());
    }
    if let Some(typography) = &tokens.typography {
        let mut names: Vec<String> = typography.families.keys().map(|key| format!("--font-{key}")).collect();
        names.extend(typography.groups.iter().flat_map(|group| group.styles.iter().map(|style| format!(".ds-{}", style.name))));
        out.insert("type".into(), names);
    }
    for (family, value) in &tokens.rest {
        if let Ok(family_tokens) = serde_json::from_value::<Family>(value.clone()) {
            out.insert(family.clone(), family_tokens.tokens.iter().map(|token| format!("--{}", token.name)).collect());
        }
    }
    if !tokens.aliases.is_empty() {
        out.insert("aliases".into(), tokens.aliases.iter().map(|(alias, target)| format!("--{alias} = --{target}")).collect());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compiled(json: &str) -> Compiled {
        compile(&parse(json).expect("tokens"), &BTreeMap::new())
    }

    /// Silt's own system, which its repository keeps as any repository
    /// keeps one (ADR 0087).
    struct Nous {
        tokens: String,
        components_css: String,
        components: Vec<(String, String)>,
    }

    fn nous() -> Nous {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.silt/design-systems/nous");
        let read = |path: &std::path::Path| std::fs::read_to_string(path).unwrap_or_else(|_| panic!("{}", path.display()));
        let mut components: Vec<(String, String)> = std::fs::read_dir(dir.join("components"))
            .expect("components")
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_str()?.strip_suffix(".html")?.to_owned();
                Some((name, read(&entry.path())))
            })
            .collect();
        components.sort();
        Nous { tokens: read(&dir.join("tokens.json")), components_css: read(&dir.join("components.css")), components }
    }

    #[test]
    fn light_and_dark_follow_the_viewer_both_ways() {
        let out = compiled(
            r##"{"name":"t","color":{"themes":[{"id":"light"},{"id":"dark"}],"tokens":[
                {"name":"bg","value":{"light":"#FFFFFF","dark":"#111"}},
                {"name":"ink","value":"#222"},
                {"name":"link","value":"{ink}"}]}}"##,
        );
        assert!(out.dropped.is_empty(), "{:?}", out.dropped);
        let css = &out.css;
        assert!(css.contains(":root, :root[data-theme=\"light\"] {\n  color-scheme: light;\n  --bg: #ffffff;\n  --ink: #222;\n  --link: var(--ink);\n}"), "{css}");
        assert!(css.contains("@media (prefers-color-scheme: dark) {\n  :root:not([data-theme=\"light\"]) {\n    color-scheme: dark;\n    --bg: #111;"), "{css}");
        assert!(css.contains(":root[data-theme=\"dark\"] {\n  color-scheme: dark;\n  --bg: #111;\n  --ink: #222;"), "{css}");
    }

    #[test]
    fn nothing_that_reaches_outside_its_value_is_kept() {
        let out = compiled(
            r##"{"name":"t","color":{"themes":[{"id":"light"}],"tokens":[
                {"name":"a","value":"red; } body { background: url(https://x.example/)"},
                {"name":"b","value":"var(--a)"},
                {"name":"c","value":"{c}"},
                {"name":"d","value":"{missing}"},
                {"name":"bad name","value":"#fff"},
                {"name":"e","value":"rgb(1 2 3 / 0.5)"},
                {"name":"e","value":"#000"}]},
              "spacing":{"tokens":[{"name":"s","value":"16px"},{"name":"u","value":"url(x)"},{"name":"v","value":"1px</style><script>"}]},
              "type":{"families":{"sans":"Inter, sans-serif","evil":"x; } * { color: red"},"groups":[]}}"##,
        );
        for gone in ["url(", "var(--a)", "</style", "evil", "red;"] {
            assert!(!out.css.contains(gone), "{gone} in {}", out.css);
        }
        assert!(out.css.contains("--e: rgb(1 2 3 / 0.5);") && out.css.contains("--s: 16px;"), "{}", out.css);
        assert!(out.css.contains("--font-sans: Inter, sans-serif;"), "{}", out.css);
        assert_eq!(out.dropped.len(), 9, "{:?}", out.dropped);
    }

    #[test]
    fn faces_load_from_google_fonts_or_inline_from_the_system() {
        let tokens = parse(
            r#"{"name":"t","type":{"fonts":[
                {"family":"IBM Plex Sans","weight":"400"},{"family":"IBM Plex Sans","weight":"600"},
                {"family":"IBM Plex Sans","weight":"400","style":"italic"},
                {"family":"Acme","file":"fonts/acme.woff2","weight":"300 800"},
                {"family":"Gone","file":"fonts/gone.woff2"}],
              "families":{"sans":"\"IBM Plex Sans\", sans-serif"},
              "groups":[{"name":"Text","family":"sans","styles":[{"name":"body","fontSize":"14px","lineHeight":1.6,"fontWeight":400,"letterSpacing":"-0.01em"}]}]}}"#,
        )
        .expect("tokens");
        let files = BTreeMap::from([("fonts/acme.woff2".to_owned(), b"wOF2".to_vec())]);
        let out = compile(&tokens, &files);
        assert!(
            out.css.starts_with("@import url(\"https://fonts.googleapis.com/css2?family=IBM+Plex+Sans:ital,wght@0,400;0,600;1,400&display=swap\");\n"),
            "{}",
            out.css
        );
        assert!(
            out.css.contains(
                "font-family: \"Acme\"; src: url(data:font/woff2;base64,d09GMg==) format(\"woff2\"); font-weight: 300 800;"
            ),
            "{}",
            out.css
        );
        assert!(out.css.contains(".ds-body { font-family: var(--font-sans); font-size: 14px; line-height: 1.6; font-weight: 400; letter-spacing: -0.01em; }"), "{}", out.css);
        assert_eq!(out.dropped, ["the font file fonts/gone.woff2 isn't in the system"]);
    }

    #[test]
    fn components_css_comes_after_the_tokens_unless_it_reaches_out() {
        let mut out = compiled(r##"{"name":"A","color":{"themes":[{"id":"light"}],"tokens":[{"name":"bg","value":"#fff"}]}}"##);
        out.add_components(".a-button { background: var(--bg); mask: url( 'data:image/svg+xml,x'); }");
        assert!(
            out.css.ends_with("/* components */\n.a-button { background: var(--bg); mask: url( 'data:image/svg+xml,x'); }\n"),
            "{}",
            out.css
        );
        assert!(out.dropped.is_empty(), "{:?}", out.dropped);
        for (css, why) in [
            ("@IMPORT 'x.css';", "@import"),
            ("a::after { content: '</style>'; }", "`</`"),
            ("a { background: URL(https://example.com/x.png) }", "data: URI"),
            (&"a{}".repeat(COMPONENTS_CSS_BYTES), "128 KiB"),
        ] {
            let before = out.css.clone();
            out.add_components(css);
            assert_eq!(out.css, before, "{css:.40} left out whole");
            assert!(out.dropped.last().is_some_and(|dropped| dropped.contains(why)), "{:?}", out.dropped);
        }
    }

    fn files<'a>(name: &'a str, html: &'a str, guide: Option<&'a str>) -> ComponentFiles<'a> {
        ComponentFiles { name, html, guide }
    }

    #[test]
    fn components_are_named_noted_and_kept_within_their_limits() {
        let big = "x".repeat(COMPONENT_BYTES + 1);
        let (kept, dropped) = components([
            files("row", "  <!-- A list row:\n   28px tall. -->\n<a class=\"r\">x</a>\n", None),
            files("Bad name", "<b>x</b>", None),
            files("huge", big.as_str(), None),
            files("button", "<button>x</button>", Some("# Button\n\nStarts an action. Use one primary per view.")),
            files("tag", "<i>t</i>", Some(big.as_str())),
        ]);
        let names: Vec<&str> = kept.iter().map(|component| component.name.as_str()).collect();
        assert_eq!(names, ["button", "row", "tag"]);
        assert_eq!(kept[1].about, "A list row: 28px tall.");
        assert_eq!(kept[1].html, "<!-- A list row:\n   28px tall. -->\n<a class=\"r\">x</a>");
        assert_eq!(kept[0].about, "Starts an action.", "with no note, the guidelines' first sentence");
        assert_eq!(kept[0].guide.as_deref(), Some("# Button\n\nStarts an action. Use one primary per view."));
        assert_eq!(kept[2].guide, None, "guidelines too large go; the example stays");
        assert_eq!(dropped.len(), 3, "{dropped:?}");

        let names: Vec<String> = (0..=MAX_COMPONENTS).map(|n| format!("c{n:02}")).collect();
        let (kept, dropped) = components(names.iter().map(|name| files(name, "<i></i>", None)));
        assert_eq!((kept.len(), dropped.len()), (MAX_COMPONENTS, 1));
        let half = "x".repeat(COMPONENT_BYTES);
        let letters = ["a", "b", "c", "d", "e", "f", "g", "h", "i"];
        let (kept, dropped) = components(letters.iter().map(|name| files(name, &half, Some(&half))));
        assert_eq!((kept.len(), dropped.len()), (8, 1), "{dropped:?}");
    }

    #[test]
    fn tokens_read_for_a_page_follow_aliases_per_theme() {
        let tokens = parse(
            r##"{"name":"A","meta":{"source":"code"},
              "color":{"themes":[{"id":"light","name":"Light"},{"id":"dark"}],"tokens":[
                {"name":"accent","value":{"light":"#d9883a","dark":"#dfa055"},"usage":"Primary."},
                {"name":"link","value":"{accent}","usage":"Links."},
                {"name":"ink","value":"#111"}]},
              "type":{"families":{"sans":"Inter, sans-serif"},"groups":[{"name":"Text","family":"sans","styles":[
                {"name":"body","fontSize":"15px","lineHeight":1.6,"fontWeight":400}]}]},
              "spacing":{"tokens":[{"name":"space-4","value":"16px","usage":"Gutters."}]}}"##,
        )
        .expect("tokens");
        assert_eq!(theme_list(&tokens), [("light".to_owned(), "Light".to_owned()), ("dark".to_owned(), "dark".to_owned())]);
        let rows = token_rows(&tokens);
        let link = rows.iter().find(|row| row.name == "link").expect("link");
        assert_eq!(link.values, [("dark".to_owned(), "#dfa055".to_owned()), ("light".to_owned(), "#d9883a".to_owned())].into());
        assert_eq!(rows.iter().find(|row| row.name == "ink").expect("ink").values, [("*".to_owned(), "#111".to_owned())].into());
        let space = rows.iter().find(|row| row.name == "space-4").expect("space");
        assert_eq!((space.family.as_str(), space.usage.as_str()), ("spacing", "Gutters."));
        let body = &type_rows(&tokens)[0];
        assert_eq!(
            (body.family.as_str(), body.line_height.as_deref(), body.font_weight.as_deref()),
            ("Inter, sans-serif", Some("1.6"), Some("400"))
        );
        assert_eq!(made_from(&tokens).as_deref(), Some("code"));
    }

    #[test]
    fn a_cover_and_assets_are_kept_within_their_limits() {
        assert_eq!(cover("  <svg></svg>\n").as_deref(), Ok("<svg></svg>"));
        assert!(cover(&"x".repeat(COVER_BYTES + 1)).is_err());

        assert_eq!(asset_type("Logos/acme-mark.svg"), Some("image/svg+xml"));
        assert_eq!(asset_type("Icons/README.md"), Some("text/markdown"));
        assert_eq!(asset_type("Images/hero.JPG"), Some("image/jpeg"));
        for refused in
            ["mark.svg", "Logos/../x.svg", "Logos/.hidden.svg", "Logos/a b.svg", "Logos/x.html", "Logos/sub/x.svg", "Logos/x.js"]
        {
            assert_eq!(asset_type(refused), None, "{refused}");
        }
        let (kept, dropped) = assets([
            ("Logos/b.png".to_owned(), 10),
            ("Logos/a.svg".to_owned(), 10),
            ("Logos/big.png".to_owned(), ASSET_BYTES + 1),
            ("Logos/x.html".to_owned(), 1),
        ]);
        assert_eq!(kept, [("Logos/a.svg".to_owned(), "image/svg+xml", 10), ("Logos/b.png".to_owned(), "image/png", 10)]);
        assert_eq!(dropped.len(), 2, "{dropped:?}");
        let many = (0..=MAX_ASSETS).map(|n| (format!("Icons/i{n:02}.svg"), 1));
        assert_eq!(assets(many).0.len(), MAX_ASSETS);
        let heavy = (0..5).map(|n| (format!("Images/i{n}.png"), ASSET_BYTES));
        assert_eq!(assets(heavy).0.len(), 4);
    }

    #[test]
    fn nous_components_read_only_its_tokens_and_use_only_classes_it_defines() {
        let nous = nous();
        let tokens = parse(&nous.tokens).expect("tokens");
        let known: BTreeSet<String> = token_names(&tokens).into_values().flatten().collect();
        let css = nous.components_css.as_str();
        for (at, _) in css.match_indices("var(--") {
            let name: String = css[at + 4..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
            assert!(known.contains(&name), "components.css reads {name}, which Nous doesn't define");
        }
        let mut out = compile(&tokens, &BTreeMap::new());
        out.add_components(css);
        assert!(out.dropped.is_empty(), "{:?}", out.dropped);

        let (kept, dropped) = components(nous.components.iter().map(|(name, html)| files(name, html, None)));
        assert!(dropped.is_empty(), "{dropped:?}");
        assert_eq!(kept.len(), nous.components.len());
        for component in &kept {
            assert!(!component.about.is_empty(), "{} has a note", component.name);
            for class in component.html.split("class=\"").skip(1).filter_map(|rest| rest.split('"').next()) {
                for class in class.split_whitespace().filter(|class| class.starts_with("n-")) {
                    assert!(
                        css.contains(&format!(".{class}")),
                        "{} uses .{class}, which components.css doesn't define",
                        component.name
                    );
                }
            }
        }
    }

    #[test]
    fn nous_compiles_whole() {
        let out = compile(&parse(&nous().tokens).expect("tokens"), &BTreeMap::new());
        assert!(out.dropped.is_empty(), "{:?}", out.dropped);
        assert!(out.css.contains("--bg:") && out.css.contains("--chart-6:"), "{}", out.css);
        assert!(out.css.contains("--chart-texture: repeating-linear-gradient(45deg"), "{:?}", out.dropped);
    }

    /// Nous's tokens are the web UI's: each colour both define has the same
    /// value in both themes.
    #[test]
    fn nous_matches_the_web_ui() {
        let css = include_str!("../../../web/src/nous.css");
        let mut palette = BTreeMap::new();
        let mut defined = BTreeMap::new();
        for line in css.lines().map(str::trim) {
            let Some((name, value)) = line.strip_prefix("--").and_then(|line| line.split_once(": ")) else {
                continue;
            };
            let value = value.trim_end_matches(';');
            if let Some(pair) = value.strip_prefix("light-dark(").and_then(|value| value.strip_suffix(')')) {
                defined.insert(name.to_owned(), pair.to_owned());
            } else if value.starts_with('#') {
                palette.insert(format!("var(--{name})"), value.to_owned());
                defined.insert(name.to_owned(), format!("{value}, {value}"));
            } else if let Some(other) = value.strip_prefix("var(--") {
                defined.insert(name.to_owned(), format!("var(--{other}, var(--{other}"));
            }
        }
        let resolve = |value: &str| palette.get(value.trim()).cloned().unwrap_or_else(|| value.trim().to_owned());
        let tokens = parse(&nous().tokens).expect("tokens");
        let mut compared = 0;
        for token in tokens.color.expect("colours").tokens {
            let Some(pair) = defined.get(&token.name) else {
                continue;
            };
            let TokenValue::ByTheme(values) = &token.value else {
                panic!("{} has one value", token.name);
            };
            let Some((light, dark)) = split_pair(pair) else {
                continue;
            };
            let norm = |value: &str| value.replace(' ', "").replace("0.1)", "0.10)");
            assert_eq!(norm(&resolve(&light)), norm(&values["light"]), "{} in light", token.name);
            assert_eq!(norm(&resolve(&dark)), norm(&values["dark"]), "{} in dark", token.name);
            compared += 1;
        }
        assert!(compared >= 20, "only {compared} compared");
    }

    fn split_pair(pair: &str) -> Option<(String, String)> {
        let mut depth = 0;
        for (at, c) in pair.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                ',' if depth == 0 => return Some((pair[..at].to_owned(), pair[at + 1..].to_owned())),
                _ => {}
            }
        }
        None
    }

    /// Nous's chart colours, and the default palette `artifact-dataviz`
    /// gives a page with no system, pass the palette checks they ask agents
    /// to run (ADR 0073): the categorical palette in order on `bg`, the
    /// first three for all pairs, and the sequential and diverging scales
    /// as ordinal ramps, in both themes.
    #[test]
    fn the_chart_colours_silt_offers_pass_the_palette_checks() {
        use crate::frame::Theme;
        use crate::palette::{Pairs, Srgb, categorical, ordinal};
        let mut palettes: Vec<(&str, BTreeMap<String, [String; 2]>)> = Vec::new();
        let nous: BTreeMap<String, [String; 2]> = parse(&nous().tokens)
            .expect("tokens")
            .color
            .expect("colours")
            .tokens
            .into_iter()
            .filter_map(|token| match token.value {
                TokenValue::ByTheme(values) => Some((token.name, [values.get("light")?.clone(), values.get("dark")?.clone()])),
                TokenValue::One(_) => None,
            })
            .collect();
        palettes.push(("nous", nous));
        // Each table row of the default palette names its token first, then
        // its light and dark values.
        let default: BTreeMap<String, [String; 2]> =
            include_str!("../../silt-agents/skills/artifact-dataviz/references/palette.md")
                .lines()
                .filter_map(|line| {
                    let quoted: Vec<&str> = line.split('`').skip(1).step_by(2).collect();
                    let hexes: Vec<&str> = quoted.iter().copied().filter(|value| value.starts_with('#')).collect();
                    let name = quoted.first().filter(|name| !name.starts_with('#'))?;
                    (line.starts_with('|') && hexes.len() >= 2)
                        .then(|| ((*name).to_owned(), [hexes[0].to_owned(), hexes[1].to_owned()]))
                })
                .collect();
        palettes.push(("the default palette", default));
        for (name, colors) in &palettes {
            for (theme, index) in [(Theme::Light, 0), (Theme::Dark, 1)] {
                let value = |token: &str| {
                    let found = colors.get(token).unwrap_or_else(|| panic!("{name} has {token}"));
                    Srgb::parse(&found[index]).expect("a hex colour")
                };
                let bg = value("bg");
                let slots: Vec<Srgb> = (1..=6).map(|slot| value(&format!("chart-{slot}"))).collect();
                let report = categorical(&slots, bg, theme, Pairs::Adjacent);
                assert!(report.passed(), "{name} in {}:\n{report}", theme.as_str());
                let report = categorical(&slots[..3], bg, theme, Pairs::All);
                assert!(report.passed(), "{name}'s first three in {}:\n{report}", theme.as_str());
                let ramps = [
                    (1..=7).map(|step| format!("chart-seq-{step}")).collect::<Vec<_>>(),
                    (1..=3).rev().map(|step| format!("chart-div-neg-{step}")).collect(),
                    (1..=3).map(|step| format!("chart-div-pos-{step}")).collect(),
                ];
                for ramp in ramps {
                    let steps: Vec<Srgb> = ramp.iter().map(|token| value(token)).collect();
                    let report = ordinal(&steps, bg, theme);
                    assert!(report.passed(), "{name}'s {} in {}:\n{report}", ramp[0], theme.as_str());
                }
            }
        }
    }

    #[test]
    fn aliases_name_tokens_that_exist() {
        let out = compiled(
            r##"{"name":"t","color":{"themes":[{"id":"light"}],"tokens":[{"name":"bg","value":"#fff"}]},
                "radius":{"tokens":[{"name":"radius-md","value":"4px"}]},
                "aliases":{"color-background-primary":"bg","border-radius-md":"radius-md","gone":"missing","bg":"radius-md"}}"##,
        );
        assert!(
            out.css.contains("--color-background-primary: var(--bg);")
                && out.css.contains("--border-radius-md: var(--radius-md);"),
            "{}",
            out.css
        );
        assert_eq!(out.dropped, ["the alias bg is a token's own name", "the alias gone names no token"]);
    }

    #[test]
    fn names_are_checked() {
        assert!(SystemName::try_from("nous".to_owned()).is_ok());
        for bad in ["", "Nous", "../x", "-x", "a b"] {
            assert!(SystemName::try_from(bad.to_owned()).is_err(), "{bad}");
        }
    }
}
