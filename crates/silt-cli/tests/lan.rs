//! Phones on the local network with no server (ADR 0112): a phone, played
//! by curl trusting only the machine's own authority, gets the certificate,
//! pairs once the person allows it here, and may do only what it was given.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{Machine, eventually, origin, temp_root};
use serde_json::{Value, json};

const IPHONE: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 19_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/19.0 Mobile/15E148 Safari/604.1";

/// Two free ports in a row: the listener's and the certificate's page's.
/// They're below every system's ephemeral range, where other tests'
/// listeners get theirs: Windows hands those out in turn, so the port after
/// one is often the next taken. macOS and Windows let a listener on 0.0.0.0
/// share a port with another's on 127.0.0.1, which then answers what curl
/// sends there, so both must be free.
fn free_ports() -> u16 {
    let free = |port: u16| {
        std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() && std::net::TcpListener::bind(("0.0.0.0", port)).is_ok()
    };
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("after 1970").subsec_nanos();
    let start = (seed ^ std::process::id()) % 10_000;
    (0..10_000)
        .map(|step| 20_000 + u16::try_from((start + step * 2) % 10_000).expect("small"))
        .find(|&port| free(port) && free(port + 1))
        .expect("two free ports")
}

/// A phone: its own cookies, trusting only the machine's authority, and
/// reaching the machine's `.local` name on loopback.
struct Phone {
    jar: PathBuf,
    authority: PathBuf,
    host: String,
    port: u16,
}

impl Phone {
    fn new(root: &Path, name: &str, machine: &Machine, host: &str, port: u16) -> Self {
        Self {
            jar: root.join(format!("{name}.cookies")),
            authority: machine.silt_home.join("lan/ca.pem"),
            host: host.to_owned(),
            port,
        }
    }

    /// The status and body of `method` `path`; `app` sends what the app
    /// sends with a change, its header and origin.
    fn ask(&self, method: &str, path: &str, body: Option<Value>, app: bool) -> (u16, String) {
        let mut command = Command::new("curl");
        command
            .args(["-s", "--noproxy", "*", "-w", "\n%{http_code}", "-X", method, "-A", IPHONE, "--cacert"])
            .arg(&self.authority)
            .arg("--resolve")
            .arg(format!("{}:{}:127.0.0.1", self.host, self.port))
            .arg("-c")
            .arg(&self.jar)
            .arg("-b")
            .arg(&self.jar);
        // Schannel, Windows' curl, can't check revocation for a private authority.
        if cfg!(windows) {
            command.arg("--ssl-no-revoke");
        }
        if app {
            command.args(["-H", "X-Silt-Request: 1", "-H"]).arg(format!("Origin: https://{}:{}", self.host, self.port));
        }
        if let Some(body) = body {
            command.args(["-H", "Content-Type: application/json", "--data"]).arg(body.to_string());
        }
        let output = command.arg(format!("https://{}:{}{path}", self.host, self.port)).output().expect("curl runs");
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        let (body, status) = text.rsplit_once('\n').unwrap_or(("", "0"));
        (status.parse().unwrap_or(0), body.to_owned())
    }

    fn get(&self, path: &str) -> (u16, String) {
        self.ask("GET", path, None, false)
    }
}

fn plain(port: u16, path: &str) -> (u16, Vec<u8>) {
    let output = Command::new("curl")
        .args(["-s", "--noproxy", "*", "-w", "\n%{http_code}"])
        .arg(format!("http://127.0.0.1:{}{path}", port + 1))
        .output()
        .expect("curl runs");
    let split = output.stdout.iter().rposition(|byte| *byte == b'\n').unwrap_or(0);
    let status = String::from_utf8_lossy(&output.stdout[split + 1..]).parse().unwrap_or(0);
    (status, output.stdout[..split].to_vec())
}

fn pem_body(pem: &str) -> Vec<u8> {
    use base64::Engine as _;
    let body: String = pem.lines().filter(|line| !line.starts_with("-----")).collect();
    base64::engine::general_purpose::STANDARD.decode(body).expect("base64")
}

