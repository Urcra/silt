//! Artifacts (ADR 0068) between machines: what an agent showed on one
//! machine reaches the person's other machines with the thread.

mod common;

use std::fs;

use base64::Engine as _;
use common::{Machine, Server, eventually, origin};

/// Fields drop in order: the machines stop their daemons before the
/// directories holding their sockets go away.
struct World {
    laptop: Machine,
    home: Machine,
    server: Server,
    _root: tempfile::TempDir,
}

fn world() -> World {
    let root = common::temp_root("silt-artifacts");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    let home = Machine::new(root.path(), "home", &origin, &server);
    let fake = laptop.fake.with_file_name(format!("silt-fake-agent{}", std::env::consts::EXE_SUFFIX));
    let agents = serde_json::json!([{"id": "v1", "name": "Fake v1", "command": fake}]);
    fs::write(laptop.silt_home.join("agents.json"), agents.to_string()).expect("write agents.json");
    World { laptop, home, server, _root: root }
}

fn artifacts(machine: &Machine, thread: &str) -> Vec<serde_json::Value> {
    let (status, history) = machine.web_answer("GET", &format!("/api/threads/{thread}/history"), None);
    if status != "200" {
        return Vec::new();
    }
    let history: serde_json::Value = serde_json::from_str(&history).unwrap_or_default();
    history["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry["kind"] == "artifact")
        .map(|entry| entry["artifact"].clone())
        .collect()
}

#[test]
fn an_artifact_made_on_one_machine_opens_on_another() {
    let world = world();
    let (laptop, home) = (&world.laptop, &world.home);
    let started = laptop.web(
        "POST",
        "/api/agents/start",
        Some(serde_json::json!({"clone": laptop.repo, "agent": "v1", "prompt": "Artifact"})),
    );
    let thread = started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("thread").to_owned());
    assert!(eventually(|| artifacts(laptop, &thread).len() == 1), "the laptop keeps it");

    let mut seen = Vec::new();
    assert!(
        eventually(|| {
            seen = artifacts(home, &thread);
            seen.len() == 1
        }),
        "the other machine pulls it: {seen:?}"
    );
    let blob = seen[0]["blob"].as_str().expect("blob");
    let content = home.web("GET", &format!("/api/threads/{thread}/artifacts/{blob}"), None);
    let bytes = base64::engine::general_purpose::STANDARD.decode(content["data"].as_str().expect("data")).expect("base64");
    assert_eq!(String::from_utf8_lossy(&bytes), "<h1>Rescan benchmark</h1>", "its content comes from the server");
    assert_eq!(content["media_type"], "text/html");
}

