//! Who may read and write an artifact's page data, and what a query of it
//! answers (ADR 0072). Pure, so the daemon, the server and the agent's tool
//! answer the same.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How much a person may do in a thread, lowest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub enum Level {
    /// Anyone who can read the thread, a read-only link included.
    Read,
    /// Anyone signed in whom the thread's visibility admits, as a comment needs.
    Comment,
    /// The owner and the thread's collaborators.
    Collaborator,
    Owner,
    /// A level nobody holds, written `self`: on a prefix whose `{self}` rule
    /// lets each person write their own, nobody else may, the owner included.
    Nobody,
}

impl Level {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "read" => Some(Self::Read),
            "comment" => Some(Self::Comment),
            "collaborator" => Some(Self::Collaborator),
            "owner" => Some(Self::Owner),
            "self" => Some(Self::Nobody),
            _ => None,
        }
    }
}

/// One rule: the levels a path needs, `{self}` in it matching the
/// person's own id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    segments: Vec<String>,
    read: Level,
    write: Level,
}

/// The rules a version's `db` capability declares; none means anyone who
/// can read reads, and anyone who may comment writes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Rules(Vec<Rule>);

impl Rules {
    /// The rules in a `db` capability's config, such as
    /// `{"rules": [{"path": "votes/{self}", "read": "read", "write": "comment"}]}`.
    pub fn of(config: Option<&Value>) -> Result<Self, String> {
        let Some(list) = config.and_then(|config| config.get("rules")) else {
            return Ok(Self::default());
        };
        let list = list.as_array().ok_or("rules is a list")?;
        if list.len() > 64 {
            return Err("at most 64 rules".into());
        }
        let mut rules = Vec::new();
        for rule in list {
            let path = rule.get("path").and_then(Value::as_str).ok_or("each rule has a path")?;
            let segments: Vec<String> = path.split('/').map(str::to_owned).collect();
            if segments.is_empty() || segments.len() > 8 || segments.iter().any(|segment| !valid_pattern(segment)) {
                return Err(format!("{path} isn't a path"));
            }
            let level = |name: &str, default: Level| match rule.get(name) {
                None => Ok(default),
                Some(Value::String(level)) => Level::parse(level).ok_or_else(|| format!("{level} isn't a level")),
                Some(_) => Err(format!("{name} is a level")),
            };
            rules.push(Rule { segments, read: level("read", Level::Read)?, write: level("write", Level::Comment)? });
        }
        Ok(Self(rules))
    }

    /// The levels `path` needs to be read and written by the person whose
    /// id is `me`: the most specific rule that covers it, else the default.
    pub fn needs(&self, path: &str, me: &str) -> (Level, Level) {
        let segments: Vec<&str> = path.split('/').collect();
        self.0
            .iter()
            .filter(|rule| rule.segments.len() <= segments.len())
            .filter(|rule| {
                rule.segments
                    .iter()
                    .zip(&segments)
                    .all(|(pattern, segment)| pattern == segment || (pattern == "{self}" && *segment == me))
            })
            .max_by_key(|rule| rule.segments.len())
            .map_or((Level::Read, Level::Comment), |rule| (rule.read, rule.write))
    }

    pub fn may_read(&self, path: &str, me: &str, level: Level) -> bool {
        private_to(path).is_none_or(|owner| owner == me) && level >= self.needs(path, me).0
    }

    pub fn may_write(&self, path: &str, me: &str, level: Level) -> bool {
        // `_assets` holds what people add to the page, written through its own routes.
        !path.starts_with("_") && private_to(path).is_none_or(|owner| owner == me) && level >= self.needs(path, me).1
    }
}

/// Whose a path under `data/users/<id>/` is: that person's alone, whatever
/// the rules or anyone's level, the owner's included.
fn private_to(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("data/users/")?;
    Some(rest.split('/').next().unwrap_or(rest))
}

