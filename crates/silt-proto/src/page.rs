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
}

impl Level {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "read" => Some(Self::Read),
            "comment" => Some(Self::Comment),
            "collaborator" => Some(Self::Collaborator),
            "owner" => Some(Self::Owner),
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
        level >= self.needs(path, me).0
    }

    pub fn may_write(&self, path: &str, me: &str, level: Level) -> bool {
        // `_assets` holds what people add to the page, written through its own routes.
        !path.starts_with("_") && level >= self.needs(path, me).1
    }
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
    /// Its writes, in order; none for a read.
    pub fn writes(self) -> Result<Vec<PageWrite>, String> {
        let writes = match self {
            Self::Get { .. } | Self::List { .. } | Self::Query { .. } => return Ok(Vec::new()),
            Self::Set { path, value } => vec![PageWrite::Set { path, value }],
            Self::Update { path, fields } => vec![PageWrite::Update { path, fields }],
            Self::Delete { path } => vec![PageWrite::Delete { path }],
            Self::Batch { writes } => writes,
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
            return Err(format!("you may not write {path}"));
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