/// What a prompt's images and videos the machine's history names, kept as blobs.
fn stored_media(machine: &Machine, thread: &str) -> Vec<serde_json::Value> {
    let (status, history) = machine.web_answer("GET", &format!("/api/threads/{thread}/history"), None);
    if status != "200" {
        return Vec::new();
    }
    let history: serde_json::Value = serde_json::from_str(&history).unwrap_or_default();
    history["entries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry["image"]["stored"].as_object().map(|stored| serde_json::Value::Object(stored.clone())))
        .collect()
}

/// Part of what the machine's daemon serves at `path`, as a video asks for it.
fn ranged(machine: &Machine, path: &str, range: &str) -> (String, Vec<u8>) {
    let url = machine.stdout(&["open", "--no-browser"]);
    let (base, token) = common::web_ui(&url);
    let output = std::process::Command::new("curl")
        .args(["-s", "-r", range, "-w", "%{http_code}", "-H"])
        .arg(format!("Authorization: Bearer {token}"))
        .arg(format!("{base}{path}"))
        .output()
        .expect("curl runs");
    let (body, status) = output.stdout.split_at(output.stdout.len().saturating_sub(3));
    (String::from_utf8_lossy(status).into_owned(), body.to_vec())
}

/// A video attached to a prompt has no size limit: it's uploaded to the
/// machine first, goes to the server in chunks after the deltas naming it,
/// and another machine reads it by range from the server (ADR 0121).
#[test]
fn a_large_video_reaches_another_machine_in_chunks_and_is_read_by_range() {
    let world = world();
    let (laptop, home) = (&world.laptop, &world.home);
    // A WebM's header, then past one chunk, so it goes after its deltas.
    let mut video = fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../silt-fake-agent/fixtures/login.webm")).expect("the clip");
    video.resize((9 << 20) + 4321, 7);
    let file = laptop.silt_home.join("checkout.webm");
    fs::write(&file, &video).expect("write the video");
    let url = laptop.stdout(&["open", "--no-browser"]);
    let (base, token) = common::web_ui(&url);
    let output = std::process::Command::new("curl")
        .args(["-s", "-X", "POST", "-H", "X-Silt-Request: 1", "-H"])
        .arg(format!("Authorization: Bearer {token}"))
        .arg("--data-binary")
        .arg(format!("@{}", file.display()))
        .arg(format!("{base}/api/uploads"))
        .output()
        .expect("curl runs");
    let upload: serde_json::Value = serde_json::from_slice(&output.stdout).expect("the upload answers");
    assert_eq!(upload["size"], video.len(), "{upload}");
    let attachment = serde_json::json!({"name": "checkout.webm", "mime": "video/webm", "data": "", "upload": upload["upload"]});
    let started = laptop.web(
        "POST",
        "/api/agents/start",
        Some(serde_json::json!({"clone": laptop.repo, "agent": "v1", "prompt": "watch this", "attachments": [attachment]})),
    );
    let thread = started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("thread").to_owned());
    let mut kept = Vec::new();
    assert!(
        eventually(|| {
            kept = stored_media(laptop, &thread);
            kept.len() == 1
        }),
        "the laptop keeps the video with the prompt"
    );
    let blob = kept[0]["blob"].as_str().expect("blob").to_owned();
    assert_eq!(kept[0]["size"], video.len());
    assert!(eventually(|| stored_media(home, &thread).len() == 1), "the other machine pulls the prompt");
    let path = format!("/api/threads/{thread}/blobs/{blob}");
    let mut seen = (String::new(), Vec::new());
    assert!(
        eventually(|| {
            seen = ranged(home, &path, "0-15");
            seen.0 == "206"
        }),
        "the other machine reads it by range once it has arrived: {}",
        seen.0
    );
    assert_eq!(seen.1, video[..16]);
    let (status, tail) = ranged(home, &path, &format!("{}-", video.len() - 10));
    assert_eq!((status.as_str(), tail.as_slice()), ("206", &video[video.len() - 10..]));
    let state = home.web("GET", &format!("{path}/state"), None);
    assert_eq!(state["stored"], true, "{state}");
    assert!(state["from"].as_str().is_some_and(|from| !from.is_empty()), "it names the machine it came from: {state}");
    assert_eq!(laptop.web("GET", &format!("{path}/state"), None)["stored"], true);
    let (refused, _) = home.web_answer("GET", &format!("/api/threads/{thread}/blobs/{}", "ab".repeat(32)), None);
    assert_ne!(refused, "200", "only a blob the thread names is served");
}

/// A server capped below a file keeps the conversation going without it:
/// its machine says the file is only there, and the other machine says
/// which machine has it and why (ADR 0121).
#[test]
fn a_file_over_the_servers_cap_stays_on_its_machine_and_both_say_why() {
    let root = common::temp_root("silt-artifacts-cap");
    let origin = origin(root.path());
    let data = root.path().join("server");
    fs::create_dir_all(&data).expect("server data");
    fs::write(data.join("server.toml"), "max_file_mib = 8\n").expect("server.toml");
    let server = Server::start(&data);
    let laptop = Machine::new(root.path(), "laptop", &origin, &server);
    let home = Machine::new(root.path(), "home", &origin, &server);
    let fake = laptop.fake.with_file_name(format!("silt-fake-agent{}", std::env::consts::EXE_SUFFIX));
    let agents = serde_json::json!([{"id": "v1", "name": "Fake v1", "command": fake}]);
    fs::write(laptop.silt_home.join("agents.json"), agents.to_string()).expect("write agents.json");

    let mut video = fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../silt-fake-agent/fixtures/login.webm")).expect("the clip");
    video.resize((9 << 20) + 17, 3);
    let file = laptop.silt_home.join("long.webm");
    fs::write(&file, &video).expect("write the video");
    let url = laptop.stdout(&["open", "--no-browser"]);
    let (base, token) = common::web_ui(&url);
    let output = std::process::Command::new("curl")
        .args(["-s", "-X", "POST", "-H", "X-Silt-Request: 1", "-H"])
        .arg(format!("Authorization: Bearer {token}"))
        .arg("--data-binary")
        .arg(format!("@{}", file.display()))
        .arg(format!("{base}/api/uploads"))
        .output()
        .expect("curl runs");
    let upload: serde_json::Value = serde_json::from_slice(&output.stdout).expect("the upload answers");
    let attachment = serde_json::json!({"name": "long.webm", "mime": "video/webm", "data": "", "upload": upload["upload"]});
    let started = laptop.web(
        "POST",
        "/api/agents/start",
        Some(serde_json::json!({"clone": laptop.repo, "agent": "v1", "prompt": "watch this", "attachments": [attachment]})),
    );
    let thread = started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("thread").to_owned());
    assert!(eventually(|| stored_media(&home, &thread).len() == 1), "the conversation goes on without the file");
    let blob = stored_media(&home, &thread)[0]["blob"].as_str().expect("blob").to_owned();
    let path = format!("/api/threads/{thread}/blobs/{blob}/state");
    let mut there = serde_json::Value::Null;
    assert!(
        eventually(|| {
            there = home.web("GET", &path, None);
            there["refused"].as_str().is_some_and(|why| why.contains("at most 8 MiB"))
        }),
        "the other machine hears why it won't arrive: {there}"
    );
    assert_eq!(there["stored"], false);
    assert!(there["from"].as_str().is_some_and(|from| !from.is_empty()), "and which machine has it: {there}");
    let mut here = serde_json::Value::Null;
    assert!(
        eventually(|| {
            here = laptop.web("GET", &path, None);
            here["stored"] == true && here["refused"].as_str().is_some()
        }),
        "its own machine plays it and says it's only there: {here}"
    );
}

