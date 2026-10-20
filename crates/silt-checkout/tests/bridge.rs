//! Two checkouts of one thread, each with an in-memory "disk", kept in step
//! through one family like the daemon does.

use std::collections::{BTreeMap, HashMap};

use silt_checkout::{Bridge, DiskAction, DiskContent, DiskEntry, DiskFile, ImportFile, import};
use silt_log::{Author, Delta, Family, FileView};
use silt_types::{ActorId, GitOid, ThreadId};

const THREAD: ThreadId = ThreadId(1);

fn oid(byte: u8) -> GitOid {
    GitOid([byte; 20])
}

fn bases() -> HashMap<GitOid, String> {
    HashMap::from([
        (oid(1), "fn main() {\n    println!(\"hi\");\n}\n".to_owned()),
        (oid(2), "# readme\n".to_owned()),
    ])
}

struct Checkout {
    bridge: Bridge,
    disk: BTreeMap<String, String>,
}

impl Checkout {
    fn new(actor: u128, family: &Family) -> Self {
        Self::on_thread(actor, THREAD, family)
    }

    fn on_thread(actor: u128, thread: ThreadId, family: &Family) -> Self {
        let mut checkout = Self {
            bridge: Bridge::new(ActorId(actor), thread, BTreeMap::new()),
            disk: BTreeMap::new(),
        };
        let inodes = family.tree().inodes();
        let actions = checkout.bridge.reconcile(family, &mut bases(), inodes);
        checkout.perform(actions);
        checkout
    }

    fn file(text: &str) -> DiskFile {
        DiskFile {
            content: DiskContent::Text(text.to_owned()),
            executable: false,
            hash: content_hash(text),
            size: text.len() as u64,
            mtime_ns: 0,
        }
    }

    /// The agent or user edited `path` on this checkout's disk.
    fn edit(&mut self, family: &mut Family, path: &str, text: Option<&str>) -> Vec<Delta> {
        match text {
            Some(text) => self.disk.insert(path.to_owned(), text.to_owned()),
            None => self.disk.remove(path),
        };
        let ingested = self
            .bridge
            .ingest(family, &mut bases(), path, text.map(Self::file), &Author::User, 0)
            .expect("ingests");
        self.perform(ingested.actions);
        ingested.deltas
    }

    /// Deltas from elsewhere were applied to the family.
    fn sync(&mut self, family: &Family, deltas: &[Delta]) {
        let inodes: Vec<_> = deltas
            .iter()
            .filter(|delta| !self.bridge.is_own(delta))
            .filter_map(touched_inode)
            .collect();
        let actions = self.bridge.reconcile(family, &mut bases(), inodes);
        self.perform(actions);
    }

    fn perform(&mut self, actions: Vec<DiskAction>) {
        for action in actions {
            match action {
                DiskAction::Write {
                    path,
                    inode,
                    content: DiskContent::Text(text),
                    version,
                    ..
                } => {
                    self.bridge.record_written(
                        &path,
                        DiskEntry {
                            inode,
                            hash: content_hash(&text),
                            size: text.len() as u64,
                            mtime_ns: 0,
                            version,
                        },
                    );
                    self.disk.insert(path, text);
                }
                DiskAction::Write { .. } => panic!("no binary files in this test"),
                DiskAction::Remove { path } => {
                    self.bridge.record_removed(&path);
                    self.disk.remove(&path);
                }
            }
        }
    }
}

fn touched_inode(delta: &Delta) -> Option<silt_crdt::InodeId> {
    use silt_log::Op;
    match &delta.op {
        Op::Create { .. } => Some(silt_crdt::InodeId(delta.span.first())),
        Op::Move { inode, .. }
        | Op::Delete { inode }
        | Op::Text { inode, .. }
        | Op::SetContent { inode, .. }
        | Op::SetExecutable { inode, .. } => Some(*inode),
        Op::Conv(_)
        | Op::Git(_)
        | Op::Lease { .. }
        | Op::Fork { .. }
        | Op::Merge { .. }
        | Op::Mr(_)
        | Op::Comment(_) => None,
    }
}

/// Stand-in for a content hash; only equality matters here.
fn content_hash(text: &str) -> [u8; 32] {
    let mut hash = [0u8; 32];
    for (index, byte) in text.bytes().enumerate() {
        hash[index % 32] = hash[index % 32].wrapping_mul(31).wrapping_add(byte);
    }
    hash
}

fn imported_family() -> Family {
    let mut family = Family::new();
    import(
        &mut family,
        ActorId(100),
        THREAD,
        Some((oid(9), Some("main".into()))),
        &[
            ImportFile {
                path: "README.md".into(),
                oid: oid(2),
                executable: false,
            },
            ImportFile {
                path: "src/main.rs".into(),
                oid: oid(1),
                executable: false,
            },
        ],
        0,
    )
    .expect("imports");
    family
}

