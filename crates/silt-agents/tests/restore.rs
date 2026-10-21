//! Sessions recorded by other people (ADR 0016): whatever names they carry,
//! a session that passes its check is restored only inside the agent's
//! session store.

use std::fs;
use std::path::{Path, PathBuf};

use proptest::prelude::*;
use silt_agents::RecordedSession;

fn hostile_name() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => "[a-z0-9_-]{1,12}",
        1 => Just("..".to_owned()),
        1 => Just("../../escape".to_owned()),
        1 => Just("/etc/passwd".to_owned()),
        1 => Just("a/../../b".to_owned()),
        1 => Just("-rf".to_owned()),
        1 => Just(String::new()),
        1 => "[a-z./\\\\]{1,12}",
    ]
}

fn session() -> impl Strategy<Value = RecordedSession> {
    (
        hostile_name(),
        proptest::collection::vec((hostile_name(), proptest::collection::vec("[a-z ]{0,8}", 0..3)), 0..3),
        proptest::collection::vec((hostile_name(), proptest::collection::vec(any::<u8>(), 0..8)), 0..3),
    )
        .prop_map(|(id, subagents, attachments)| RecordedSession {
            id,
            former_ids: vec!["old".into()],
            former_cwds: vec!["/old/repo".into()],
            main: vec![r#"{"type":"user","cwd":"/old/repo","sessionId":"old"}"#.into()],
            subagents: subagents
                .into_iter()
                .map(|(name, lines)| (name, lines.into_iter().map(|line| format!(r#"{{"text":"{line}"}}"#)).collect()))
                .collect(),
            attachments,
            ..Default::default()
        })
}

fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                stack.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found
}

proptest! {
    #[test]
    fn checked_sessions_are_restored_inside_the_store(session in session()) {
        let root = tempfile::tempdir().expect("temp dir");
        let config = root.path().join("claude");
        let outside = root.path().join("outside");
        fs::create_dir_all(&outside).expect("mkdir");
        if session.check().is_err() {
            return Ok(());
        }
        let restored = silt_agents::claude::restore_into(&config, Path::new("/new"), Path::new("/new/repo"), &session);
        prop_assert!(restored.is_ok(), "{:?}", restored);
        prop_assert!(files_under(&outside).is_empty());
        for file in files_under(root.path()) {
            prop_assert!(file.starts_with(&config), "{} is outside the store", file.display());
        }
    }
}