/// What `url` answers: its status, the headers asked for, and its body.
fn fetch(url: &str, headers: &[&str]) -> (String, Vec<String>, String) {
    let output = std::process::Command::new("curl").args(["-s", "-D", "-", url]).output().expect("curl runs");
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));
    let status = head.split_whitespace().nth(1).unwrap_or_default().to_owned();
    let found = headers
        .iter()
        .map(|name| {
            head.lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(key, _)| key.eq_ignore_ascii_case(name))
                        .map(|(_, value)| value.trim().to_owned())
                })
                .unwrap_or_default()
        })
        .collect();
    (status, found, body.to_owned())
}

/// A read-only link to one artifact (ADR 0068) shows it alone, its content
/// served under a sandbox that lets it reach nothing; a link names only an
/// artifact the thread has.
#[test]
fn a_read_only_link_shows_one_artifact_sandboxed() {
    let world = world();
    let laptop = &world.laptop;
    let started = laptop.web(
        "POST",
        "/api/agents/start",
        Some(serde_json::json!({"clone": laptop.repo, "agent": "v1", "prompt": "Artifact"})),
    );
    let thread = started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("thread").to_owned());
    let mut shown = Vec::new();
    assert!(
        eventually(|| {
            shown = artifacts(laptop, &thread);
            shown.len() == 1
                && laptop.web_answer("GET", &format!("/api/threads/{thread}/links"), None).0 == "200"
                && laptop
                    .web_answer(
                        "POST",
                        &format!("/api/threads/{thread}/links"),
                        Some(serde_json::json!({"days": 1, "artifact": {"id": "0123456789ab"}})),
                    )
                    .0
                    == "400"
        }),
        "the server has the thread, and refuses an artifact it doesn't have"
    );
    let id = shown[0]["id"].as_str().expect("id").to_owned();
    // The artifact reaches the server after the thread does.
    let mut link = serde_json::Value::Null;
    assert!(
        eventually(|| {
            let body = serde_json::json!({"days": 1, "artifact": {"id": id}});
            let (status, answer) = laptop.web_answer("POST", &format!("/api/threads/{thread}/links"), Some(body));
            link = serde_json::from_str(&answer).unwrap_or_default();
            status == "200"
        }),
        "the server takes a link to the artifact once it has it: {link}"
    );
    let url = link["url"].as_str().expect("the link");
    assert!(url.starts_with(&world.server.url), "{url}");
    assert_eq!(link["artifact"]["id"], id);

    let (status, _, page) = fetch(url, &[]);
    assert_eq!(status, "200", "{page}");
    assert!(!page.contains("Done: Artifact"), "nothing else of the thread: {page}");
    // The app's own viewer shows it where the web UI was built (ADR 0073);
    // the server's plain page does otherwise.
    if !page.contains(r#"name="silt-link""#) {
        assert!(page.contains("Rescan benchmark") && page.contains(r#"sandbox="allow-scripts""#), "{page}");
    }
    let (status, _, view) = fetch(&format!("{url}/view"), &[]);
    assert_eq!(status, "200", "{view}");
    let view: serde_json::Value = serde_json::from_str(&view).expect("the link's view is JSON");
    assert_eq!(
        (view["artifact"]["id"].as_str(), view["artifact"]["title"].as_str()),
        (Some(id.as_str()), Some("Rescan benchmark"))
    );
    assert!(!view.to_string().contains("Done: Artifact"), "nothing else of the thread: {view}");

    let (status, headers, body) =
        fetch(&format!("{url}/content"), &["content-security-policy", "content-type", "x-content-type-options"]);
    assert_eq!(status, "200");
    assert!(body.starts_with("<!doctype html><script>window.silt=") && body.ends_with("<h1>Rescan benchmark</h1>"), "{body}");
    assert!(headers[0].starts_with("sandbox allow-scripts;") && headers[0].contains("default-src 'none'"), "{headers:?}");
    assert!(headers[0].contains("https://cdnjs.cloudflare.com") && !headers[0].contains("connect-src"), "{headers:?}");
    assert_eq!((headers[1].as_str(), headers[2].as_str()), ("text/html; charset=utf-8", "nosniff"));

    let thread_link = laptop.web("POST", &format!("/api/threads/{thread}/links"), Some(serde_json::json!({"days": 1})));
    let (status, _, _) = fetch(&format!("{}/content", thread_link["url"].as_str().expect("link")), &[]);
    assert_eq!(status, "404", "a link to the conversation serves no artifact content");
}

/// A person's own design systems follow them (ADR 0071): one made on the
/// laptop reaches the home machine through the server, and removing it
/// there removes it on the laptop too.
#[test]
fn a_persons_design_systems_follow_them_between_machines() {
    let world = world();
    let on = |machine: &Machine| machine.silt_home.join("design-systems/acme");
    fs::create_dir_all(on(&world.laptop).join("fonts")).expect("mkdir");
    let tokens =
        r##"{"name":"Acme","color":{"themes":[{"id":"light"}],"tokens":[{"name":"bg","value":"#fbfaf7","usage":"Page."}]}}"##;
    fs::write(on(&world.laptop).join("tokens.json"), tokens).expect("tokens");
    fs::write(on(&world.laptop).join("README.md"), "Set pages on `bg`.").expect("readme");
    fs::write(on(&world.laptop).join("fonts/acme.woff2"), b"wOF2").expect("font");

    let listed = |machine: &Machine| machine.web("GET", "/api/design-systems", None).to_string();
    assert!(listed(&world.laptop).contains(r#""name":"acme""#), "{}", listed(&world.laptop));
    assert!(
        eventually(|| listed(&world.home).contains(r#""name":"acme""#)),
        "the home machine has it: {}",
        world.home.daemon_log()
    );
    assert_eq!(fs::read_to_string(on(&world.home).join("tokens.json")).expect("tokens"), tokens);
    assert_eq!(fs::read(on(&world.home).join("fonts/acme.woff2")).expect("font"), b"wOF2");

    fs::remove_dir_all(on(&world.home)).expect("remove");
    assert!(eventually(|| !on(&world.laptop).exists()), "the removal reaches the laptop: {}", world.laptop.daemon_log());
}

/// An artifact's page data between two people on two machines (ADR 0072):
/// each writes their own vote, the rules refuse a vote written for someone
/// else, both see both votes, and a fork inherits them and then diverges.
#[test]
fn a_polls_votes_from_two_people_both_count_and_a_fork_inherits_them() {
    let root = common::temp_root("silt-page-data");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let alice = Machine::new_for(root.path(), "alice", "alice", &origin, &server);
    let bob = Machine::new_for(root.path(), "bob", "bob", &origin, &server);
    let fake = alice.fake.with_file_name(format!("silt-fake-agent{}", std::env::consts::EXE_SUFFIX));
    let agents = serde_json::json!([{"id": "v1", "name": "Fake v1", "command": fake}]);
    fs::write(alice.silt_home.join("agents.json"), agents.to_string()).expect("write agents.json");
    let started = alice.web(
        "POST",
        "/api/agents/start",
        Some(serde_json::json!({"clone": alice.repo, "agent": "v1", "prompt": "Artifact poll"})),
    );
    let thread = started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("thread").to_owned());
    let mut shown = Vec::new();
    assert!(eventually(|| {
        shown = artifacts(&alice, &thread);
        shown.len() == 1
    }));
    let id = shown[0]["id"].as_str().expect("id").to_owned();
    assert!(shown[0]["capabilities"]["db"].is_object(), "{shown:?}");
    let data = format!("/api/threads/{thread}/artifacts/{id}/data");
    let (ann, ben) = (silt_proto::page::person_id("alice"), silt_proto::page::person_id("bob"));

    let vote = |machine: &Machine, who: &str, pick: &str| {
        let body = serde_json::json!({"op": "set", "path": format!("votes/{who}"), "value": {"pick": pick}});
        machine.web_answer("POST", &data, Some(body))
    };
    assert_eq!(vote(&alice, &ann, "pizza").0, "200");
    assert!(eventually(|| artifacts(&bob, &thread).len() == 1), "bob's machine pulls the thread");
    let (status, refused) = vote(&bob, &ann, "salad");
    assert_ne!(status, "200", "bob can't vote as alice: {refused}");
    let mut votes = serde_json::Value::Null;
    let mut last = (String::new(), String::new());
    assert!(
        eventually(|| {
            last = vote(&bob, &ben, "soup");
            last.0 == "200"
        }),
        "bob votes once his machine has the version that declares the rules: {last:?}"
    );
    let list = serde_json::json!({"op": "list", "collection": "votes"});
    assert!(
        eventually(|| {
            votes = alice.web("POST", &data, Some(list.clone()));
            votes.as_array().is_some_and(|docs| docs.len() == 2)
        }),
        "alice sees both votes: {votes}"
    );
    let picks: Vec<&str> = votes.as_array().expect("docs").iter().filter_map(|doc| doc["data"]["pick"].as_str()).collect();
    assert!(picks.contains(&"pizza") && picks.contains(&"soup"), "{picks:?}");

    let forked = alice.web("POST", &format!("/api/threads/{thread}/fork"), Some(serde_json::json!({})));
    let fork = forked.as_str().map(str::to_owned).unwrap_or_else(|| forked["thread"].as_str().expect("fork").to_owned());
    let fork_data = format!("/api/threads/{fork}/artifacts/{id}/data");
    let inherited = alice.web("POST", &fork_data, Some(list.clone()));
    assert_eq!(inherited.as_array().map(Vec::len), Some(2), "the fork inherits the votes: {inherited}");
    let set = serde_json::json!({"op": "set", "path": format!("votes/{ann}"), "value": {"pick": "tacos"}});
    assert_eq!(alice.web_answer("POST", &fork_data, Some(set)).0, "200");
    let source = alice.web("POST", &data, Some(serde_json::json!({"op": "get", "path": format!("votes/{ann}")})));
    assert_eq!(source["data"]["pick"], "pizza", "the source keeps its own: {source}");
    let changed = alice.web("POST", &fork_data, Some(serde_json::json!({"op": "get", "path": format!("votes/{ann}")})));
    assert_eq!(changed["data"]["pick"], "tacos");

    // A read-only link's page reads the votes as they stood, and writes none (ADR 0072).
    let mut link = serde_json::Value::Null;
    assert!(
        eventually(|| {
            let body = serde_json::json!({"days": 1, "frozen": true, "artifact": {"id": id}});
            let (status, answer) = alice.web_answer("POST", &format!("/api/threads/{thread}/links"), Some(body));
            link = serde_json::from_str(&answer).unwrap_or_default();
            status == "200"
        }),
        "a link to the poll: {link}"
    );
    let content = format!("{}/content", link["url"].as_str().expect("the link"));
    let mut body = String::new();
    assert!(
        eventually(|| {
            body = std::process::Command::new("curl")
                .args(["-s", &content])
                .output()
                .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
                .unwrap_or_default();
            body.contains("\"pick\":\"pizza\"") && body.contains("\"pick\":\"soup\"")
        }),
        "the link's page has both votes: {body}"
    );
    assert!(body.contains("'not_granted'") && !body.contains("tacos"), "its frozen db, without the fork's vote: {body}");
}

type RoomSocket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// What a room's socket hears, up to its first moment or ten quiet seconds.
async fn heard(socket: &mut RoomSocket) -> Vec<serde_json::Value> {
    use futures_util::StreamExt as _;
    let mut seen = Vec::new();
    loop {
        let next = tokio::time::timeout(std::time::Duration::from_secs(10), socket.next()).await;
        let Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text)))) = next else { return seen };
        let event: serde_json::Value = serde_json::from_str(&text).expect("json");
        let moment = event["event"] == "moment";
        seen.push(event);
        if moment {
            return seen;
        }
    }
}

