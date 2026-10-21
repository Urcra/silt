use std::collections::HashMap;

use silt_crdt::{InodeId, InodeKind, Parent, TextOp};
use silt_log::{Applied, Author, Content, ConvOp, Delta, Family, FileView, LogError, Op, SessionFile, SessionRef, TurnMark};
use silt_types::{ActorId, BlobHash, Frontier, GitOid, ThreadId};

const DAEMON: ActorId = ActorId(1);
const CHECKOUT_A: ActorId = ActorId(2);
const CHECKOUT_B: ActorId = ActorId(3);
const THREAD: ThreadId = ThreadId(7);

fn oid(byte: u8) -> GitOid {
    GitOid([byte; 20])
}

fn bases() -> HashMap<GitOid, String> {
    HashMap::from([(oid(1), "fn main() {}\n".to_owned())])
}

/// Applies `op` by `actor` on top of `parents` and returns the new delta.
fn write(family: &mut Family, actor: ActorId, parents: Frontier, op: Op) -> Delta {
    let delta = family.delta(actor, THREAD, Author::User, 0, parents, op);
    assert_eq!(family.apply(delta.clone(), &mut bases()), Ok(Applied::New));
    delta
}

fn write_on_head(family: &mut Family, actor: ActorId, op: Op) -> Delta {
    let head = family.thread_head(THREAD);
    write(family, actor, head, op)
}

fn create_main(family: &mut Family) -> InodeId {
    let src = write_on_head(
        family,
        DAEMON,
        Op::Create { kind: InodeKind::Dir, parent: Parent::Root, name: "src".into(), content: Content::Empty, executable: false },
    );
    let main = write_on_head(
        family,
        DAEMON,
        Op::Create {
            kind: InodeKind::File,
            parent: Parent::Dir(InodeId(src.span.first())),
            name: "main.rs".into(),
            content: Content::GitText(oid(1)),
            executable: false,
        },
    );
    InodeId(main.span.first())
}

fn text(family: &Family, inode: InodeId) -> String {
    match family.file(inode) {
        Some(FileView::Text(text)) => text,
        other => panic!("not text: {other:?}"),
    }
}

#[test]
fn git_files_stay_lazy_until_edited() {
    let mut family = Family::new();
    let main = create_main(&mut family);
    assert_eq!(family.file(main), Some(FileView::GitText(oid(1))));
    assert_eq!(family.tree().path_of(main).as_deref(), Some("src/main.rs"));

    write_on_head(&mut family, CHECKOUT_A, Op::Text { inode: main, op: TextOp::Insert { pos: 0, content: "// hi\n".into() } });
    assert_eq!(text(&family, main), "// hi\nfn main() {}\n");
}

#[test]
fn edits_from_two_checkouts_based_on_the_same_disk_version_merge() {
    let mut family = Family::new();
    let main = create_main(&mut family);
    let disk_version = family.thread_head(THREAD);
    write(
        &mut family,
        CHECKOUT_A,
        disk_version.clone(),
        Op::Text { inode: main, op: TextOp::Insert { pos: 0, content: "use a;\n".into() } },
    );
    write(
        &mut family,
        CHECKOUT_B,
        disk_version,
        Op::Text { inode: main, op: TextOp::Insert { pos: 13, content: "// b\n".into() } },
    );
    assert_eq!(text(&family, main), "use a;\nfn main() {}\n// b\n");
    assert_eq!(family.thread(THREAD).expect("thread").head().len(), 2);
    assert_eq!(
        silt_log::thread_head_in(family.log(), family.graph(), THREAD),
        family.thread_head(THREAD),
        "a server's copy of the log finds the same head"
    );
    let other = ThreadId(8);
    assert_eq!(silt_log::thread_head_in(family.log(), family.graph(), other), family.thread_head(other));
}