fn valid_pattern(segment: &str) -> bool {
    segment == "{self}"
        || ((1..=64).contains(&segment.len()) && segment.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'))
}

/// A filter of a query: a field compared with a value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Where {
    pub field: String,
    /// `==`, `!=`, `<`, `<=`, `>`, `>=`, `in` or `contains`.
    pub op: String,
    pub value: Value,
}

/// A query of the documents directly in a collection.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Query {
    #[serde(default, rename = "where")]
    pub filters: Vec<Where>,
    #[serde(default)]
    pub order_by: Option<String>,
    #[serde(default)]
    pub descending: bool,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// The most documents a query answers.
pub const QUERY_LIMIT: usize = 1000;

/// A document as a query answers it: its id within its collection, its
/// path, and its data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Doc {
    pub id: String,
    pub path: String,
    #[cfg_attr(feature = "ts", ts(type = "unknown"))]
    pub data: Value,
}

/// The documents directly in `collection` that match `query`, in order.
pub fn query<'a>(docs: impl IntoIterator<Item = (&'a String, &'a Value)>, collection: &str, query: &Query) -> Vec<Doc> {
    let prefix = format!("{collection}/");
    let mut found: Vec<Doc> = docs
        .into_iter()
        .filter_map(|(path, data)| {
            let id = path.strip_prefix(&prefix)?;
            (!id.contains('/')).then(|| Doc { id: id.to_owned(), path: path.clone(), data: data.clone() })
        })
        .filter(|doc| query.filters.iter().all(|filter| matches(&doc.data, filter)))
        .collect();
    if let Some(field) = &query.order_by {
        found.sort_by(|a, b| compare(a.data.get(field), b.data.get(field)));
        if query.descending {
            found.reverse();
        }
    }
    found.truncate(query.limit.unwrap_or(QUERY_LIMIT).min(QUERY_LIMIT));
    found
}

fn matches(data: &Value, filter: &Where) -> bool {
    let field = data.get(&filter.field);
    let ordering = || field.map(|field| compare(Some(field), Some(&filter.value)));
    match filter.op.as_str() {
        "==" => field == Some(&filter.value),
        "!=" => field != Some(&filter.value),
        "<" => ordering() == Some(std::cmp::Ordering::Less),
        "<=" => ordering().is_some_and(|order| order != std::cmp::Ordering::Greater),
        ">" => ordering() == Some(std::cmp::Ordering::Greater),
        ">=" => ordering().is_some_and(|order| order != std::cmp::Ordering::Less),
        "in" => filter.value.as_array().is_some_and(|values| field.is_some_and(|field| values.contains(field))),
        "contains" => field.and_then(Value::as_array).is_some_and(|values| values.contains(&filter.value)),
        _ => false,
    }
}

