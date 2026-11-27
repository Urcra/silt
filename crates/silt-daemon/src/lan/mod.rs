//! Phones on the local network, with no server (ADR 0112): a second
//! listener, on every interface and over HTTPS only, with a certificate
//! authority of this machine's own; pairing by a one-time code the person
//! allows here; and pushes from this machine.

mod certs;
mod mdns;
pub(crate) mod needs;
mod pages;
mod phones;
mod push;
pub(crate) mod routes;
mod serve;

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};
use silt_proto::{AgentState, Capabilities};
use silt_types::ThreadId;
use tokio::sync::broadcast;
use tracing::warn;

use crate::protocol::BusEvent;
pub(crate) use certs::is_private;
pub(crate) use phones::PushSubscription;
use phones::{Pairing, PhoneBook};

/// The port phones reach this machine on, so a phone's saved app keeps its
/// address; the next one up serves the certificate over plain HTTP. Taken,
/// as by another person's Silt on the same computer, the next free pair is
/// used and remembered.
pub const DEFAULT_PORT: u16 = 7419;
/// How many pairs of ports to try, from [`DEFAULT_PORT`] up.
const PORT_TRIES: u16 = 20;
const CONFIG_FILE: &str = "config.json";
/// Heard from in the last two minutes.
const ONLINE_MS: i64 = 2 * 60 * 1000;
/// How often the addresses are looked at again.
const LOOK_AGAIN: Duration = Duration::from_secs(20);
/// How often what phones last did is written down.
const SAVE_SEEN: Duration = Duration::from_secs(60);

/// How it stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum LanState {
    Off,
    On,
    /// On, but this machine has no address on a local network.
    NoNetwork,
    /// Another program has the port.
    PortTaken,
    Failed,
}

/// A phone asking to pair, for the person to allow or deny here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LanAsk {
    pub id: String,
    /// What its browser says it is: `iPhone · Safari 19`.
    pub phone: String,
    /// The address it asked from.
    pub from: String,
    /// The code it opened, as shown.
    pub code: String,
    pub capabilities: Capabilities,
    pub made_ms: i64,
}

/// A phone paired over the network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LanPhone {
    pub id: String,
    pub name: String,
    pub capabilities: Capabilities,
    pub created_ms: i64,
    pub last_seen_ms: i64,
    pub online: bool,
    /// Whether it gets pushes.
    pub notifies: bool,
}

/// The code on show, and the QR codes that carry it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LanPairing {
    /// As shown: `K7QM-2XPA`.
    pub code: String,
    /// The page that pairs, by this machine's `.local` name.
    pub url: String,
    pub qr_svg: String,
    /// The same by address, for a network that doesn't pass `.local` names.
    pub address_url: Option<String>,
    pub address_qr_svg: Option<String>,
    pub capabilities: Capabilities,
    pub expires_ms: i64,
}

/// Phones on the local network, as Devices shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct LanView {
    pub state: LanState,
    /// What went wrong, when it failed.
    pub error: Option<String>,
    /// This machine, as people name it.
    pub machine: String,
    /// Whose Silt this is on it, as the OS names them.
    pub person: Option<String>,
    /// Its name on the network: `silt-workstation.local`.
    pub host: String,
    pub port: u16,
    pub addresses: Vec<String>,
    /// The authority's SHA-256, as a phone shows it.
    pub fingerprint: Option<String>,
    /// When the listener's certificate is next made again.
    pub renews_ms: Option<i64>,
    /// The page a phone gets the authority from, and its QR code.
    pub trust_url: Option<String>,
    pub trust_qr_svg: Option<String>,
    /// What lets port `port` through this machine's firewall, when it may
    /// be in the way.
    pub firewall: Option<String>,
    pub asks: Vec<LanAsk>,
    pub phones: Vec<LanPhone>,
    pub pairing: Option<LanPairing>,
}

/// A phone's push subscription, as its browser's `toJSON()` gives it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct Subscription {
    pub endpoint: String,
    pub keys: SubscriptionKeys,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct SubscriptionKeys {
    pub p256dh: String,
    pub auth: String,
}

/// What's kept in `lan/config.json`: whether it's on, and the name and port
/// phones know this machine by, which stay put once a phone trusts them.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Config {
    on: bool,
    #[serde(default)]
    host: Option<String>,
    #[serde(default)]
    port: Option<u16>,
}