#[test]
fn replacing_content_wins_over_concurrent_edits_in_any_order() {
    let mut original = Family::new();
    let main = create_main(&mut original);
    let base = original.thread_head(THREAD);

    let mut a = original.clone();
    let edit =
        write(&mut a, CHECKOUT_A, base.clone(), Op::Text { inode: main, op: TextOp::Insert { pos: 0, content: "x".into() } });
    let mut b = original.clone();
    let replace = write(&mut b, CHECKOUT_B, base, Op::SetContent { inode: main, content: Content::Blob(BlobHash([9; 32])) });

    a.apply(replace, &mut bases()).expect("applies");
    b.apply(edit, &mut bases()).expect("applies");
    assert_eq!(a.file(main), b.file(main));
    assert_eq!(a.file(main), Some(FileView::Blob(BlobHash([9; 32]))));
}

#[test]
fn conversation_is_recorded_per_thread() {
    let mut family = Family::new();
    let session = SessionRef { agent: "claude-code".into(), id: "s1".into() };
    for line in ["{\"type\":\"user\"}", "{\"type\":\"assistant\"}"] {
        write_on_head(
            &mut family,
            DAEMON,
            Op::Conv(ConvOp::Native { session: session.clone(), file: SessionFile::Main, line: line.into() }),
        );
    }
    write_on_head(&mut family, DAEMON, Op::Conv(ConvOp::Title("Fix the parser".into())));
    let lines: Vec<&Op> = family.conversation(THREAD).map(|delta| &delta.op).collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(family.thread(THREAD).and_then(|t| t.title()), Some("Fix the parser"));
    assert_eq!(family.thread(THREAD).expect("thread").head().len(), 1);
}