/// A page's room spans machines (ADR 0081): each daemon relays its views'
/// room to the server the thread syncs with, so a moment Alice's view emits
/// reaches Bob's view on his own machine.
#[tokio::test(flavor = "multi_thread")]
async fn a_rooms_moments_reach_a_view_on_another_machine() {
    use futures_util::SinkExt as _;
    use tokio_tungstenite::tungstenite::Message;
    let root = common::temp_root("silt-rooms");
    let origin = origin(root.path());
    let server = Server::start(&root.path().join("server"));
    let alice = Machine::new_for(root.path(), "alice", "alice", &origin, &server);
    let bob = Machine::new_for(root.path(), "bob", "bob", &origin, &server);
    let fake = alice.fake.with_file_name(format!("silt-fake-agent{}", std::env::consts::EXE_SUFFIX));
    let agents = serde_json::json!([{"id": "v1", "name": "Fake v1", "command": fake}]);
    fs::write(alice.silt_home.join("agents.json"), agents.to_string()).expect("write agents.json");
    let started = alice.web(
        "POST",
        "/api/agents/start",
        Some(serde_json::json!({"clone": alice.repo, "agent": "v1", "prompt": "Artifact board"})),
    );
    let thread = started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("thread").to_owned());
    let mut shown = Vec::new();
    assert!(eventually(|| {
        shown = artifacts(&bob, &thread);
        shown.len() == 1
    }));
    let id = shown[0]["id"].as_str().expect("id").to_owned();

    let join = |machine: &Machine| {
        let url = machine.stdout(&["open", "--no-browser"]);
        let (base, token) = common::web_ui(&url);
        let address = format!("{}/api/rooms?token={token}", base.trim_end_matches('/').replacen("http", "ws", 1));
        let join = serde_json::json!({"thread": thread, "artifact": id}).to_string();
        async move {
            let (mut socket, _) = tokio_tungstenite::connect_async(address).await.expect("connects");
            socket.send(Message::Text(join.into())).await.expect("joins");
            socket
        }
    };
    let mut theirs = join(&bob).await;
    let mut mine = join(&alice).await;
    // Alice owns the thread, so she may emit to any topic.
    mine.send(Message::Text(r#"{"op": "emit", "topic": "move", "data": {"x": 7}}"#.into())).await.expect("emits");
    let seen = heard(&mut theirs).await;
    let moment = seen.iter().find(|event| event["event"] == "moment").unwrap_or_else(|| panic!("bob hears it: {seen:?}"));
    assert_eq!((&moment["topic"], &moment["data"]["x"]), (&"move".into(), &7.into()));
    // Alice's agent speaks where the views meet, so Bob hears it too.
    alice.web("POST", &format!("/api/threads/{thread}/agent/prompt"), Some(serde_json::json!({"text": "Room say"})));
    let mut said = Vec::new();
    for _ in 0..10 {
        said.extend(heard(&mut theirs).await);
        let noted = said.iter().any(|event| event["event"] == "moment" && event["topic"] == "note");
        if noted && said.iter().any(|event| event["event"] == "presence" && event["peer"]["kind"] == "agent") {
            break;
        }
    }
    let note = said.iter().find(|event| event["event"] == "moment" && event["topic"] == "note");
    let note = note.unwrap_or_else(|| panic!("bob hears the agent: {said:?}"));
    assert_eq!((&note["data"]["said"], &note["from"]["kind"]), (&"hello".into(), &"agent".into()), "{note}");
    let present = said.iter().find(|event| event["event"] == "presence" && event["peer"]["kind"] == "agent");
    assert_eq!(
        present.map(|event| &event["peer"]["state"]["status"]),
        Some(&"listening".into()),
        "and sees what it's on: {said:?}"
    );
    drop(mine);
    drop(theirs);
}

/// A pin follows the person between their machines through their server
/// (ADR 0075).
#[test]
fn a_pin_made_on_one_machine_shows_on_another() {
    let world = world();
    let (laptop, home) = (&world.laptop, &world.home);
    let started = laptop.web(
        "POST",
        "/api/agents/start",
        Some(serde_json::json!({"clone": laptop.repo, "agent": "v1", "prompt": "Artifact"})),
    );
    let thread = started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("thread").to_owned());
    assert!(eventually(|| artifacts(home, &thread).len() == 1), "the other machine pulls it");
    let id = artifacts(laptop, &thread)[0]["id"].as_str().expect("id").to_owned();
    laptop.web("PUT", &format!("/api/threads/{thread}/artifacts/{id}/pin"), Some(serde_json::json!({"pinned": true})));
    let pinned_on = |machine: &Machine| {
        let listed = machine.web("GET", "/api/artifacts", None);
        listed.as_array().is_some_and(|rows| rows.iter().any(|row| row["artifact"]["id"] == id.as_str() && row["pinned"] == true))
    };
    assert!(eventually(|| pinned_on(home)), "pinned on the other machine too");
}