impl Config {
    fn load(dir: &Path) -> Self {
        std::fs::read(dir.join(CONFIG_FILE)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
    }

    fn save(&self, dir: &Path) -> std::io::Result<()> {
        silt_os::create_private_dir(dir)?;
        silt_os::write_private(&dir.join(CONFIG_FILE), &serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)
    }
}

/// The phone a request on the network comes from.
#[derive(Debug, Clone)]
pub(crate) struct Caller {
    pub(crate) phone: String,
    pub(crate) capabilities: Capabilities,
}

tokio::task_local! {
    /// The phone whose request this is, on the network listener; requests
    /// there without one are refused (ADR 0112).
    pub(crate) static CALLER: Caller;
}

/// What the network listener reads while it runs.
pub(crate) struct Shared {
    pub(crate) dir: PathBuf,
    port: AtomicU16,
    pub(crate) machine: String,
    /// Whose Silt this is on the machine, so people sharing a network, or a
    /// computer, tell theirs apart.
    pub(crate) person: Option<String>,
    pub(crate) phones: RwLock<PhoneBook>,
    pub(crate) pairing: Mutex<Pairing>,
    pub(crate) names: RwLock<certs::Names>,
    /// The authority's certificate as DER, and its fingerprint.
    pub(crate) authority: RwLock<Option<(Vec<u8>, String)>>,
    pub(crate) bus: broadcast::Sender<BusEvent>,
    pub(crate) push: push::Pusher,
    seen_dirty: Mutex<bool>,
}

impl Shared {
    pub(crate) fn announce(&self) {
        let _ = self.bus.send(BusEvent::Lan);
    }

    /// The phone `token` belongs to, noting it was just heard from.
    pub(crate) fn phone_by_token(&self, token: &str) -> Option<Caller> {
        let mut phones = self.phones.write().expect("lock");
        let phone = phones.by_token(token)?.id.clone();
        let phone = phones.get_mut(&phone)?;
        let now = now_ms();
        if now - phone.last_seen_ms > 10_000 {
            phone.last_seen_ms = now;
            *self.seen_dirty.lock().expect("lock") = true;
        }
        Some(Caller { phone: phone.id.clone(), capabilities: phone.capabilities })
    }

    fn save_phones(&self) -> std::io::Result<()> {
        *self.seen_dirty.lock().expect("lock") = false;
        self.phones.read().expect("lock").save(&self.dir)
    }

    pub(crate) fn port(&self) -> u16 {
        self.port.load(Ordering::Relaxed)
    }

    /// `Silt on workstation (ada)`: how the machine and person show to a phone.
    pub(crate) fn label(&self) -> String {
        match &self.person {
            Some(person) => format!("Silt on {} ({person})", self.machine),
            None => format!("Silt on {}", self.machine),
        }
    }

    /// The hosts the listener answers to: its name and its addresses.
    pub(crate) fn hosts(&self) -> Vec<String> {
        let names = self.names.read().expect("lock");
        let port = self.port();
        let mut hosts = vec![format!("{}:{port}", names.host)];
        hosts.extend(names.addresses.iter().map(|address| match address {
            IpAddr::V4(v4) => format!("{v4}:{port}"),
            IpAddr::V6(v6) => format!("[{v6}]:{port}"),
        }));
        hosts
    }
}

fn now_ms() -> i64 {
    crate::core::now_ms()
}

/// The port `SILT_LAN_PORT` fixes, if it does.
fn fixed_port() -> Option<u16> {
    std::env::var("SILT_LAN_PORT").ok().and_then(|port| port.parse().ok())
}

/// The person this daemon runs for, as the OS names them.
fn person() -> Option<String> {
    ["USER", "USERNAME"].into_iter().find_map(|name| std::env::var(name).ok()).filter(|name| !name.trim().is_empty())
}

/// Binds `port` and the one after it; with `search`, the first free pair
/// from `port` up instead, two at a time.
async fn bind_pair(port: u16, search: bool) -> std::io::Result<(tokio::net::TcpListener, tokio::net::TcpListener, u16)> {
    let tries = if search { PORT_TRIES } else { 1 };
    let mut last = None;
    for step in 0..tries {
        let Some(secure) = port.checked_add(step * 2) else { break };
        let Some(plain) = secure.checked_add(1) else { break };
        match (serve::bind(secure).await, serve::bind(plain).await) {
            (Ok(secure_listener), Ok(plain_listener)) => return Ok((secure_listener, plain_listener, secure)),
            (Err(error), _) | (_, Err(error)) => last = Some(error),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::from(std::io::ErrorKind::AddrInUse)))
}

/// This machine's private addresses, phones' way to it; `SILT_LAN_ADDRESSES`,
/// comma-separated, names them instead, for tests and fixtures.
fn private_addresses() -> Vec<IpAddr> {
    if let Ok(named) = std::env::var("SILT_LAN_ADDRESSES") {
        return named
            .split(',')
            .filter_map(|address| address.trim().parse().ok())
            .filter(|address| is_private(*address))
            .collect();
    }
    let mut addresses: Vec<IpAddr> = if_addrs::get_if_addrs()
        .map(|interfaces| {
            interfaces.into_iter().filter(|interface| !interface.is_loopback()).map(|interface| interface.ip()).collect()
        })
        .unwrap_or_default();
    addresses.retain(|address| is_private(*address));
    // IPv4 first: phones and routers reach it most surely.
    addresses.sort_by_key(|address| (address.is_ipv6(), *address));
    addresses.dedup();
    addresses
}

/// A QR code for `text`, as an SVG image.
pub(crate) fn qr_svg(text: &str) -> Option<String> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    Some(code.render::<qrcode::render::svg::Color>().min_dimensions(200, 200).build())
}