#[test]
fn a_phone_on_the_network_pairs_with_no_server_and_may_do_only_what_it_was_given() {
    let root = temp_root("silt-lan");
    let origin = origin(root.path());
    let machine = Machine::without_server(root.path(), "workstation", &origin);
    let port = free_ports();
    let port_text = port.to_string();
    let envs = [("SILT_LAN_PORT", port_text.as_str())];

    let said = String::from_utf8_lossy(&machine.silt_with(&["lan", "on"], &envs).stdout).into_owned();
    assert!(said.contains("phones on this network: on") || said.contains("isn't on a local network"), "{said}");
    let view = machine.web("GET", "/api/lan", None);
    let host = view["host"].as_str().expect("a .local name").to_owned();
    assert!(host.starts_with("silt-") && host.ends_with(".local"), "{host}");
    assert_eq!(view["port"], port);

    // The certificate's page, over plain HTTP: the authority, and nothing else.
    let (status, der) = plain(port, "/silt-ca.crt");
    assert_eq!(status, 200);
    let authority = std::fs::read_to_string(machine.silt_home.join("lan/ca.pem")).expect("the authority");
    assert_eq!(der, pem_body(&authority));
    let (status, page) = plain(port, "/trust");
    assert_eq!(status, 200);
    assert!(String::from_utf8_lossy(&page).contains(&view["fingerprint"].as_str().expect("fingerprint")[..11]));
    assert_eq!(plain(port, "/api/threads").0, 404, "the plain port serves no API");

    let phone = Phone::new(root.path(), "phone", &machine, &host, port);
    assert_eq!(phone.get("/api/threads").0, 401, "unpaired");
    let (status, _) = {
        let output = Command::new("curl")
            .args(["-s", "--noproxy", "*", "-k", "-o", "/dev/null", "-w", "%{http_code}"])
            .arg(format!("https://127.0.0.1:{port}/api/threads"))
            .output()
            .expect("curl runs");
        (String::from_utf8_lossy(&output.stdout).into_owned(), ())
    };
    assert_eq!(status, "403", "a host that isn't this machine's own name or address, as DNS rebinding would send");

    // A code, opened by the phone: it waits for the person, and the code is used up.
    let pairing = machine.web("POST", "/api/lan/pair", Some(json!({})))["pairing"].clone();
    let code = pairing["code"].as_str().expect("a code").replace('-', "");
    assert!(pairing["url"].as_str().unwrap().ends_with(&format!("/pair/{code}")));
    let (status, waiting) = phone.get(&format!("/pair/{code}"));
    assert_eq!(status, 200, "{waiting}");
    assert!(waiting.contains("Waiting for you to allow this phone"), "{waiting}");
    let other = Phone::new(root.path(), "other", &machine, &host, port);
    assert_eq!(other.get(&format!("/pair/{code}")).0, 404, "a code pairs once");
    assert_eq!(phone.get("/api/threads").0, 401, "not until the person allows it");

    let asks = machine.web("GET", "/api/lan", None)["asks"].clone();
    assert_eq!(asks[0]["phone"], "iPhone · Safari 19");
    assert_eq!(asks[0]["from"], "127.0.0.1");
    machine.web("POST", &format!("/api/lan/asks/{}", asks[0]["id"].as_str().unwrap()), Some(json!({"allow": true})));
    let (status, paired) = phone.get(&format!("/pair/{code}"));
    assert_eq!(status, 200);
    assert!(paired.contains("answer permissions and review merge requests"), "{paired}");
    assert!(
        paired.contains(&format!(":{}/trust\"", port + 1)),
        "the certificate, for a phone that went past the warning: {paired}"
    );

    // Paired: it reads, and is refused what it wasn't given.
    let (status, threads) = phone.get("/api/threads");
    assert_eq!(status, 200, "{threads}");
    let (status, me) = phone.get("/api/lan/me");
    assert_eq!(status, 200, "{me}");
    let me: Value = serde_json::from_str(&me).unwrap();
    assert_eq!(me["capabilities"]["send_prompts"], false);
    assert!(me["push_key"].as_str().is_some_and(|key| key.len() > 80));
    assert_eq!(phone.get("/api/lan").0, 403, "a phone doesn't change this machine");
    let cancel = "/api/threads/535c3cecd5e3/agent/cancel";
    let (status, said) = phone.ask("POST", cancel, None, false);
    assert_eq!(status, 403, "a change without the app's header and origin: {said}");
    let (status, said) = phone.ask("POST", cancel, None, true);
    assert_eq!(status, 403, "{said}");
    assert!(said.contains("may not send prompts"), "{said}");
    let (status, said) = phone.ask("POST", "/api/lan/forget", None, true);
    assert_eq!(status, 403, "{said}");

    // Given more, it may; signed out, it may nothing.
    let id = machine.web("GET", "/api/lan", None)["phones"][0]["id"].as_str().expect("the phone").to_owned();
    let mut capabilities = me["capabilities"].clone();
    capabilities["send_prompts"] = json!(true);
    machine.web("PATCH", &format!("/api/lan/phones/{id}"), Some(json!({"capabilities": capabilities})));
    let (status, said) = phone.ask("POST", cancel, None, true);
    assert_ne!(status, 403, "{said}");
    machine.web("POST", &format!("/api/lan/phones/{id}/revoke"), None);
    assert_eq!(phone.get("/api/threads").0, 401);

    // It stays on across a restart, and off once turned off.
    machine.silt(&["daemon", "stop"]);
    let status = String::from_utf8_lossy(&machine.silt_with(&["lan", "status"], &envs).stdout).into_owned();
    assert!(!status.contains(": off"), "{status}");
    assert!(eventually(|| phone.get("/api/threads").0 == 401), "listening again after the restart");
    machine.silt_with(&["lan", "off"], &envs);
    assert!(eventually(|| phone.get("/api/threads").0 == 0), "nothing listens once it's off");
}