/// Values in one order: missing, null, booleans, numbers, strings, then the rest by their JSON.
fn compare(a: Option<&Value>, b: Option<&Value>) -> std::cmp::Ordering {
    fn rank(value: Option<&Value>) -> u8 {
        match value {
            None => 0,
            Some(Value::Null) => 1,
            Some(Value::Bool(_)) => 2,
            Some(Value::Number(_)) => 3,
            Some(Value::String(_)) => 4,
            Some(_) => 5,
        }
    }
    match (a, b) {
        (Some(Value::Bool(x)), Some(Value::Bool(y))) => x.cmp(y),
        (Some(Value::Number(x)), Some(Value::Number(y))) => x.as_f64().unwrap_or(0.0).total_cmp(&y.as_f64().unwrap_or(0.0)),
        (Some(Value::String(x)), Some(Value::String(y))) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)).then_with(|| a.map(Value::to_string).cmp(&b.map(Value::to_string))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rules_fall_back_to_reading_for_all_and_writing_for_commenters() {
        let rules = Rules::of(None).expect("none");
        assert!(rules.may_read("tasks/t1", "me", Level::Read));
        assert!(!rules.may_write("tasks/t1", "me", Level::Read));
        assert!(rules.may_write("tasks/t1", "me", Level::Comment));
        assert!(!rules.may_write("_assets/a1", "me", Level::Owner), "assets have their own routes");
    }

    #[test]
    fn everyone_reads_and_each_writes_their_own() {
        let config = json!({"rules": [
            {"path": "votes", "read": "read", "write": "owner"},
            {"path": "votes/{self}", "write": "comment"}
        ]});
        let rules = Rules::of(Some(&config)).expect("rules");
        assert!(rules.may_write("votes/ann", "ann", Level::Comment), "her own");
        assert!(!rules.may_write("votes/bob", "ann", Level::Comment), "not someone else's");
        assert!(rules.may_write("votes/bob", "ann", Level::Owner), "the owner tidies up");
        assert!(rules.may_read("votes/bob", "ann", Level::Read));
        assert!(Rules::of(Some(&json!({"rules": [{"path": "a/../b"}]}))).is_err());
        assert!(Rules::of(Some(&json!({"rules": [{"path": "a", "write": "everyone"}]}))).is_err());
    }

    #[test]
    fn str_replace_edits_text_that_occurs_once() {
        let docs =
            std::collections::BTreeMap::from([("notes/n1".to_owned(), serde_json::json!({"body": "ship on Friday, then rest"}))]);
        let asked = |old: &str| PageRequest::StrReplace {
            path: "notes/n1".into(),
            field: "body".into(),
            old: old.into(),
            new: "Monday".into(),
        };
        let writes = asked("Friday").writes(&docs).expect("replaced");
        assert_eq!(
            writes,
            vec![PageWrite::Update { path: "notes/n1".into(), fields: serde_json::json!({"body": "ship on Monday, then rest"}) }]
        );
        assert!(asked("Sunday").writes(&docs).is_err(), "text that isn't there");
        assert!(asked("").writes(&docs).is_err(), "nothing to find");
        let twice = std::collections::BTreeMap::from([("notes/n1".to_owned(), serde_json::json!({"body": "a a"}))]);
        assert!(asked("a").writes(&twice).is_err(), "text that occurs twice");
    }

    #[test]
    fn a_self_level_holds_for_nobody_and_a_persons_own_paths_are_theirs_alone() {
        let config = json!({"rules": [
            {"path": "signoffs", "read": "read", "write": "self"},
            {"path": "signoffs/{self}", "read": "read", "write": "comment"}
        ]});
        let rules = Rules::of(Some(&config)).expect("rules");
        assert!(rules.may_write("signoffs/ann", "ann", Level::Comment), "her own");
        assert!(!rules.may_write("signoffs/ann", "bob", Level::Owner), "not even the owner");
        assert!(rules.may_read("signoffs/ann", "bob", Level::Read));
        let open = Rules::of(None).expect("none");
        assert!(open.may_write("data/users/ann/notes/n1", "ann", Level::Comment));
        assert!(!open.may_read("data/users/ann/notes/n1", "bob", Level::Owner), "private even from the owner");
        assert!(!open.may_write("data/users/ann", "bob", Level::Owner));
    }

    #[test]
    fn a_query_answers_a_collections_own_documents_filtered_ordered_and_limited() {
        let docs: std::collections::BTreeMap<String, Value> = [
            ("tasks/a", json!({"done": false, "rank": 2, "tags": ["x"]})),
            ("tasks/b", json!({"done": true, "rank": 1})),
            ("tasks/c", json!({"done": false, "rank": 3})),
            ("tasks/c/notes/n1", json!({"text": "deeper"})),
            ("other/z", json!({"done": false})),
        ]
        .into_iter()
        .map(|(path, value)| (path.to_owned(), value))
        .collect();
        let open = Query {
            filters: vec![Where { field: "done".into(), op: "==".into(), value: json!(false) }],
            order_by: Some("rank".into()),
            descending: true,
            limit: Some(5),
        };
        let found: Vec<String> = query(&docs, "tasks", &open).into_iter().map(|doc| doc.id).collect();
        assert_eq!(found, ["c", "a"]);
        let tagged =
            Query { filters: vec![Where { field: "tags".into(), op: "contains".into(), value: json!("x") }], ..Query::default() };
        assert_eq!(query(&docs, "tasks", &tagged).len(), 1);
        let first = Query { limit: Some(1), order_by: Some("rank".into()), ..Query::default() };
        assert_eq!(query(&docs, "tasks", &first)[0].id, "b");
    }
}