/// What lets phones through this machine's firewall, if it may have one.
fn firewall_hint(port: u16) -> Option<String> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    match std::env::consts::OS {
        "linux" if silt_os::find_program("ufw", &path).is_some() => Some(format!("sudo ufw allow {port}/tcp")),
        "linux" if silt_os::find_program("firewall-cmd", &path).is_some() => {
            Some(format!("sudo firewall-cmd --add-port={port}/tcp --permanent && sudo firewall-cmd --reload"))
        }
        "macos" => Some("allow incoming connections for silt when macOS asks".to_owned()),
        "windows" => Some("allow silt on private networks when Windows Defender Firewall asks".to_owned()),
        _ => None,
    }
}

struct Running {
    current: Arc<certs::Current>,
    leaf: certs::Leaf,
    tasks: Vec<tokio::task::AbortHandle>,
    mdns: Option<mdns::Advertised>,
}

impl Drop for Running {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Phones on the network, as the daemon runs them.
pub(crate) struct Lan {
    pub(crate) shared: Arc<Shared>,
    config: Config,
    state: LanState,
    error: Option<String>,
    authority: Option<certs::Authority>,
    running: Option<Running>,
    looked: Option<Instant>,
    saved_seen: Instant,
    /// What each thread's agent was doing, to push only when it changes.
    agents: HashMap<ThreadId, AgentState>,
}

impl Lan {
    pub(crate) fn new(data_dir: &Path, bus: broadcast::Sender<BusEvent>) -> Self {
        let dir = data_dir.join("lan");
        let phones = PhoneBook::load(&dir).unwrap_or_else(|error| {
            warn!(%error, "could not read the phones paired over the network");
            PhoneBook::default()
        });
        // `SILT_LAN_NAME` names the machine to phones, as the person likes it.
        let machine = std::env::var("SILT_LAN_NAME")
            .ok()
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty())
            .or_else(silt_os::host_name)
            .unwrap_or_else(|| "this machine".to_owned());
        let config = Config::load(&dir);
        let host = config.host.clone().unwrap_or_else(|| certs::local_name(&machine, &certs::new_suffix()));
        let shared = Arc::new(Shared {
            push: push::Pusher::new(&dir),
            port: AtomicU16::new(fixed_port().or(config.port).unwrap_or(DEFAULT_PORT)),
            dir,
            person: person(),
            machine,
            phones: RwLock::new(phones),
            pairing: Mutex::default(),
            names: RwLock::new(certs::Names { host, addresses: Vec::new() }),
            authority: RwLock::new(None),
            bus,
            seen_dirty: Mutex::new(false),
        });
        Self {
            shared,
            config,
            state: LanState::Off,
            error: None,
            authority: None,
            running: None,
            looked: None,
            saved_seen: Instant::now(),
            agents: HashMap::new(),
        }
    }

    /// Whether the person turned it on, here or before a restart.
    pub(crate) fn wanted(&self) -> bool {
        self.config.on
    }

    fn remember(&mut self, on: bool) -> std::io::Result<()> {
        self.config.on = on;
        self.config.save(&self.shared.dir)
    }

