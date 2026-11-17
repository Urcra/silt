//! Devices relayed to a sandbox (ADR 0093): Silt holds each on this machine
//! and hands the thread's sandbox a session of it, which outlives the
//! agent. A serial port reaches the sandbox as a pseudo-terminal at the
//! port's own path; adb reaches this machine's adb server through a filter
//! that lets through only the one device's serial.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use silt_types::ThreadId;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpStream, UnixListener, UnixStream};

use crate::CoreError;
use crate::core::{Core, now_ms};
use crate::plugins::RelayView;
pub use crate::sandbox::Relayed;

/// adb's server on this machine.
const ADB_SERVER: ([u8; 4], u16) = ([127, 0, 0, 1], 5037);

/// A device as a workspace's rules name it: `/dev/ttyUSB0`, with
/// `@115200` for its speed, or `adb:SERIAL`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceSpec {
    Serial { path: PathBuf, baud: Option<u32> },
    Adb { serial: String },
}

impl DeviceSpec {
    pub fn parse(spec: &str) -> Result<Self, String> {
        if let Some(serial) = spec.strip_prefix("adb:") {
            let valid = !serial.is_empty() && serial.chars().all(|c| c.is_ascii_alphanumeric() || "-_.:".contains(c));
            return if valid {
                Ok(Self::Adb { serial: serial.to_owned() })
            } else {
                Err(format!("{serial} isn't an adb serial"))
            };
        }
        let (path, baud) = match spec.rsplit_once('@') {
            Some((path, baud)) => (path, Some(baud.parse::<u32>().map_err(|_| format!("{baud} isn't a speed"))?)),
            None => (spec, None),
        };
        let path = PathBuf::from(path);
        if !path.starts_with("/dev/") || path.components().any(|part| matches!(part, std::path::Component::ParentDir)) {
            return Err(format!("{} isn't a device under /dev", path.display()));
        }
        Ok(Self::Serial { path, baud })
    }

    fn key(&self) -> String {
        match self {
            Self::Serial { path, .. } => path.to_string_lossy().into_owned(),
            Self::Adb { serial } => format!("adb:{serial}"),
        }
    }
}

struct Relay {
    relayed: Relayed,
    since_ms: i64,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The relays the daemon holds, by thread and device.
#[derive(Default)]
pub struct Relays {
    held: BTreeMap<(ThreadId, String), Relay>,
    /// Another adb server than this machine's, for tests.
    adb_server: Option<std::net::SocketAddr>,
}

impl Core {
    /// The thread's relays of `specs`: those it holds already, kept as they
    /// are so a session survives the agent restarting, and new ones.
    pub(crate) async fn relay_devices(&mut self, thread: ThreadId, specs: &[String]) -> Result<Vec<Relayed>, CoreError> {
        let mut out = Vec::new();
        for spec in specs {
            let spec = DeviceSpec::parse(spec).map_err(CoreError::Invalid)?;
            let key = (thread, spec.key());
            if let Some(relay) = self.relays.held.get(&key).filter(|relay| !relay.task.is_finished()) {
                out.push(relay.relayed.clone());
                continue;
            }
            let (relayed, task) = match &spec {
                DeviceSpec::Serial { path, baud } => {
                    let (pts, task) = serial_relay(path, *baud)?;
                    (Relayed::Serial { device: path.clone(), pts }, task)
                }
                DeviceSpec::Adb { serial } => {
                    let dir = self.data_dir.join("devices");
                    silt_os::create_private_dir(&dir)?;
                    let socket = crate::sandbox::short_socket(&dir, &format!("{}-adb-{serial}.sock", thread.short()))?;
                    let _ = std::fs::remove_file(&socket);
                    let listener = UnixListener::bind(&socket)?;
                    let server = self.relays.adb_server.unwrap_or_else(|| ADB_SERVER.into());
                    (Relayed::Adb { serial: serial.clone(), socket }, tokio::spawn(adb_relay(listener, server, serial.clone())))
                }
            };
            out.push(relayed.clone());
            self.relays.held.insert(key, Relay { relayed, since_ms: now_ms(), task });
        }
        Ok(out)
    }