/// A read or write of an artifact's page data, as the web UI's bridge and
/// the agent's `silt_artifact_data` ask it (ADR 0072).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PageRequest {
    Get {
        path: String,
    },
    List {
        collection: String,
    },
    Query {
        collection: String,
        #[serde(default)]
        query: Query,
    },
    Set {
        path: String,
        value: Value,
    },
    Update {
        path: String,
        fields: Value,
    },
    Delete {
        path: String,
    },
    /// Several writes, all or none.
    Batch {
        writes: Vec<PageWrite>,
    },
    /// Replaces `old`, which occurs once in the document's text field
    /// `field`, with `new`: an edit of long text without resending it.
    StrReplace {
        path: String,
        field: String,
        old: String,
        new: String,
    },
}

/// One write of a batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PageWrite {
    Set { path: String, value: Value },
    Update { path: String, fields: Value },
    Delete { path: String },
}

/// The most writes one batch takes.
pub const BATCH_WRITES: usize = 100;

impl PageRequest {
    /// Its writes, in order, against `docs` as they stand; none for a read.
    pub fn writes(self, docs: &std::collections::BTreeMap<String, Value>) -> Result<Vec<PageWrite>, String> {
        let writes = match self {
            Self::Get { .. } | Self::List { .. } | Self::Query { .. } => return Ok(Vec::new()),
            Self::Set { path, value } => vec![PageWrite::Set { path, value }],
            Self::Update { path, fields } => vec![PageWrite::Update { path, fields }],
            Self::Delete { path } => vec![PageWrite::Delete { path }],
            Self::Batch { writes } => writes,
            Self::StrReplace { path, field, old, new } => {
                let text = docs
                    .get(&path)
                    .and_then(|doc| doc.get(&field))
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("{path} has no text field {field}"))?;
                if old.is_empty() {
                    return Err("the text to replace is empty".into());
                }
                let mut found = text.match_indices(old.as_str());
                let (at, _) = found.next().ok_or_else(|| format!("{field} doesn't hold that text"))?;
                if found.next().is_some() {
                    return Err(format!("that text occurs more than once in {field}; give more of it"));
                }
                let replaced = format!("{}{new}{}", &text[..at], &text[at + old.len()..]);
                let mut fields = serde_json::Map::new();
                fields.insert(field, Value::String(replaced));
                vec![PageWrite::Update { path, fields: Value::Object(fields) }]
            }
        };
        if writes.len() > BATCH_WRITES {
            return Err(format!("a batch is at most {BATCH_WRITES} writes"));
        }
        Ok(writes)
    }
}

impl PageWrite {
    pub fn path(&self) -> &str {
        match self {
            Self::Set { path, .. } | Self::Update { path, .. } | Self::Delete { path } => path,
        }
    }
}

/// What `request` reads from `docs` for someone at `level` whose id is
/// `me`, under `rules`; `None` for a write.
pub fn read(
    docs: &std::collections::BTreeMap<String, Value>,
    request: &PageRequest,
    rules: &Rules,
    me: &str,
    level: Level,
) -> Option<Value> {
    let readable = |path: &str| rules.may_read(path, me, level);
    let visible = docs.iter().filter(|(path, _)| readable(path));
    Some(match request {
        PageRequest::Get { path } => {
            let doc = docs.get(path).filter(|_| readable(path)).map(|data| Doc {
                id: path.rsplit('/').next().unwrap_or_default().to_owned(),
                path: path.clone(),
                data: data.clone(),
            });
            serde_json::to_value(doc).unwrap_or(Value::Null)
        }
        PageRequest::List { collection } => {
            serde_json::to_value(query(visible, collection, &Query::default())).unwrap_or_default()
        }
        PageRequest::Query { collection, query: asked } => {
            serde_json::to_value(query(visible, collection, asked)).unwrap_or_default()
        }
        _ => return None,
    })
}