fn family_text(family: &Family, path: &str) -> String {
    let inode = family.tree().paths()[path];
    match family.file(inode) {
        Some(FileView::Text(text)) => text,
        Some(FileView::GitText(oid)) => bases()[&oid].clone(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn import_materializes_and_edits_flow_to_the_other_checkout() {
    let mut family = imported_family();
    let mut a = Checkout::new(1, &family);
    let mut b = Checkout::new(2, &family);
    assert_eq!(a.disk["src/main.rs"], bases()[&oid(1)]);

    let deltas = a.edit(&mut family, "src/main.rs", Some("fn main() {\n    println!(\"hello\");\n}\n"));
    assert_eq!(deltas.len(), 2, "a one-word change is one delete and one insert");
    b.sync(&family, &deltas);
    assert_eq!(b.disk["src/main.rs"], "fn main() {\n    println!(\"hello\");\n}\n");
}

#[test]
fn simultaneous_disk_writes_in_two_checkouts_both_survive() {
    let mut family = imported_family();
    let mut a = Checkout::new(1, &family);
    let mut b = Checkout::new(2, &family);

    let from_a = a.edit(&mut family, "src/main.rs", Some("// a\nfn main() {\n    println!(\"hi\");\n}\n"));
    let from_b = b.edit(&mut family, "src/main.rs", Some("fn main() {\n    println!(\"hi\");\n}\n// b\n"));

    let merged = "// a\nfn main() {\n    println!(\"hi\");\n}\n// b\n";
    assert_eq!(family_text(&family, "src/main.rs"), merged);
    assert_eq!(b.disk["src/main.rs"], merged, "b's ingest saw the merge and wrote it back");
    a.sync(&family, &from_b);
    assert_eq!(a.disk["src/main.rs"], merged);
    assert!(!from_a.is_empty());
}

#[test]
fn new_files_create_directories_and_deletes_propagate() {
    let mut family = imported_family();
    let mut a = Checkout::new(1, &family);
    let mut b = Checkout::new(2, &family);

    let created = a.edit(&mut family, "src/parser/lexer.rs", Some("pub struct Lexer;\n"));
    b.sync(&family, &created);
    assert_eq!(b.disk["src/parser/lexer.rs"], "pub struct Lexer;\n");

    let deleted = a.edit(&mut family, "README.md", None);
    b.sync(&family, &deleted);
    assert!(!b.disk.contains_key("README.md"));
    assert!(!family.tree().paths().contains_key("README.md"));
}

#[test]
fn rewriting_what_was_written_creates_no_deltas() {
    let mut family = imported_family();
    let mut a = Checkout::new(1, &family);
    let mut b = Checkout::new(2, &family);
    let deltas = a.edit(&mut family, "README.md", Some("# readme\nmore\n"));
    b.sync(&family, &deltas);
    let echo = b.edit(&mut family, "README.md", Some("# readme\nmore\n"));
    assert!(echo.is_empty());
}

#[test]
fn edits_continue_after_many_rounds() {
    let mut family = imported_family();
    let mut a = Checkout::new(1, &family);
    let mut b = Checkout::new(2, &family);
    let mut expected = bases()[&oid(2)].clone();
    for round in 0..20 {
        expected.push_str(&format!("line {round}\n"));
        let (writer, reader) = if round % 2 == 0 { (&mut a, &mut b) } else { (&mut b, &mut a) };
        let deltas = writer.edit(&mut family, "README.md", Some(&expected));
        reader.sync(&family, &deltas);
        assert_eq!(reader.disk["README.md"], expected);
    }
    assert_eq!(family_text(&family, "README.md"), expected);
}

#[test]
fn a_fork_sees_its_own_edits_and_the_source_does_not() {
    let mut family = imported_family();
    let mut source = Checkout::new(1, &family);
    let fork = ThreadId(2);
    let fork_delta = family.delta(
        ActorId(100),
        fork,
        Author::User,
        0,
        family.thread_head(THREAD),
        silt_log::Op::Fork { from: THREAD },
    );
    family.apply(fork_delta, &mut bases()).expect("forks");
    let mut forked = Checkout::on_thread(3, fork, &family);
    assert_eq!(forked.disk["README.md"], "# readme\n");

    let from_fork = forked.edit(&mut family, "README.md", Some("# readme\nfork work\n"));
    source.sync(&family, &from_fork);
    assert_eq!(source.disk["README.md"], "# readme\n", "the source thread doesn't see fork edits");

    let from_source = source.edit(&mut family, "src/new.rs", Some("// source work\n"));
    forked.sync(&family, &from_source);
    assert!(!forked.disk.contains_key("src/new.rs"), "the fork doesn't see later source edits");
    assert_eq!(family.thread_view(fork).file(family.tree().paths()["README.md"]), Some(FileView::Text("# readme\nfork work\n".into())));
}

#[test]
fn a_rename_is_a_move_so_concurrent_edits_follow_the_file() {
    let mut family = imported_family();
    let mut a = Checkout::new(1, &family);
    let mut b = Checkout::new(2, &family);
    let inode = family.tree().paths()["src/main.rs"];

    let text = a.disk.remove("src/main.rs").expect("materialized");
    a.disk.insert("src/bin/app.rs".into(), text.clone());
    let moved = a
        .bridge
        .rename(&mut family, &mut bases(), "src/main.rs", "src/bin/app.rs", &Checkout::file(&text), &Author::User, 0)
        .expect("renames")
        .expect("src/main.rs is a file of the thread");
    assert!(moved.deltas.iter().any(|delta| matches!(delta.op, silt_log::Op::Move { .. })));
    assert!(!moved.deltas.iter().any(|delta| matches!(delta.op, silt_log::Op::Delete { .. })));
    assert_eq!(family.tree().paths()["src/bin/app.rs"], inode);

    let edited = b.edit(&mut family, "src/main.rs", Some("// b\nfn main() {\n    println!(\"hi\");\n}\n"));
    b.sync(&family, &moved.deltas);
    a.sync(&family, &edited);
    let merged = "// b\nfn main() {\n    println!(\"hi\");\n}\n";
    assert_eq!(a.disk["src/bin/app.rs"], merged);
    assert_eq!(b.disk.get("src/bin/app.rs").map(String::as_str), Some(merged));
    assert!(!b.disk.contains_key("src/main.rs"));
}