    /// Turns it on or off, remembering which for the next start.
    pub(crate) async fn turn(&mut self, on: bool, app: impl FnOnce() -> axum::Router) -> std::io::Result<()> {
        self.remember(on)?;
        if on {
            self.start(app()).await
        } else {
            self.stop()
        }
        Ok(())
    }

    /// Starts listening, serving `app` to paired phones.
    pub(crate) async fn start(&mut self, app: axum::Router) {
        if self.running.is_some() {
            return;
        }
        match self.listen(app).await {
            Ok(running) => {
                self.running = Some(running);
                self.error = None;
                self.state =
                    if self.shared.names.read().expect("lock").addresses.is_empty() { LanState::NoNetwork } else { LanState::On };
            }
            Err(error) => {
                self.state = if error.kind() == std::io::ErrorKind::AddrInUse { LanState::PortTaken } else { LanState::Failed };
                warn!(%error, "phones on the network can't reach this machine");
                self.error = Some(error.to_string());
            }
        }
        self.shared.announce();
    }

    async fn listen(&mut self, app: axum::Router) -> std::io::Result<Running> {
        let now = SystemTime::now();
        let dir = self.shared.dir.clone();
        let host = self.shared.names.read().expect("lock").host.clone();
        let authority = match self.authority.take() {
            Some(authority) => authority,
            None => {
                let (host, label) = (host.clone(), self.shared.label());
                tokio::task::spawn_blocking(move || certs::Authority::load_or_create(&dir, &host, &label, now))
                    .await
                    .map_err(std::io::Error::other)??
            }
        };
        let names = certs::Names { host, addresses: private_addresses() };
        let leaf = authority.leaf(&names, now)?;
        *self.shared.names.write().expect("lock") = names.clone();
        *self.shared.authority.write().expect("lock") = Some((authority.der().to_vec(), authority.fingerprint()));
        let current = Arc::new(certs::Current::new(leaf.key.clone()));
        // A port phones already know stays, even taken: moving would strand them.
        let (wanted, search) = match fixed_port().or(self.config.port) {
            Some(port) => (port, false),
            None => (DEFAULT_PORT, true),
        };
        let (secure, plain, port) = bind_pair(wanted, search).await?;
        self.shared.port.store(port, Ordering::Relaxed);
        if self.config.host.as_deref() != Some(names.host.as_str()) || (fixed_port().is_none() && self.config.port != Some(port))
        {
            self.config.host = Some(names.host.clone());
            if fixed_port().is_none() {
                self.config.port = Some(port);
            }
            self.config.save(&self.shared.dir)?;
        }
        let tasks =
            vec![serve::secure(secure, current.clone(), app), serve::plain(plain, routes::trust_routes(self.shared.clone()))];
        let mdns = mdns::advertise(&self.shared.label(), &names.host, port);
        self.authority = Some(authority);
        self.looked = Some(Instant::now());
        Ok(Running { current, leaf, tasks, mdns })
    }

    fn stop(&mut self) {
        self.running = None;
        self.state = LanState::Off;
        self.error = None;
        self.shared.announce();
    }

    /// Signs every phone out and forgets the authority; phones need the
    /// new one. Starts again if it was on.
    pub(crate) async fn forget(&mut self, app: impl FnOnce() -> axum::Router) -> std::io::Result<()> {
        let was_running = self.running.is_some();
        self.running = None;
        self.authority = None;
        *self.shared.authority.write().expect("lock") = None;
        certs::Authority::forget(&self.shared.dir)?;
        // A new authority, a new name: phones that trusted the old one can't be fooled by the new.
        let host = certs::local_name(&self.shared.machine, &certs::new_suffix());
        self.shared.names.write().expect("lock").host = host.clone();
        self.config.host = Some(host);
        self.config.save(&self.shared.dir)?;
        *self.shared.phones.write().expect("lock") = PhoneBook::default();
        *self.shared.pairing.lock().expect("lock") = Pairing::default();
        self.shared.save_phones()?;
        if was_running {
            self.start(app()).await;
        } else {
            self.shared.announce();
        }
        Ok(())
    }