/// A person's id in page data, as `{self}` in a rule matches it: the same
/// for their name on every machine and on the server, so a page they write
/// on one is theirs on all. With no server, a machine's person is `local`.
pub fn person_id(user: &str) -> String {
    // FNV-1a, 64 bits: stable everywhere, and no name shows in a path.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in b"silt person\0".iter().chain(user.as_bytes()) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// What a page asks of its assets (ADR 0074).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum AssetRequest {
    /// Adds a file: its name, its type, and its bytes as base64.
    Add {
        name: String,
        media_type: String,
        data: String,
    },
    List,
    Delete {
        id: String,
    },
}

/// An asset as a page reads it: `url` is relative to the page's frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct AssetView {
    pub id: String,
    pub name: String,
    pub media_type: String,
    pub size: u64,
    pub url: String,
}

/// The assets among a page's documents.
pub fn assets(docs: &std::collections::BTreeMap<String, Value>) -> Vec<AssetView> {
    docs.iter()
        .filter_map(|(path, doc)| {
            let id = path.strip_prefix("_assets/")?;
            Some(AssetView {
                id: id.to_owned(),
                name: doc.get("name")?.as_str()?.to_owned(),
                media_type: doc.get("media_type")?.as_str()?.to_owned(),
                size: doc.get("size")?.as_u64()?,
                url: format!("_assets/{id}"),
            })
        })
        .collect()
}

/// The level adding an asset needs: a collaborator's, unless the version
/// lets anyone who may comment add one with `{"upload": "comment"}`.
pub fn asset_level(config: &Value) -> Level {
    match config.get("upload").and_then(Value::as_str) {
        Some("comment") => Level::Comment,
        _ => Level::Collaborator,
    }
}

/// Whether `id` may name an asset: 12 lowercase hex characters, as Silt makes them.
pub fn asset_id_ok(id: &str) -> bool {
    id.len() == 12 && id.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// What a page asks of the thread's files, through `files` (ADR 0078).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FilesRequest {
    /// The files under `path` (the whole tree when empty), only those
    /// directly in it unless `recursive`, after `cursor`.
    List {
        #[serde(default)]
        path: String,
        #[serde(default)]
        recursive: bool,
        #[serde(default)]
        cursor: Option<String>,
    },
    /// A file's text, at the thread's head or `at` a version.
    Read {
        path: String,
        #[serde(default)]
        at: Option<String>,
    },
}

/// The most entries one listing answers.
pub const FILES_PAGE: usize = 500;

/// One page of a listing of `paths`, as `List` asks: the files directly
/// under `path` (or every one below it), then folders once each, and the
/// cursor for the next page.
pub fn list_files<'a>(
    paths: impl IntoIterator<Item = &'a str>,
    path: &str,
    recursive: bool,
    cursor: Option<&str>,
) -> (Vec<Value>, Option<String>) {
    let prefix = match path.trim_matches('/') {
        "" => String::new(),
        path => format!("{path}/"),
    };
    let mut seen = std::collections::BTreeSet::new();
    let mut entries = Vec::new();
    let mut next = None;
    let mut last = None;
    for full in paths {
        let Some(rest) = full.strip_prefix(&prefix) else { continue };
        let entry = match rest.split_once('/') {
            Some((folder, _)) if !recursive => format!("{prefix}{folder}/"),
            _ => full.to_owned(),
        };
        if cursor.is_some_and(|cursor| entry.as_str() <= cursor) || !seen.insert(entry.clone()) {
            continue;
        }
        if entries.len() == FILES_PAGE {
            next = last.take();
            break;
        }
        last = Some(entry.clone());
        let kind = if entry.ends_with('/') { "folder" } else { "file" };
        entries.push(serde_json::json!({"path": entry.trim_end_matches('/'), "kind": kind}));
    }
    (entries, next)
}

