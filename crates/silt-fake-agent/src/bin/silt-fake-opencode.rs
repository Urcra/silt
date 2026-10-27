//! A stand-in for OpenCode, whose sessions live in a store reached only
//! through `opencode session list`, `export` and `import`. Here the store
//! is `$FAKE_OPENCODE_DIR`: an export per session, as one line, and a
//! `.meta` line of its id, creation time and directory.

use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::time::Duration;

use serde_json::{Value, json};
use silt_fake_agent::{
    Failed, Step, append, cwd, env_dir, env_or, exit, expect_words, io, main_rs, mentions, millis, read, write,
};

fn main() -> ExitCode {
    exit(run())
}

fn run() -> Step {
    let store = env_dir("FAKE_OPENCODE_DIR")?;
    fs::create_dir_all(&store).map_err(io)?;
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("session") => return list(&store),
        Some("export") => return export(&store, args.get(1).map_or("", String::as_str)),
        Some("import") => return import(&store, Path::new(args.get(1).map_or("", String::as_str))),
        _ => {}
    }
    let mut session = String::new();
    let mut rest = args.into_iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--session" => session = rest.next().unwrap_or_default(),
            "--prompt" => write(&store.join("last-prompt"), rest.next().unwrap_or_default().as_bytes())?,
            _ => {}
        }
    }
    write(&store.join("last-config"), env_or("OPENCODE_CONFIG_CONTENT", "").as_bytes())?;
    let cwd = cwd()?;
    if !session.is_empty() {
        let file = store.join(format!("{session}.json"));
        if !file.is_file() {
            return Err(Failed(3, format!("no session {session}")));
        }
        let text = read(&file);
        let mut export: Value = serde_json::from_str(&text).map_err(|error| Failed(3, format!("{session}: {error}")))?;
        if !mentions(&export, &cwd.to_string_lossy()) {
            return Err(Failed(3, format!("session not restored for {}", cwd.display())));
        }
        expect_words(&text, "session")?;
        append(&main_rs(&cwd), &format!("{}\n", env_or("FAKE_LINE", "fn opencode() {}")))?;
        let said = env_or("FAKE_SAY", "Continuing in opencode");
        if let Some(messages) = export["messages"].as_array_mut() {
            messages.extend(message(&session, &cwd, &said, "0000000002ZZZZZZZZZZZZZZZZ"));
        }
        write(&file, format!("{export}\n").as_bytes())?;
        std::thread::sleep(Duration::from_secs(4));
        return Ok(());
    }
    let created = millis();
    let random = uuid::Uuid::new_v4().simple().to_string();
    let session = format!("ses_{}{}", &created.to_string()[..12], &random[..14]);
    write(&store.join(format!("{session}.meta")), format!("{session} {created} {}\n", cwd.display()).as_bytes())?;
    append(&main_rs(&cwd), "fn farewell() {}\n")?;
    let export = json!({
        "info": {"id": session, "directory": cwd, "title": "Farewell", "time": {"created": created}},
        "messages": message(&session, &cwd, "Please add a farewell line to main", "0000000001AAAAAAAAAAAAAAAA"),
    });
    write(&store.join(format!("{session}.json")), format!("{export}\n").as_bytes())?;
    std::thread::sleep(Duration::from_secs(4));
    Ok(())
}

/// What `opencode export <id>` prints. As OpenCode does, it finds only the
/// sessions of the project it runs in.
fn export(store: &Path, id: &str) -> Step {
    eprintln!("Exporting session: {id}");
    let meta = read(&store.join(format!("{id}.meta")));
    let directory = meta.trim_end().splitn(3, ' ').nth(2).unwrap_or_default();
    let here = |dir: &Path| fs::canonicalize(dir).ok();
    if here(Path::new(directory)).is_none() || here(Path::new(directory)) != here(&cwd()?) {
        return Err(Failed(1, format!("Error: Session not found: {id}")));
    }
    print!("{}", read(&store.join(format!("{id}.json"))));
    Ok(())
}

/// What `opencode session list --format json` prints.
fn list(store: &Path) -> Step {
    let mut metas: Vec<_> = fs::read_dir(store)
        .map_err(io)?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "meta"))
        .collect();
    metas.sort();
    let sessions: Vec<Value> = metas
        .iter()
        .filter_map(|meta| {
            let line = read(meta);
            let mut fields = line.trim_end().splitn(3, ' ');
            let (id, created, directory) = (fields.next()?, fields.next()?.parse::<u64>().ok()?, fields.next()?);
            Some(json!({"id": id, "created": created, "updated": created, "directory": directory}))
        })
        .collect();
    println!("{}", Value::Array(sessions));
    Ok(())
}

fn import(store: &Path, file: &Path) -> Step {
    let text = read(file);
    let export: Value = serde_json::from_str(&text).map_err(|error| Failed(3, format!("{}: {error}", file.display())))?;
    let id =
        export["info"]["id"].as_str().filter(|id| id.starts_with("ses_")).ok_or_else(|| Failed(3, "no session id".into()))?;
    write(&store.join(format!("{id}.json")), text.as_bytes())?;
    let directory = export["info"]["directory"].as_str().unwrap_or_default();
    write(&store.join(format!("{id}.meta")), format!("{id} {} {directory}\n", millis()).as_bytes())?;
    println!("Imported session: {id}");
    Ok(())
}

/// A prompt and the reply that edits `src/main.rs` and says so.
fn message(session: &str, cwd: &Path, said: &str, id: &str) -> [Value; 2] {
    [
        json!({
            "info": {"id": format!("msg_{id}u"), "role": "user", "sessionID": session},
            "parts": [{"id": format!("prt_{id}ua"), "messageID": format!("msg_{id}u"), "type": "text", "text": said}],
        }),
        json!({
            "info": {"id": format!("msg_{id}a"), "role": "assistant", "sessionID": session, "path": {"cwd": cwd}},
            "parts": [
                {
                    "id": format!("prt_{id}b"), "messageID": format!("msg_{id}a"), "type": "tool", "callID": format!("call_{id}"),
                    "tool": "edit", "state": {"status": "completed", "input": {"filePath": main_rs(cwd)}, "output": "edited"},
                },
                {"id": format!("prt_{id}c"), "messageID": format!("msg_{id}a"), "type": "text", "text": "OpenCode says done.", "time": {"start": 1, "end": 2}},
                {"id": format!("prt_{id}d"), "messageID": format!("msg_{id}a"), "type": "step-finish", "reason": "stop"},
            ],
        }),
    ]
}