    /// Looks at the addresses now and then, making the certificate again
    /// when they changed or it nears its end; writes down when phones
    /// were last heard from.
    pub(crate) fn tick(&mut self) {
        let now_ms = now_ms();
        let waiting = {
            let mut pairing = self.shared.pairing.lock().expect("lock");
            let before = pairing.waiting().count();
            pairing.expire(now_ms);
            before != pairing.waiting().count()
        };
        if waiting {
            self.shared.announce();
        }
        if self.saved_seen.elapsed() >= SAVE_SEEN && *self.shared.seen_dirty.lock().expect("lock") {
            self.saved_seen = Instant::now();
            if let Err(error) = self.shared.save_phones() {
                warn!(%error, "could not note when phones were last heard from");
            }
        }
        let (Some(running), Some(authority)) = (self.running.as_mut(), self.authority.as_ref()) else { return };
        if self.looked.is_some_and(|looked| looked.elapsed() < LOOK_AGAIN) {
            return;
        }
        self.looked = Some(Instant::now());
        let names = certs::Names { host: running.leaf.names.host.clone(), addresses: private_addresses() };
        let now = SystemTime::now();
        if running.leaf.stale(&names, now) {
            match authority.leaf(&names, now) {
                Ok(leaf) => {
                    running.current.replace(leaf.key.clone());
                    running.leaf = leaf;
                    *self.shared.names.write().expect("lock") = names.clone();
                    if let Some(mdns) = &running.mdns {
                        mdns.readdress();
                    }
                }
                Err(error) => warn!(%error, "could not make the network listener's certificate again"),
            }
        }
        let state = if names.addresses.is_empty() { LanState::NoNetwork } else { LanState::On };
        if state != self.state {
            self.state = state;
            self.shared.announce();
        }
    }

    pub(crate) fn view(&self) -> LanView {
        let shared = &self.shared;
        let names = shared.names.read().expect("lock").clone();
        let now = now_ms();
        let on = self.running.is_some();
        let first_address = names.addresses.first().map(|address| match address {
            IpAddr::V4(v4) => v4.to_string(),
            IpAddr::V6(v6) => format!("[{v6}]"),
        });
        let trust_url =
            first_address.as_ref().filter(|_| on).map(|address| format!("http://{address}:{}/trust", shared.port() + 1));
        let pairing = shared.pairing.lock().expect("lock");
        let shown = pairing.code.as_ref().filter(|code| code.expires_ms > now && on).map(|code| {
            let url = format!("https://{}:{}/pair/{}", names.host, shared.port(), code.code);
            let address_url =
                first_address.as_ref().map(|address| format!("https://{address}:{}/pair/{}", shared.port(), code.code));
            LanPairing {
                code: phones::shown(&code.code),
                qr_svg: qr_svg(&url).unwrap_or_default(),
                address_qr_svg: address_url.as_deref().and_then(qr_svg),
                url,
                address_url,
                capabilities: code.capabilities,
                expires_ms: code.expires_ms,
            }
        });
        LanView {
            state: self.state,
            error: self.error.clone(),
            machine: shared.machine.clone(),
            person: shared.person.clone(),
            host: names.host.clone(),
            port: shared.port(),
            addresses: names.addresses.iter().map(ToString::to_string).collect(),
            fingerprint: shared.authority.read().expect("lock").as_ref().map(|(_, fingerprint)| fingerprint.clone()),
            renews_ms: self.running.as_ref().map(|running| {
                running
                    .leaf
                    .renews()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map_or(0, |since| i64::try_from(since.as_millis()).unwrap_or(i64::MAX))
            }),
            trust_qr_svg: trust_url.as_deref().and_then(qr_svg),
            trust_url,
            firewall: firewall_hint(shared.port()),
            asks: pairing
                .waiting()
                .map(|ask| LanAsk {
                    id: ask.id.clone(),
                    phone: ask.phone.clone(),
                    from: ask.from.clone(),
                    code: phones::shown(&ask.code),
                    capabilities: ask.capabilities,
                    made_ms: ask.made_ms,
                })
                .collect(),
            phones: shared
                .phones
                .read()
                .expect("lock")
                .phones
                .iter()
                .map(|phone| LanPhone {
                    id: phone.id.clone(),
                    name: phone.name.clone(),
                    capabilities: phone.capabilities,
                    created_ms: phone.created_ms,
                    last_seen_ms: phone.last_seen_ms,
                    online: on && now - phone.last_seen_ms < ONLINE_MS,
                    notifies: phone.push.is_some(),
                })
                .collect(),
            pairing: shown,
        }
    }