/// Whether `path` is among the `paths` globs a `files` declaration names:
/// `*` within a segment, `**` across them; none names the whole tree.
pub fn files_allowed(config: &Value, path: &str) -> bool {
    let Some(globs) = config.get("paths").and_then(Value::as_array) else {
        return true;
    };
    globs.iter().filter_map(Value::as_str).any(|glob| glob_matches(glob, path))
}

/// Whether `path` matches `glob`, `**` standing for any number of segments
/// and `*` for any run within one. Both are matched by table, not by
/// backtracking, so a peer's glob can't take exponential time.
fn glob_matches(glob: &str, path: &str) -> bool {
    let glob: Vec<&str> = glob.trim_matches('/').split('/').collect();
    let path: Vec<&str> = path.trim_matches('/').split('/').collect();
    // `matched[p]`: whether the glob's segments from `g` on match the path's from `p` on.
    let mut matched = vec![false; path.len() + 1];
    matched[path.len()] = true;
    for pattern in glob.iter().rev() {
        let mut next = vec![false; path.len() + 1];
        for at in (0..=path.len()).rev() {
            next[at] = if *pattern == "**" {
                matched[at] || (at < path.len() && next[at + 1])
            } else {
                at < path.len() && matched[at + 1] && segment_matches(pattern, path[at])
            };
        }
        matched = next;
    }
    matched[0]
}

/// Whether `segment` matches `pattern`, whose `*` stand for any run of
/// characters: the usual linear wildcard walk, back to the last `*` only.
fn segment_matches(pattern: &str, segment: &str) -> bool {
    let (pattern, segment): (Vec<char>, Vec<char>) = (pattern.chars().collect(), segment.chars().collect());
    let (mut p, mut s) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while s < segment.len() {
        if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, s));
            p += 1;
        } else if p < pattern.len() && pattern[p] == segment[s] {
            p += 1;
            s += 1;
        } else if let Some((at, from)) = star {
            p = at + 1;
            s = from + 1;
            star = Some((at, from + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|char| *char == '*')
}

/// Who views an artifact's page, as its `user` capability answers it: their
/// id in page data, their level, and the names of the people the thread
/// knows, by id, for the ids a page keeps to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Viewer {
    pub id: String,
    pub level: Level,
    pub people: std::collections::BTreeMap<String, String>,
}

impl Viewer {
    /// The viewer `me` at `level` in a thread whose owner and collaborators
    /// are `people`.
    pub fn of<'a>(me: Option<&str>, level: Level, people: impl IntoIterator<Item = &'a str>) -> Self {
        let id = me.filter(|me| !me.is_empty()).map_or_else(|| "local".to_owned(), person_id);
        let mut known: std::collections::BTreeMap<String, String> =
            people.into_iter().filter(|name| !name.is_empty()).map(|name| (person_id(name), name.to_owned())).collect();
        if let Some(me) = me.filter(|me| !me.is_empty()) {
            known.insert(id.clone(), me.to_owned());
        }
        Self { id, level, people: known }
    }
}