/// A person's own themes follow them (ADR 0099): a folder theme with a font,
/// made on the laptop, reaches the home machine and the hosted app through
/// the server, and removing it on the home machine removes it on the laptop.
#[test]
fn a_persons_themes_follow_them_between_machines() {
    let world = world();
    let on = |machine: &Machine| machine.silt_home.join("themes/harbour");
    fs::create_dir_all(on(&world.laptop).join("fonts")).expect("mkdir");
    let theme = r##"{ "name": "Harbour", "themes": [{ "id": "harbour", "name": "Harbour", "appearance": "light",
        "colors": { "bg": "#f7f8f6", "fg-1": "#22302c", "accent": "#2f7d6d" },
        "fonts": { "ui": ["Harbour Sans", "system-ui"], "faces": [{ "family": "Harbour Sans", "file": "fonts/sans.woff2" }] } }] }"##;
    fs::write(on(&world.laptop).join("theme.json"), theme).expect("theme");
    fs::write(on(&world.laptop).join("fonts/sans.woff2"), b"wOF2 a font").expect("font");

    let listed = |machine: &Machine| machine.web("GET", "/api/themes", None).to_string();
    assert!(listed(&world.laptop).contains(r#""id":"harbour""#), "{}", listed(&world.laptop));
    assert!(
        eventually(|| listed(&world.home).contains(r#""id":"harbour""#)),
        "the home machine has it: {}",
        world.home.daemon_log()
    );
    assert_eq!(fs::read_to_string(on(&world.home).join("theme.json")).expect("theme"), theme);
    assert_eq!(fs::read(on(&world.home).join("fonts/sans.woff2")).expect("font"), b"wOF2 a font");

    let browser = HostedBrowser::sign_in(&world.server, "owner", world._root.path().join("cookies"));
    let hosted = browser.call("GET", "/themes", None);
    let harbour = hosted["themes"].as_array().expect("themes").iter().find(|theme| theme["id"] == "harbour").cloned();
    assert_eq!(harbour.as_ref().map(|theme| theme["source"].clone()), Some(serde_json::json!("yours")), "{hosted}");
    let css = browser.call("GET", "/themes/css?light=harbour", None);
    assert!(css["css"].as_str().expect("css").contains("url(theme-fonts/harbour/sans.woff2)"), "{css}");
    let font = std::process::Command::new("curl")
        .args(["-s", "-b"])
        .arg(&browser.jar)
        .arg(format!("{}/app/theme-fonts/harbour/sans.woff2", world.server.url))
        .output()
        .expect("curl runs");
    assert_eq!(font.stdout, b"wOF2 a font");

    fs::remove_dir_all(on(&world.home)).expect("remove");
    assert!(eventually(|| !on(&world.laptop).exists()), "the removal reaches the laptop: {}", world.laptop.daemon_log());
}

/// The person's browser session on the hosted app, as `curl` keeps it.
struct HostedBrowser {
    base: String,
    jar: std::path::PathBuf,
}

impl HostedBrowser {
    fn sign_in(server: &Server, user: &str, jar: std::path::PathBuf) -> Self {
        use std::io::Write as _;
        let mut passwd = common::silt_command()
            .args(["server", "user", "passwd", user, "--password-stdin", "--data"])
            .arg(&server.data)
            .stdin(std::process::Stdio::piped())
            .spawn()
            .expect("passwd runs");
        writeln!(passwd.stdin.take().expect("stdin"), "correct horse battery").expect("writes");
        assert!(passwd.wait().expect("waits").success());
        let signed = std::process::Command::new("curl")
            .args(["-s", "-w", "%{http_code}", "-o"])
            .arg(jar.with_extension("page"))
            .arg("-c")
            .arg(&jar)
            .args(["--data-urlencode", &format!("user={user}"), "--data-urlencode", "password=correct horse battery"])
            .args(["--data-urlencode", "next=/app/", &format!("{}/login", server.url)])
            .output()
            .expect("curl runs");
        assert_eq!(String::from_utf8_lossy(&signed.stdout), "303", "signed in");
        Self { base: server.url.clone(), jar }
    }

    fn call(&self, method: &str, path: &str, body: Option<serde_json::Value>) -> serde_json::Value {
        let mut curl = std::process::Command::new("curl");
        curl.args(["-s", "-X", method, "-b"]).arg(&self.jar).args(["-H", "x-silt-request: 1"]);
        if let Some(body) = body {
            curl.args(["-H", "content-type: application/json", "--data-binary", &body.to_string()]);
        }
        let output = curl.arg(format!("{}/app/api{path}", self.base)).output().expect("curl runs");
        serde_json::from_slice(&output.stdout)
            .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(&output.stdout).into()))
    }
}

/// A page's question in the hosted app is answered by one of the person's
/// online computers, on the tier it names, and a tool the page gives it is
/// called through the server and answered by the page (ADR 0072).
#[test]
fn the_hosted_app_asks_a_pages_question_of_the_persons_computer() {
    let world = world();
    let laptop = &world.laptop;
    let started = laptop.web(
        "POST",
        "/api/agents/start",
        Some(serde_json::json!({"clone": laptop.repo, "agent": "v1", "prompt": "Artifact saver"})),
    );
    let thread = started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("thread").to_owned());
    let browser = HostedBrowser::sign_in(&world.server, "owner", world._root.path().join("cookies"));
    let mut id = String::new();
    assert!(
        eventually(|| {
            let history = browser.call("GET", &format!("/threads/{thread}/history"), None);
            let found = history["entries"]
                .as_array()
                .and_then(|entries| entries.iter().find_map(|entry| entry["artifact"]["id"].as_str().map(str::to_owned)));
            found.map(|found| id = found).is_some()
        }),
        "the hosted app has the artifact"
    );
    let sample = format!("/threads/{thread}/artifacts/{id}/sample");
    let refused = browser.call("POST", &sample, Some(serde_json::json!({"input": "hi"})));
    assert_eq!(refused["refusal"], "not_granted", "the server asks for the grant too: {refused}");
    let grant = serde_json::json!({"capability": "sample", "state": "granted"});
    browser.call("PUT", &format!("/threads/{thread}/artifacts/{id}/grants"), Some(grant));
    let mut answer = serde_json::Value::Null;
    assert!(
        eventually(|| {
            answer = browser.call(
                "POST",
                &sample,
                Some(serde_json::json!({"ask": "h1", "input": "Name three rivers", "model": {"model_tier": "quick"}})),
            );
            answer["text"].is_string()
        }),
        "the laptop answers: {answer}"
    );
    assert_eq!(answer["text"], "Sampled: Name three rivers (on haiku)", "{answer}");

    let tools = serde_json::json!([{"name": "lookup", "description": "Finds a row"}]);
    let answered = std::thread::scope(|scope| {
        let asking = scope.spawn(|| {
            browser.call("POST", &sample, Some(serde_json::json!({"ask": "h2", "input": "Which row?", "tools": tools})))
        });
        let mut call = serde_json::Value::Null;
        assert!(
            eventually(|| {
                let so_far = browser.call("GET", &format!("{sample}/h2"), None);
                call = so_far["calls"][0].clone();
                call.is_object()
            }),
            "the page hears of the agent's call"
        );
        assert_eq!(call["name"], "lookup");
        let result = browser.call(
            "POST",
            &format!("{sample}/h2/tools/{}", call["call"]),
            Some(serde_json::json!({"ok": true, "value": "row 7"})),
        );
        assert!(result.is_null() || result == "", "the answer reaches the laptop: {result}");
        asking.join().expect("asked")
    });
    assert_eq!(answered["text"], "Sampled: Which row? [lookup: row 7]", "{answered}");

    // A page that stops asking ends the run on the laptop, which waited on its tool.
    let began = std::time::Instant::now();
    let stopped = std::thread::scope(|scope| {
        let asking = scope.spawn(|| {
            browser.call("POST", &sample, Some(serde_json::json!({"ask": "h3", "input": "Which row?", "tools": tools})))
        });
        while !asking.is_finished() && began.elapsed() < std::time::Duration::from_secs(60) {
            browser.call("POST", &format!("{sample}/h3/stop"), None);
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        asking.join().expect("asked")
    });
    assert!(stopped["text"].is_null() && stopped.to_string().contains("stopped asking"), "{stopped}");
    assert!(began.elapsed() < std::time::Duration::from_secs(60), "well before the question's own wait");
}

/// With none of the person's devices online, the hosted app lists a
/// thread's files from the server's own fold of its deltas, and reads them
/// where the server has their text (ADR 0078).
#[test]
fn the_hosted_app_reads_a_threads_files_with_no_device_online() {
    let world = world();
    let laptop = &world.laptop;
    let started = laptop.web(
        "POST",
        "/api/agents/start",
        Some(serde_json::json!({"clone": laptop.repo, "agent": "v1", "prompt": "Artifact files"})),
    );
    let thread = started.as_str().map(str::to_owned).unwrap_or_else(|| started["thread"].as_str().expect("thread").to_owned());
    let browser = HostedBrowser::sign_in(&world.server, "owner", world._root.path().join("cookies"));
    let mut id = String::new();
    assert!(
        eventually(|| {
            let history = browser.call("GET", &format!("/threads/{thread}/history"), None);
            let found = history["entries"]
                .as_array()
                .and_then(|entries| entries.iter().find_map(|entry| entry["artifact"]["id"].as_str().map(str::to_owned)));
            found.map(|found| id = found).is_some()
        }),
        "the hosted app has the artifact"
    );
    let grant = serde_json::json!({"capability": "files", "state": "granted"});
    browser.call("PUT", &format!("/threads/{thread}/artifacts/{id}/grants"), Some(grant));
    laptop.silt(&["daemon", "stop"]);
    let files = format!("/threads/{thread}/artifacts/{id}/files");
    let mut listed = serde_json::Value::Null;
    assert!(
        eventually(|| {
            listed = browser.call("POST", &files, Some(serde_json::json!({"op": "list", "recursive": true})));
            listed["entries"].as_array().is_some_and(|entries| entries.iter().any(|entry| entry["path"] == "src/main.rs"))
        }),
        "the server lists the thread's files itself: {listed}"
    );
    let paths: Vec<&str> =
        listed["entries"].as_array().expect("entries").iter().filter_map(|entry| entry["path"].as_str()).collect();
    assert!(paths.iter().all(|path| path.starts_with("src/")), "only what the page declares: {paths:?}");
    let read = browser.call("POST", &files, Some(serde_json::json!({"op": "read", "path": "src/main.rs"})));
    assert!(
        read["text"].as_str().is_some_and(|text| text.contains("fn main")) || read.to_string().contains("isn't on the server"),
        "read from the mirror, or said why not: {read}"
    );
    let outside = browser.call("POST", &files, Some(serde_json::json!({"op": "read", "path": "README.md"})));
    assert!(outside["text"].is_null(), "outside its globs: {outside}");
}