    /// Shows a new code, which a phone pairs with once.
    pub(crate) fn pair(&mut self, capabilities: Capabilities) -> Result<LanView, String> {
        if self.running.is_none() {
            return Err("turn on phones on this network first".to_owned());
        }
        self.shared.pairing.lock().expect("lock").start(capabilities, now_ms());
        self.shared.announce();
        Ok(self.view())
    }

    pub(crate) fn answer(&mut self, ask: &str, allow: bool) -> Result<LanView, String> {
        let added = {
            let mut phones = self.shared.phones.write().expect("lock");
            self.shared
                .pairing
                .lock()
                .expect("lock")
                .answer(ask, allow, &mut phones, now_ms())
                .map_err(|error| error.to_string())?
        };
        if added.is_some() {
            self.shared.save_phones().map_err(|error| error.to_string())?;
        }
        self.shared.announce();
        Ok(self.view())
    }

    pub(crate) fn change(
        &mut self,
        phone: &str,
        name: Option<String>,
        capabilities: Option<Capabilities>,
    ) -> Result<LanView, String> {
        {
            let mut phones = self.shared.phones.write().expect("lock");
            let found = phones.get_mut(phone).ok_or("no phone of that id is paired")?;
            if let Some(name) = name.map(|name| name.trim().to_owned()).filter(|name| !name.is_empty()) {
                found.name = name;
            }
            if let Some(capabilities) = capabilities {
                found.capabilities = capabilities;
            }
        }
        self.shared.save_phones().map_err(|error| error.to_string())?;
        self.shared.announce();
        Ok(self.view())
    }

    pub(crate) fn revoke(&mut self, phone: &str) -> Result<LanView, String> {
        self.shared.phones.write().expect("lock").remove(phone).ok_or("no phone of that id is paired")?;
        self.shared.save_phones().map_err(|error| error.to_string())?;
        self.shared.announce();
        Ok(self.view())
    }

    pub(crate) fn subscribe(&mut self, phone: &str, subscription: Option<Subscription>) -> Result<(), String> {
        if let Some(subscription) = &subscription
            && !subscription.endpoint.starts_with("https://")
        {
            return Err("a push service is reached over HTTPS".to_owned());
        }
        {
            let mut phones = self.shared.phones.write().expect("lock");
            let found = phones.get_mut(phone).ok_or("no phone of that id is paired")?;
            found.push = subscription.map(|subscription| PushSubscription {
                endpoint: subscription.endpoint,
                p256dh: subscription.keys.p256dh,
                auth: subscription.keys.auth,
            });
        }
        self.shared.save_phones().map_err(|error| error.to_string())?;
        self.shared.announce();
        Ok(())
    }

    /// Pushes to the paired phones when `thread`'s agent starts waiting on a
    /// permission or a plan, or stops with an error (ADR 0028's kinds).
    pub(crate) fn agent_changed(&mut self, thread: ThreadId, status: Option<&crate::protocol::AgentStatus>) {
        let state = status.map(|status| status.state);
        let before = match state {
            Some(state) => self.agents.insert(thread, state),
            None => self.agents.remove(&thread),
        };
        let (Some(state), Some(status)) = (state, status) else { return };
        if before == Some(state) || self.running.is_none() {
            return;
        }
        let Some((title, body)) = notice(&self.shared.machine, status) else { return };
        let targets: Vec<_> =
            self.shared.phones.read().expect("lock").phones.iter().filter_map(|phone| phone.push.clone()).collect();
        if !targets.is_empty() {
            self.shared.push.send(targets, push::Payload::new(title, &body, thread));
        }
    }
}