#[cfg(test)]
mod files_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_declarations_globs_name_the_files_a_page_reads() {
        let config = json!({"paths": ["docs/**", "target/criterion/*.json"]});
        assert!(files_allowed(&config, "docs/guide/intro.md"));
        assert!(files_allowed(&config, "target/criterion/report.json"));
        assert!(!files_allowed(&config, "target/criterion/deep/report.json"));
        assert!(!files_allowed(&config, "src/main.rs"));
        assert!(files_allowed(&json!({}), "src/main.rs"), "none names the whole tree");
        assert!(glob_matches("src/*_test.rs", "src/a_test.rs") && !glob_matches("src/*_test.rs", "src/a.rs"));
        assert!(glob_matches("**/*.md", "README.md") && glob_matches("a/**/b/*", "a/x/y/b/c"));
        assert!(glob_matches("*a*b*", "xxaYYbzz") && !glob_matches("*a*b", "xxaYYbzz"));
    }

    #[test]
    fn a_hostile_glob_takes_no_time() {
        let started = std::time::Instant::now();
        let glob = format!("{}x", "**/".repeat(40));
        let path = "a/".repeat(40) + "b";
        assert!(!glob_matches(&glob, &path));
        assert!(!segment_matches(&"*a".repeat(30), &"a".repeat(29)));
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
    }

    #[test]
    fn a_listing_shows_a_folders_files_and_folders_once() {
        let paths = ["README.md", "docs/a.md", "docs/b.md", "docs/deep/c.md", "src/main.rs"];
        let (top, next) = list_files(paths, "", false, None);
        let names: Vec<&str> = top.iter().filter_map(|entry| entry["path"].as_str()).collect();
        assert_eq!(names, ["README.md", "docs", "src"]);
        assert!(next.is_none());
        let (docs, _) = list_files(paths, "docs", true, None);
        assert_eq!(docs.len(), 3);
        let (after, _) = list_files(paths, "docs", true, Some("docs/a.md"));
        assert_eq!(after[0]["path"], "docs/b.md");
    }
}

#[cfg(test)]
mod person_tests {
    #[test]
    fn a_persons_id_is_stable_and_shows_no_name() {
        let id = super::person_id("ann");
        assert_eq!(id, super::person_id("ann"));
        assert_ne!(id, super::person_id("bob"));
        assert_eq!(id.len(), 16);
        assert!(!id.contains("ann"));
    }
}

/// The ops `writes` make on `docs`, all or none: each path written may be,
/// and what's left stays within an artifact's limits.
pub fn page_ops(
    artifact: silt_types::ArtifactId,
    docs: &silt_log::page::Documents,
    writes: Vec<PageWrite>,
    rules: &Rules,
    me: &str,
    level: Level,
) -> Result<Vec<silt_log::PageOp>, String> {
    let mut after = docs.clone();
    let mut ops = Vec::new();
    for write in writes {
        let path = write.path().to_owned();
        if !silt_log::page::is_document_path(&path) {
            return Err(format!("{path} isn't a document's path"));
        }
        if !rules.may_write(&path, me, level) {
            return Err(format!("not_granted: you may not write {path}"));
        }
        let op = match write {
            PageWrite::Set { value, .. } => {
                let text = value.to_string();
                after.insert(path.clone(), value);
                silt_log::PageOp::Set { artifact, path, value: text }
            }
            PageWrite::Update { fields, .. } => {
                let serde_json::Value::Object(object) = &fields else {
                    return Err("an update's fields are an object".into());
                };
                let doc = after.entry(path.clone()).or_insert_with(|| serde_json::json!({}));
                if let Some(target) = doc.as_object_mut() {
                    for (field, value) in object {
                        if value.is_null() {
                            target.remove(field);
                        } else {
                            target.insert(field.clone(), value.clone());
                        }
                    }
                }
                silt_log::PageOp::Patched { artifact, path, fields: fields.to_string() }
            }
            PageWrite::Delete { .. } => {
                after.remove(&path);
                silt_log::PageOp::Deleted { artifact, path }
            }
        };
        if let silt_log::PageOp::Set { value: text, .. } | silt_log::PageOp::Patched { fields: text, .. } = &op
            && text.len() > silt_log::page::DOC_BYTES
        {
            return Err("a document is at most 64 KiB".into());
        }
        ops.push(op);
    }
    if after.len() > silt_log::page::DOCS || silt_log::page::size(&after) > silt_log::page::PAGE_BYTES {
        return Err("an artifact holds at most 10,000 documents and 16 MiB".into());
    }
    Ok(ops)
}