    /// Lets go of the thread's relay of `device`, ending its session.
    pub(crate) fn release_device(&mut self, thread: ThreadId, device: &str) -> Result<(), CoreError> {
        self.relays
            .held
            .remove(&(thread, device.to_owned()))
            .map(drop)
            .ok_or_else(|| CoreError::Invalid(format!("{device} isn't relayed to this thread")))
    }

    /// Lets go of every device relayed to the thread.
    pub(crate) fn release_devices(&mut self, thread: ThreadId) {
        self.relays.held.retain(|(held, _), _| *held != thread);
    }

    pub(crate) fn devices_view(&self, thread: ThreadId) -> Vec<RelayView> {
        self.relays
            .held
            .iter()
            .filter(|((held, _), relay)| *held == thread && !relay.task.is_finished())
            .map(|((_, device), relay)| RelayView {
                device: device.clone(),
                kind: match relay.relayed {
                    Relayed::Serial { .. } => "serial".into(),
                    Relayed::Adb { .. } => "adb".into(),
                },
                since_ms: relay.since_ms,
            })
            .collect()
    }
}

/// Opens `device` and a pseudo-terminal, and passes bytes between them
/// until the relay is dropped.
fn serial_relay(device: &Path, baud: Option<u32>) -> Result<(PathBuf, tokio::task::JoinHandle<()>), CoreError> {
    let port = silt_os::serial::open(device, baud)?;
    let pty = silt_os::serial::Pty::open()?;
    let pts = pty.path.clone();
    let task = tokio::spawn(async move {
        let _ = silt_os::serial::splice(port, pty).await;
    });
    Ok((pts, task))
}

/// Answers the sandbox's adb, one connection at a time each, through the
/// filter.
async fn adb_relay(listener: UnixListener, server: std::net::SocketAddr, serial: String) {
    while let Ok((client, _)) = listener.accept().await {
        let serial = serial.clone();
        tokio::spawn(async move {
            let _ = adb_connection(client, server, &serial).await;
        });
    }
}

async fn adb_connection(mut client: UnixStream, address: std::net::SocketAddr, serial: &str) -> std::io::Result<()> {
    let mut header = [0u8; 4];
    client.read_exact(&mut header).await?;
    let Some(length) = silt_access::adb::length(&header).filter(|length| *length <= 4096) else {
        return client.write_all(&silt_access::adb::fail("not an adb request")).await;
    };
    let mut payload = vec![0u8; length];
    client.read_exact(&mut payload).await?;
    let request = String::from_utf8_lossy(&payload).into_owned();
    match silt_access::adb::decide(&request, serial) {
        silt_access::adb::Decision::Refuse(why) => client.write_all(&silt_access::adb::fail(&why)).await,
        silt_access::adb::Decision::Pass(request) => {
            let mut server = TcpStream::connect(address).await?;
            server.write_all(&silt_access::adb::frame(&request)).await?;
            tokio::io::copy_bidirectional(&mut client, &mut server).await.map(drop)
        }
        silt_access::adb::Decision::Devices(request) => {
            let mut server = TcpStream::connect(address).await?;
            server.write_all(&silt_access::adb::frame(&request)).await?;
            let mut status = [0u8; 4];
            server.read_exact(&mut status).await?;
            if &status != b"OKAY" {
                return client.write_all(&silt_access::adb::fail("adb's server refused")).await;
            }
            let mut header = [0u8; 4];
            server.read_exact(&mut header).await?;
            let length = silt_access::adb::length(&header).unwrap_or_default().min(1 << 20);
            let mut list = vec![0u8; length];
            server.read_exact(&mut list).await?;
            let shown = silt_access::adb::only(&String::from_utf8_lossy(&list), serial);
            client.write_all(b"OKAY").await?;
            client.write_all(&silt_access::adb::frame(&shown)).await
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use silt_store::Store;
    use silt_types::ActorId;
    use tokio::net::TcpListener;

    use super::*;

    async fn core(data: &Path) -> Core {
        let store = Store::open(data).await.expect("opens");
        let (events, _inbox) = tokio::sync::mpsc::unbounded_channel();
        Core::load(store, ActorId(1), data, events).await.expect("loads")
    }

    #[test]
    fn only_devices_and_adb_serials_are_named() {
        assert_eq!(
            DeviceSpec::parse("/dev/ttyUSB0@115200"),
            Ok(DeviceSpec::Serial { path: "/dev/ttyUSB0".into(), baud: Some(115_200) })
        );
        assert_eq!(DeviceSpec::parse("adb:R58M"), Ok(DeviceSpec::Adb { serial: "R58M".into() }));
        for refused in ["/etc/shadow", "/dev/../etc/shadow", "adb:", "adb:a b", "/dev/ttyUSB0@fast"] {
            assert!(DeviceSpec::parse(refused).is_err(), "{refused}");
        }
    }

    #[tokio::test]
    async fn a_serial_session_outlives_the_agent_and_is_handed_back() {
        let data = tempfile::tempdir().expect("temp dir");
        let mut core = core(data.path()).await;
        // A terminal stands in for the board's port.
        let board = silt_os::serial::Pty::open().expect("a terminal for the board");
        let spec = vec![format!("{}@115200", board.path.display())];
        let thread = ThreadId(7);
        let first = core.relay_devices(thread, &spec).await.expect("relays");
        let again = core.relay_devices(thread, &spec).await.expect("relays");
        assert_eq!(first, again, "the same session, as the agent starts again");
        let Relayed::Serial { pts, .. } = &first[0] else { panic!("a serial relay") };
        assert_eq!(core.devices_view(thread).len(), 1);
        assert!(core.devices_view(ThreadId(8)).is_empty(), "another thread's sandbox gets none");
        let mut sandbox = std::fs::OpenOptions::new().read(true).write(true).open(pts).expect("the sandbox opens it");
        tokio::task::spawn_blocking(move || sandbox.write_all(b"AT\r")).await.expect("joins").expect("writes");
        core.release_device(thread, &board.path.to_string_lossy()).expect("releases");
        assert!(core.devices_view(thread).is_empty());
    }

    #[tokio::test]
    async fn the_sandboxs_adb_reaches_one_device_only() {
        let data = tempfile::tempdir().expect("temp dir");
        let mut core = core(data.path()).await;
        // An adb server that lists two devices and says OKAY to the rest.
        let server = TcpListener::bind("127.0.0.1:0").await.expect("binds");
        core.relays.adb_server = Some(server.local_addr().expect("address"));
        tokio::spawn(async move {
            while let Ok((mut client, _)) = server.accept().await {
                tokio::spawn(async move {
                    let mut header = [0u8; 4];
                    if client.read_exact(&mut header).await.is_err() {
                        return;
                    }
                    let mut request = vec![0u8; silt_access::adb::length(&header).unwrap_or_default()];
                    let _ = client.read_exact(&mut request).await;
                    let answer = if request == b"host:devices" {
                        let mut out = b"OKAY".to_vec();
                        out.extend(silt_access::adb::frame("R58M\tdevice\nemulator-5554\tdevice\n"));
                        out
                    } else {
                        b"OKAY".to_vec()
                    };
                    let _ = client.write_all(&answer).await;
                });
            }
        });
        let relayed = core.relay_devices(ThreadId(7), &["adb:R58M".to_owned()]).await.expect("relays");
        let Relayed::Adb { socket, .. } = &relayed[0] else { panic!("an adb relay") };
        let ask = |request: &'static str| {
            let socket = socket.clone();
            async move {
                let mut stream = UnixStream::connect(&socket).await.expect("connects");
                stream.write_all(&silt_access::adb::frame(request)).await.expect("asks");
                let mut answer = Vec::new();
                let _ = stream.read_to_end(&mut answer).await;
                String::from_utf8_lossy(&answer).into_owned()
            }
        };
        assert_eq!(ask("host:devices").await, "OKAY000cR58M\tdevice\n");
        assert!(ask("host:transport:emulator-5554").await.starts_with("FAIL"));
        assert!(ask("host:kill").await.starts_with("FAIL"));
        assert_eq!(ask("host:transport:R58M").await, "OKAY");
    }
}