/// What a push says when an agent comes to `status`: waiting on a
/// permission or a plan, or stopped with an error; nothing for the rest.
fn notice(machine: &str, status: &crate::protocol::AgentStatus) -> Option<(&'static str, String)> {
    let agent = &status.agent;
    Some(match status.state {
        AgentState::AwaitingPermission => match (status.permissions.first(), status.escalations.first()) {
            (None, Some(escalated)) => (
                "An agent asks you to allow what Auto refused",
                format!("{agent} on {machine} asks you to allow: {}", escalated.call),
            ),
            (asked, _) => {
                let asked = asked.map_or("something", |asked| asked.title.as_str());
                ("An agent asks for a permission", format!("{agent} on {machine} asks to run: {asked}"))
            }
        },
        AgentState::AwaitingInput => {
            ("An agent wrote a plan to approve", format!("{agent} on {machine} wrote a plan for you to approve"))
        }
        AgentState::Error => ("An agent stopped with an error", format!("{agent} on {machine} stopped with an error")),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(state: AgentState) -> crate::protocol::AgentStatus {
        crate::protocol::AgentStatus {
            agent: "Claude Code".to_owned(),
            agent_id: "claude".to_owned(),
            state,
            streaming: String::new(),
            thinking: String::new(),
            permissions: Vec::new(),
            questions: Vec::new(),
            sign_in: None,
            context: None,
            error: None,
            modes: None,
            settings: Vec::new(),
            takes_images: false,
            takes_files: false,
            commands: Vec::new(),
            suggestion: None,
            stoppable: Vec::new(),
            sandbox: None,
            auto_check: None,
            checking: Vec::new(),
            escalations: Vec::new(),
            steers: false,
            waiting_steers: Vec::new(),
        }
    }

    #[test]
    fn phones_hear_of_what_needs_the_person() {
        let (title, body) = notice("workstation", &status(AgentState::AwaitingInput)).unwrap();
        assert_eq!(title, "An agent wrote a plan to approve");
        assert_eq!(body, "Claude Code on workstation wrote a plan for you to approve");
        assert!(notice("workstation", &status(AgentState::Error)).is_some());
        assert!(notice("workstation", &status(AgentState::AwaitingPermission)).unwrap().1.contains("asks to run"));
        let escalated = crate::protocol::AgentStatus {
            escalations: vec![crate::protocol::EscalationAsked {
                id: 1,
                call: "git push --force".into(),
                refused: "Rewrites history.".into(),
                why: "A rebase.".into(),
            }],
            ..status(AgentState::AwaitingPermission)
        };
        assert_eq!(
            notice("workstation", &escalated).unwrap().1,
            "Claude Code on workstation asks you to allow: git push --force"
        );
        for quiet in [AgentState::Working, AgentState::Idle, AgentState::Starting, AgentState::Stopped] {
            assert!(notice("workstation", &status(quiet)).is_none(), "{quiet:?}");
        }
    }

    #[tokio::test]
    async fn a_taken_port_moves_to_the_next_free_pair_unless_phones_know_it() {
        // Another person's Silt on the same computer holds a port.
        let held = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
        let port = held.local_addr().unwrap().port();
        assert!(bind_pair(port, false).await.is_err(), "a port phones know is never swapped for another");
        let (_secure, _plain, found) = bind_pair(port, true).await.unwrap();
        assert!(found > port && (found - port) % 2 == 0, "{found} after {port}");
    }

    #[test]
    fn the_name_stays_across_restarts_and_starting_over_makes_a_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let lan = Lan::new(dir.path(), broadcast::channel(4).0);
        let host = lan.shared.names.read().unwrap().host.clone();
        assert!(host.starts_with("silt-") && host.ends_with(".local"), "{host}");
        let config = Config { on: true, host: Some(host.clone()), port: Some(7421) };
        config.save(&dir.path().join("lan")).unwrap();
        let again = Lan::new(dir.path(), broadcast::channel(4).0);
        assert_eq!(again.shared.names.read().unwrap().host, host);
        assert!(again.wanted());
        let other = Lan::new(tempfile::tempdir().unwrap().path(), broadcast::channel(4).0);
        assert_ne!(other.shared.names.read().unwrap().host, host, "two machines of one name tell apart");
    }

    #[tokio::test]
    async fn starting_over_makes_a_new_name() {
        let dir = tempfile::tempdir().unwrap();
        let mut lan = Lan::new(dir.path(), broadcast::channel(4).0);
        let before = lan.shared.names.read().unwrap().host.clone();
        lan.forget(axum::Router::new).await.unwrap();
        assert_ne!(lan.shared.names.read().unwrap().host, before);
        assert_eq!(Config::load(&dir.path().join("lan")).host.as_deref(), Some(lan.shared.names.read().unwrap().host.as_str()));
    }

    #[test]
    fn a_push_goes_once_per_change_only_while_listening() {
        let dir = tempfile::tempdir().unwrap();
        let mut lan = Lan::new(dir.path(), broadcast::channel(4).0);
        let thread = ThreadId(7);
        lan.agent_changed(thread, Some(&status(AgentState::AwaitingInput)));
        assert_eq!(lan.agents.get(&thread), Some(&AgentState::AwaitingInput), "noted even while off");
        lan.agent_changed(thread, None);
        assert!(lan.agents.is_empty());
    }
}