#[test]
fn a_fork_from_a_turn_inherits_the_conversation_up_to_it() {
    let mut family = Family::new();
    let session = SessionRef { agent: "claude-code".into(), id: "s1".into() };
    let turn = |family: &mut Family, said: &str| {
        write_on_head(family, DAEMON, Op::Conv(ConvOp::Turn { session: session.clone(), mark: TurnMark::PromptSubmitted }));
        let line = format!("{{\"type\":\"user\",\"said\":\"{said}\"}}");
        write_on_head(family, DAEMON, Op::Conv(ConvOp::Native { session: session.clone(), file: SessionFile::Main, line }));
    };
    turn(&mut family, "first");
    turn(&mut family, "second");
    let graph = family.graph().clone();
    let after_first = silt_log::turn_point(family.log(), &graph, THREAD, 1).expect("turn 1");
    assert_eq!(
        silt_log::turn_point(family.log(), &graph, THREAD, 2),
        Some(family.thread_head(THREAD)),
        "the last turn ends at the head"
    );
    assert_eq!(silt_log::turn_point(family.log(), &graph, THREAD, 3), None);
    assert_eq!(silt_log::turn_point(family.log(), &graph, THREAD, 0), None);

    let fork = ThreadId(8);
    let delta = family.delta(CHECKOUT_A, fork, Author::User, 0, after_first, Op::Fork { from: THREAD });
    assert_eq!(family.apply(delta, &mut bases()), Ok(Applied::New));
    let said: Vec<String> = family
        .history(fork)
        .into_iter()
        .filter_map(|item| match &item.delta.op {
            Op::Conv(ConvOp::Native { line, .. }) => Some(line.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(said[0].contains("first"));

    // An agent that records its prompt before the mark: the turn still
    // ends before the next prompt.
    let mut family = Family::new();
    let line = |said: &str| {
        Op::Conv(ConvOp::Native {
            session: session.clone(),
            file: SessionFile::Main,
            line: format!("{{\"type\":\"user\",\"said\":\"{said}\"}}"),
        })
    };
    let mark = || Op::Conv(ConvOp::Turn { session: session.clone(), mark: TurnMark::PromptSubmitted });
    for op in [line("first"), mark(), line("second"), mark()] {
        write_on_head(&mut family, DAEMON, op);
    }
    let prompts = |delta: &silt_log::Delta| matches!(&delta.op, Op::Conv(ConvOp::Native { line, .. }) if line.contains("said"));
    let point = silt_log::turn_point_by(family.log(), &family.graph().clone(), THREAD, 1, prompts).expect("turn 1");
    assert_eq!(point, silt_types::Frontier::single(family.log()[1].span.last()), "before the second prompt's line");
}

#[test]
fn a_revert_rewinds_the_conversation_after_its_turn_and_keeps_it() {
    let mut family = Family::new();
    let session = SessionRef { agent: "claude-code".into(), id: "s1".into() };
    let said = |text: &str| {
        Op::Conv(ConvOp::Native { session: session.clone(), file: SessionFile::Main, line: format!("{{\"said\":\"{text}\"}}") })
    };
    let first = write_on_head(&mut family, DAEMON, said("first"));
    write_on_head(&mut family, DAEMON, said("second"));
    write_on_head(&mut family, DAEMON, said("third"));
    write_on_head(&mut family, DAEMON, Op::Conv(ConvOp::Rewound { to: first.span.first() }));
    write_on_head(&mut family, DAEMON, said("again"));

    let rewound: Vec<(String, bool)> = family
        .history(THREAD)
        .into_iter()
        .filter_map(|item| match &item.delta.op {
            Op::Conv(ConvOp::Native { line, .. }) => Some((line.clone(), item.rewound)),
            _ => None,
        })
        .collect();
    let flags: Vec<bool> = rewound.iter().map(|(_, rewound)| *rewound).collect();
    assert_eq!(flags, [false, true, true, false], "{rewound:?}");
    let from_log = silt_log::rewound_in(family.log(), THREAD);
    assert_eq!(from_log.len(), 2, "the server sees the same from the log alone");

    // A fork made after the revert inherits what was rewound as rewound.
    let fork = ThreadId(9);
    let delta = family.delta(CHECKOUT_A, fork, Author::User, 0, family.thread_head(THREAD), Op::Fork { from: THREAD });
    assert_eq!(family.apply(delta, &mut bases()), Ok(Applied::New));
    let inherited = family.history(fork).into_iter().filter(|item| item.rewound).count();
    assert_eq!(inherited, 2);
}

#[test]
fn missing_base_text_rejects_without_changes() {
    let mut family = Family::new();
    let main = create_main(&mut family);
    let before = family.log().len();
    let delta = family.delta(
        CHECKOUT_A,
        THREAD,
        Author::User,
        0,
        family.thread_head(THREAD),
        Op::Text { inode: main, op: TextOp::Insert { pos: 0, content: "x".into() } },
    );
    assert_eq!(family.apply(delta.clone(), &mut HashMap::new()), Err(LogError::MissingBase(oid(1))));
    assert_eq!(family.log().len(), before);
    assert!(!family.graph().contains(delta.span.first()));
    assert_eq!(family.apply(delta, &mut bases()), Ok(Applied::New));
}

#[test]
fn replaying_the_log_reproduces_the_family() {
    let mut family = Family::new();
    let main = create_main(&mut family);
    write_on_head(&mut family, CHECKOUT_A, Op::Text { inode: main, op: TextOp::Delete { pos: 0, len: 3 } });
    let mut replica = Family::new();
    for delta in family.log() {
        replica.apply(delta.clone(), &mut bases()).expect("replays");
        assert_eq!(replica.apply(delta.clone(), &mut bases()), Ok(Applied::AlreadyKnown));
    }
    assert_eq!(text(&replica, main), "main() {}\n");
    let bytes = postcard::to_allocvec(family.log()).expect("serializes");
    let decoded: Vec<Delta> = postcard::from_bytes(&bytes).expect("deserializes");
    assert_eq!(decoded, family.log());
}

/// The daemon's usual shape: each text edit is based on the thread head,
/// which is often a conversation delta by another actor. Kept large enough
/// that walking the whole history on every edit would show.
#[test]
fn edits_interleaved_with_conversation_stay_fast() {
    let mut family = Family::new();
    let main = create_main(&mut family);
    let session = SessionRef { agent: "claude-code".into(), id: "s".into() };
    let started = std::time::Instant::now();
    for round in 0..4000 {
        write_on_head(&mut family, CHECKOUT_A, Op::Text { inode: main, op: TextOp::Insert { pos: round, content: "x".into() } });
        write_on_head(
            &mut family,
            DAEMON,
            Op::Conv(ConvOp::Native { session: session.clone(), file: SessionFile::Main, line: "{}".into() }),
        );
    }
    assert!(text(&family, main).starts_with(&"x".repeat(4000)));
    eprintln!("4000 interleaved edits took {:?}", started.elapsed());
}

const FORK: ThreadId = ThreadId(8);

fn on_thread(family: &mut Family, actor: ActorId, thread: ThreadId, parents: Frontier, op: Op) -> Delta {
    let delta = family.delta(actor, thread, Author::User, 0, parents, op);
    assert_eq!(family.apply(delta.clone(), &mut bases()), Ok(Applied::New));
    delta
}

fn say(family: &mut Family, thread: ThreadId, line: &str) -> Delta {
    let head = family.thread_head(thread);
    on_thread(
        family,
        DAEMON.derive(thread.0),
        thread,
        head,
        Op::Conv(ConvOp::Native {
            session: SessionRef { agent: "claude-code".into(), id: format!("s{}", thread.0) },
            file: SessionFile::Main,
            line: line.into(),
        }),
    )
}

fn insert(family: &mut Family, actor: ActorId, thread: ThreadId, inode: InodeId, pos: usize, content: &str) -> Delta {
    let head = family.thread_head(thread);
    on_thread(family, actor, thread, head, Op::Text { inode, op: TextOp::Insert { pos, content: content.into() } })
}

fn lines(family: &Family, thread: ThreadId) -> Vec<(String, silt_log::HistorySource)> {
    family
        .history(thread)
        .into_iter()
        .map(|item| {
            let text = match &item.delta.op {
                Op::Conv(ConvOp::Native { line, .. }) => line.clone(),
                Op::Merge { from, mr } => format!("merge from {} ({})", from.0, mr.is_some()),
                other => format!("{other:?}"),
            };
            (text, item.source)
        })
        .collect()
}

fn view_text(family: &Family, thread: ThreadId, inode: InodeId) -> String {
    match family.thread_view(thread).file(inode) {
        Some(FileView::Text(text)) => text,
        other => panic!("not text: {other:?}"),
    }
}

#[test]
fn a_fork_inherits_history_and_a_pull_adds_upstream_progress() {
    use silt_log::HistorySource::{Inherited, Own, Upstream};
    let mut family = Family::new();
    let main = create_main(&mut family);
    say(&mut family, THREAD, "source 1");
    insert(&mut family, CHECKOUT_A, THREAD, main, 0, "// a\n");
    let head = family.thread_head(THREAD);
    on_thread(&mut family, CHECKOUT_B, FORK, head, Op::Fork { from: THREAD });
    say(&mut family, THREAD, "source 2");
    insert(&mut family, CHECKOUT_A, THREAD, main, 5, "// a2\n");
    say(&mut family, FORK, "fork 1");
    insert(&mut family, ActorId(9), FORK, main, 0, "// fork\n");

    assert_eq!(lines(&family, FORK), vec![("source 1".into(), Inherited), ("fork 1".into(), Own)]);
    assert_eq!(view_text(&family, FORK, main), "// fork\n// a\nfn main() {}\n");
    assert_eq!(view_text(&family, THREAD, main), "// a\n// a2\nfn main() {}\n");

    let parents: Frontier = family.thread_head(FORK).ids().iter().chain(family.thread_head(THREAD).ids()).copied().collect();
    on_thread(&mut family, DAEMON.derive(FORK.0), FORK, parents, Op::Merge { from: THREAD, mr: None });
    assert_eq!(
        lines(&family, FORK),
        vec![
            ("source 1".into(), Inherited),
            ("fork 1".into(), Own),
            ("source 2".into(), Upstream),
            (format!("merge from {} (false)", THREAD.0), Own),
        ]
    );
    assert_eq!(view_text(&family, FORK, main), "// fork\n// a\n// a2\nfn main() {}\n");
    assert_eq!(view_text(&family, THREAD, main), "// a\n// a2\nfn main() {}\n", "the source never changes");
    assert_eq!(lines(&family, THREAD), vec![("source 1".into(), Own), ("source 2".into(), Own)]);
}

fn pull(family: &mut Family, into: ThreadId, from: ThreadId) {
    let parents: Frontier = family.thread_head(into).ids().iter().chain(family.thread_head(from).ids()).copied().collect();
    on_thread(family, DAEMON.derive(into.0), into, parents, Op::Merge { from, mr: None });
}

#[test]
fn repeated_pulls_both_ways_keep_each_line_once() {
    use silt_log::HistorySource::{Inherited, Own, Upstream};
    let mut family = Family::new();
    create_main(&mut family);
    say(&mut family, THREAD, "source 1");
    let head = family.thread_head(THREAD);
    on_thread(&mut family, CHECKOUT_B, FORK, head, Op::Fork { from: THREAD });
    say(&mut family, THREAD, "source 2");
    pull(&mut family, FORK, THREAD);
    say(&mut family, FORK, "fork 1");
    say(&mut family, THREAD, "source 3");
    pull(&mut family, FORK, THREAD);
    pull(&mut family, THREAD, FORK);
    let merge = |from: ThreadId| format!("merge from {} (false)", from.0);
    assert_eq!(
        lines(&family, FORK),
        vec![
            ("source 1".into(), Inherited),
            ("source 2".into(), Upstream),
            (merge(THREAD), Own),
            ("fork 1".into(), Own),
            ("source 3".into(), Upstream),
            (merge(THREAD), Own),
        ]
    );
    assert_eq!(
        lines(&family, THREAD),
        vec![
            ("source 1".into(), Own),
            ("source 2".into(), Own),
            ("source 3".into(), Own),
            (merge(THREAD), Upstream),
            ("fork 1".into(), Upstream),
            (merge(THREAD), Upstream),
            (merge(FORK), Own),
        ]
    );
}

#[test]
fn a_forks_new_content_and_mode_stay_in_the_fork() {
    let mut family = Family::new();
    let main = create_main(&mut family);
    let head = family.thread_head(THREAD);
    on_thread(&mut family, CHECKOUT_B, FORK, head, Op::Fork { from: THREAD });
    let head = family.thread_head(FORK);
    let replaced =
        on_thread(&mut family, CHECKOUT_B, FORK, head, Op::SetContent { inode: main, content: Content::Blob(BlobHash([7; 32])) });
    on_thread(
        &mut family,
        CHECKOUT_B,
        FORK,
        Frontier::single(replaced.span.last()),
        Op::SetExecutable { inode: main, executable: true },
    );

    let source = family.thread_view(THREAD);
    assert_eq!(source.file(main), Some(FileView::GitText(oid(1))), "the source keeps its content");
    assert!(!source.is_executable(main), "and its mode");
    let fork = family.thread_view(FORK);
    assert_eq!(fork.file(main), Some(FileView::Blob(BlobHash([7; 32]))));
    assert!(fork.is_executable(main));

    let before = family.graph().version_vector(&replaced.parents);
    assert_eq!(family.file_at(main, &before), Some(FileView::GitText(oid(1))), "content as of a version");
}

#[test]
fn views_follow_new_deltas_on_either_thread() {
    let mut family = Family::new();
    create_main(&mut family);
    let head = family.thread_head(THREAD);
    on_thread(&mut family, CHECKOUT_B, FORK, head, Op::Fork { from: THREAD });
    let listed = |family: &Family, thread| family.thread_view(thread).paths().keys().cloned().collect::<Vec<_>>();
    assert_eq!(listed(&family, FORK), ["src", "src/main.rs"]);
    let create = |name: &str| Op::Create {
        kind: InodeKind::File,
        parent: Parent::Root,
        name: name.into(),
        content: Content::Empty,
        executable: false,
    };
    let head = family.thread_head(FORK);
    on_thread(&mut family, CHECKOUT_B, FORK, head, create("fork.txt"));
    let head = family.thread_head(THREAD);
    on_thread(&mut family, CHECKOUT_A, THREAD, head, create("source.txt"));
    assert_eq!(listed(&family, FORK), ["fork.txt", "src", "src/main.rs"]);
    assert_eq!(listed(&family, THREAD), ["source.txt", "src", "src/main.rs"]);
    let copy = family.clone();
    assert_eq!(listed(&copy, THREAD), listed(&family, THREAD));
}

#[test]
fn concurrent_decisions_titles_and_comments_read_alike_in_any_order() {
    use silt_log::{CommentOp, CommentTarget, MrOp};
    let mut family = Family::new();
    create_main(&mut family);
    let head = family.thread_head(THREAD);
    on_thread(&mut family, CHECKOUT_B, FORK, head, Op::Fork { from: THREAD });
    let head = family.thread_head(FORK);
    let opened = on_thread(
        &mut family,
        ActorId(9),
        FORK,
        head,
        Op::Mr(MrOp::Opened { target: THREAD, title: "Add a comment".into(), summary: String::new() }),
    );
    let mr = opened.span.first();
    let head: Frontier = family.thread_head(THREAD).ids().iter().copied().chain([mr]).collect();
    let concurrent = |actor: u128, op: Op| family.delta(ActorId(actor), THREAD, Author::User, 0, head.clone(), op);
    let deltas = [
        concurrent(20, Op::Mr(MrOp::Approved { mr })),
        concurrent(21, Op::Mr(MrOp::Declined { mr, reason: "no".into() })),
        concurrent(22, Op::Conv(ConvOp::Title("one".into()))),
        concurrent(23, Op::Conv(ConvOp::Title("two".into()))),
        concurrent(
            24,
            Op::Comment(CommentOp::Added { target: CommentTarget::Message { delta: mr }, text: "first".into(), by: "a".into() }),
        ),
        concurrent(
            25,
            Op::Comment(CommentOp::Added { target: CommentTarget::Message { delta: mr }, text: "second".into(), by: "b".into() }),
        ),
    ];
    let read = |order: &mut dyn Iterator<Item = &Delta>| {
        let mut replica = family.clone();
        for delta in order {
            assert_eq!(replica.apply(delta.clone(), &mut bases()), Ok(Applied::New));
        }
        let comments: Vec<String> = replica.comments(THREAD).into_iter().map(|comment| comment.text).collect();
        (
            replica.merge_requests()[0].status.clone(),
            replica.thread(THREAD).and_then(|state| state.title().map(str::to_owned)),
            comments,
        )
    };
    assert_eq!(read(&mut deltas.iter()), read(&mut deltas.iter().rev()));

    // Replies, a reply to a reply among them, sit under the first comment
    // in the same order on every replica.
    let first = deltas[4].span.first();
    let mut replica = family.clone();
    for delta in &deltas {
        replica.apply(delta.clone(), &mut bases()).expect("applies");
    }
    let tip: Frontier = replica.thread_head(THREAD);
    let reply = |actor: u128, parent, text: &str| {
        replica.delta(
            ActorId(actor),
            THREAD,
            Author::User,
            0,
            tip.clone(),
            Op::Comment(CommentOp::Replied { parent, text: text.into(), by: "c".into() }),
        )
    };
    let answer = reply(30, first, "agreed");
    let nested = reply(31, answer.span.first(), "me too");
    let other = reply(32, first, "but why");
    let replied = |order: &[&Delta]| {
        let mut copy = replica.clone();
        for delta in order {
            copy.apply((*delta).clone(), &mut bases()).expect("applies");
        }
        copy.comments(THREAD)
            .into_iter()
            .map(|comment| (comment.text, comment.replies.into_iter().map(|reply| reply.text).collect::<Vec<_>>()))
            .collect::<Vec<_>>()
    };
    let forward = replied(&[&answer, &nested, &other]);
    assert_eq!(forward, replied(&[&other, &answer, &nested]));
    assert_eq!(forward[0].0, "first");
    assert_eq!(forward[0].1.len(), 3);
    assert!(forward[1].1.is_empty());
}

#[test]
fn turn_marks_say_which_tool_is_open_and_which_were_recorded() {
    let mut family = Family::new();
    let session = SessionRef { agent: "claude-code".into(), id: "s".into() };
    let mark = |family: &mut Family, mark: TurnMark| {
        write_on_head(family, DAEMON, Op::Conv(ConvOp::Turn { session: session.clone(), mark }));
    };
    let started = |id: &str| TurnMark::ToolStarted { tool_use_id: id.into(), tool: "Edit".into() };
    let finished = |id: &str| TurnMark::ToolFinished { tool_use_id: id.into() };

    assert!(!family.in_turn(&session));
    mark(&mut family, TurnMark::PromptSubmitted);
    assert!(family.in_turn(&session));
    assert_eq!(family.open_tool(&session), None);
    assert!(family.has_turn_mark(&session, &TurnMark::PromptSubmitted), "the same prompt mark again is a repeat");
    mark(&mut family, started("t1"));
    assert_eq!(family.open_tool(&session), Some("t1"));
    assert!(family.has_turn_mark(&session, &started("t1")));
    assert!(!family.has_turn_mark(&session, &finished("t1")));
    mark(&mut family, finished("t1"));
    assert_eq!(family.open_tool(&session), None);
    assert!(!family.has_turn_mark(&session, &TurnMark::PromptSubmitted), "a new prompt after a tool is not a repeat");

    mark(&mut family, started("t2"));
    mark(&mut family, finished("t0"));
    assert_eq!(family.open_tool(&session), Some("t2"), "another tool finishing leaves this one open");
    mark(&mut family, TurnMark::Stopped);
    assert_eq!(family.open_tool(&session), None, "the turn's end closes it");
    assert!(!family.in_turn(&session));
    let other = SessionRef { agent: "claude-code".into(), id: "other".into() };
    assert!(!family.has_turn_mark(&other, &started("t1")));
}

#[test]
fn merge_requests_open_and_resolve_once() {
    use silt_log::{MrOp, MrStatus};
    let mut family = Family::new();
    let main = create_main(&mut family);
    let head = family.thread_head(THREAD);
    on_thread(&mut family, CHECKOUT_B, FORK, head, Op::Fork { from: THREAD });
    insert(&mut family, ActorId(9), FORK, main, 0, "// fork\n");
    let head = family.thread_head(FORK);
    let opened = on_thread(
        &mut family,
        ActorId(9),
        FORK,
        head,
        Op::Mr(MrOp::Opened { target: THREAD, title: "Add a comment".into(), summary: "One line.".into() }),
    );
    let mr = opened.span.first();
    assert_eq!(family.merge_requests()[0].status, MrStatus::Open);
    assert_eq!(family.merge_requests()[0].fork, FORK);

    // Asking for changes keeps it open, the latest request showing.
    for reason in ["add a test", "and a doc comment"] {
        let head = family.thread_head(THREAD);
        on_thread(
            &mut family,
            DAEMON.derive(THREAD.0),
            THREAD,
            head,
            Op::Mr(MrOp::ChangesRequested { mr, reason: reason.into() }),
        );
    }
    let request = &family.merge_requests()[0];
    assert_eq!(request.status, MrStatus::Open);
    assert_eq!(request.changes_requested.as_ref().map(|asked| asked.reason.as_str()), Some("and a doc comment"));
    for (guide, by) in [("first draft", "agent"), ("read main first", "bob")] {
        let head = family.thread_head(FORK);
        on_thread(&mut family, ActorId(9), FORK, head, Op::Mr(MrOp::Guided { mr, guide: guide.into(), by: by.into() }));
    }
    assert_eq!(family.merge_requests()[0].guide, Some(("read main first".into(), "bob".into())));

    let parents: Frontier = family.thread_head(THREAD).ids().iter().copied().chain([mr]).collect();
    on_thread(&mut family, DAEMON.derive(THREAD.0), THREAD, parents, Op::Merge { from: FORK, mr: Some(mr) });
    let head = family.thread_head(THREAD);
    on_thread(&mut family, DAEMON.derive(THREAD.0), THREAD, head, Op::Mr(MrOp::Approved { mr }));
    let head = family.thread_head(THREAD);
    on_thread(&mut family, DAEMON.derive(THREAD.0), THREAD, head, Op::Mr(MrOp::Declined { mr, reason: "too late".into() }));
    assert_eq!(family.merge_requests()[0].status, MrStatus::Approved);
    assert_eq!(view_text(&family, THREAD, main), "// fork\nfn main() {}\n");
    assert_eq!(view_text(&family, FORK, main), "// fork\nfn main() {}\n");
}
