//! What an agent's sandbox may reach beyond its own files (ADR 0093): its
//! network, devices, and what of the machine the person hands it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Policy;

/// What the sandbox can connect to, from least to most.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Network {
    /// Nothing leaves the sandbox.
    None,
    /// HTTP(S) through Silt's gateway only.
    Gateway,
    /// Any public host; never this machine, its networks or cloud metadata.
    #[default]
    Full,
    /// The machine's own network, its loopback included.
    Host,
}

impl Network {
    pub const ALL: [Self; 4] = [Self::None, Self::Gateway, Self::Full, Self::Host];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Gateway => "gateway",
            Self::Full => "full",
            Self::Host => "host",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|network| network.as_str() == name)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "No network",
            Self::Gateway => "Through Silt only",
            Self::Full => "Full internet",
            Self::Host => "This machine's network",
        }
    }
}

impl fmt::Display for Network {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// What makes Silt's sandbox. OCI containers and VMs follow (ADR 0093).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Backend {
    /// A process sandbox, on Linux.
    Bubblewrap,
    /// macOS's own sandbox, through `sandbox-exec`.
    Seatbelt,
}

/// The one this system has: Seatbelt on macOS, bubblewrap elsewhere.
impl Default for Backend {
    fn default() -> Self {
        if cfg!(target_os = "macos") { Self::Seatbelt } else { Self::Bubblewrap }
    }
}

impl Backend {
    pub const ALL: [Self; 2] = [Self::Bubblewrap, Self::Seatbelt];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Bubblewrap => "bubblewrap",
            Self::Seatbelt => "seatbelt",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|backend| backend.as_str() == name)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Bubblewrap => "bubblewrap",
            Self::Seatbelt => "Seatbelt",
        }
    }
}

/// Which devices the sandbox gets, from least to most.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Devices {
    #[default]
    None,
    /// The named devices, held by Silt and handed to the sandbox as a
    /// session that outlives the agent: a serial port as a terminal, an
    /// adb device through a filter that reaches it alone.
    Relayed,
    /// The named devices' nodes.
    Named,
    /// Every USB device, for tools that talk to them directly (libusb).
    AllUsb,
}

impl Devices {
    pub const ALL: [Self; 4] = [Self::None, Self::Relayed, Self::Named, Self::AllUsb];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Relayed => "relayed",
            Self::Named => "named",
            Self::AllUsb => "all_usb",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|devices| devices.as_str() == name)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Relayed => "Relayed by Silt",
            Self::Named => "Named devices",
            Self::AllUsb => "All USB",
        }
    }
}

/// Something of the machine a person may hand the sandbox, which a rule can
/// refuse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case")]
pub enum Extra {
    /// Address ranges past those always blocked, such as a VPN's.
    Ranges,
    /// Ports on this machine.
    Ports,
    /// Folders beyond the checkout and the toolchains.
    Paths,
    Gpu,
    /// The Docker socket: root on this machine.
    Docker,
    /// The SSH agent's socket: signs as the person, anywhere.
    SshAgent,
}

impl Extra {
    pub const ALL: [Self; 6] = [Self::Ranges, Self::Ports, Self::Paths, Self::Gpu, Self::Docker, Self::SshAgent];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ranges => "ranges",
            Self::Ports => "ports",
            Self::Paths => "paths",
            Self::Gpu => "gpu",
            Self::Docker => "docker",
            Self::SshAgent => "ssh_agent",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|extra| extra.as_str() == name)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Ranges => "Address ranges",
            Self::Ports => "Ports on this machine",
            Self::Paths => "More folders",
            Self::Gpu => "The GPU",
            Self::Docker => "Docker",
            Self::SshAgent => "ssh-agent",
        }
    }

    /// What it hands the agent, said where it's turned on.
    pub fn hands_over(self) -> &'static str {
        match self {
            Self::Ranges => "hosts on those networks, a VPN's included",
            Self::Ports => "whatever listens on those ports",
            Self::Paths => "those folders' files",
            Self::Gpu => "the GPU's devices",
            Self::Docker => "root on this machine: the agent could do anything you can",
            Self::SshAgent => "signing as you, with every key loaded, to any host",
        }
    }
}

/// What a sandbox reaches, worked out from the rules in effect.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
pub struct Reach {
    /// What makes the sandbox.
    pub backend: Backend,
    pub network: Network,
    /// The hosts "through Silt only" reaches; any public host when empty.
    pub hosts: Vec<String>,
    pub devices: Devices,
    /// The devices, for [`Devices::Relayed`] and [`Devices::Named`]: device
    /// nodes, and for a relay `adb:SERIAL` too.
    pub device_nodes: Vec<String>,
    /// Address ranges allowed past those always blocked.
    pub ranges: Vec<String>,
    /// Ports on this machine the sandbox reaches as its own `localhost`.
    pub ports: Vec<u16>,
    /// Extra folders, and whether each is writable.
    pub paths: BTreeMap<String, bool>,
    pub gpu: bool,
    pub docker: bool,
    pub ssh_agent: bool,
    /// The agent's own sandbox may run inside Silt's.
    pub nested: bool,
}

impl Reach {
    /// The reach `policy` gives, `policy` being the rules in effect: what
    /// the person turned on, less what a rule refuses.
    pub fn of(policy: &Policy) -> Self {
        let allowed = |extra: Extra| !policy.refuse.contains(&extra);
        let devices = policy.devices.unwrap_or_default();
        Self {
            backend: policy.backend.unwrap_or_default(),
            network: policy.network.unwrap_or_default(),
            hosts: policy.hosts.iter().flatten().cloned().collect(),
            devices,
            device_nodes: if matches!(devices, Devices::Named | Devices::Relayed) {
                policy.device_nodes.iter().cloned().collect()
            } else {
                Vec::new()
            },
            ranges: if allowed(Extra::Ranges) { policy.ranges.iter().cloned().collect() } else { Vec::new() },
            ports: if allowed(Extra::Ports) { policy.ports.iter().copied().collect() } else { Vec::new() },
            paths: if allowed(Extra::Paths) { policy.paths.clone() } else { BTreeMap::new() },
            gpu: policy.gpu && allowed(Extra::Gpu),
            docker: policy.docker && allowed(Extra::Docker),
            ssh_agent: policy.ssh_agent && allowed(Extra::SshAgent),
            nested: !policy.no_nested,
        }
    }

    /// One line on what the run reaches, for a terminal's banner.
    pub fn summary(&self) -> String {
        let mut parts = vec![self.network.label().to_lowercase()];
        if !self.ranges.is_empty() {
            parts.push(format!("also {}", self.ranges.join(", ")));
        }
        match self.devices {
            Devices::None => {}
            Devices::Named | Devices::Relayed if self.device_nodes.is_empty() => {}
            Devices::Named => parts.push(format!("devices: {}", self.device_nodes.join(", "))),
            Devices::Relayed => parts.push(format!("relayed: {}", self.device_nodes.join(", "))),
            Devices::AllUsb => parts.push("all USB devices".into()),
        }
        for (on, what) in [(self.gpu, "the GPU"), (self.docker, "Docker"), (self.ssh_agent, "ssh-agent")] {
            if on {
                parts.push(what.into());
            }
        }
        if !self.ports.is_empty() {
            let ports: Vec<String> = self.ports.iter().map(u16::to_string).collect();
            parts.push(format!("ports {}", ports.join(", ")));
        }
        parts.join(" · ")
    }
}

/// The lower of two caps, either unset meaning none.
pub(crate) fn lower<T: Ord + Copy>(one: Option<T>, other: Option<T>) -> Option<T> {
    match (one, other) {
        (Some(one), Some(other)) => Some(one.min(other)),
        (one, other) => one.or(other),
    }
}

/// Both lists' entries that both allow, either unset meaning any.
pub(crate) fn both<T: Ord + Clone>(one: &BTreeSet<T>, other: &BTreeSet<T>) -> BTreeSet<T> {
    one.union(other).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_internet_is_the_default_and_refusals_win() {
        let reach = Reach::of(&Policy::default());
        assert_eq!(reach.network, Network::Full);
        assert_eq!(reach.devices, Devices::None);
        assert!(reach.nested && !reach.docker);

        let mine = Policy {
            network: Some(Network::Host),
            docker: true,
            ssh_agent: true,
            ranges: BTreeSet::from(["10.20.0.0/16".to_owned()]),
            ..Policy::default()
        };
        let org = Policy {
            network: Some(Network::Full),
            refuse: BTreeSet::from([Extra::Docker, Extra::SshAgent]),
            no_nested: true,
            ..Policy::default()
        };
        let reach = Reach::of(&mine.stricter(&org));
        assert_eq!(reach.network, Network::Full, "the org caps it");
        assert!(!reach.docker && !reach.ssh_agent && !reach.nested);
        assert_eq!(reach.ranges, ["10.20.0.0/16"]);
        assert_eq!(reach.summary(), "full internet · also 10.20.0.0/16");
    }

    #[test]
    fn named_devices_only_with_that_mode() {
        let policy = Policy { device_nodes: BTreeSet::from(["/dev/ttyUSB0".to_owned()]), ..Policy::default() };
        assert!(Reach::of(&policy).device_nodes.is_empty());
        let policy = Policy { devices: Some(Devices::Named), ..policy };
        assert_eq!(Reach::of(&policy).device_nodes, ["/dev/ttyUSB0"]);
        let capped = policy.stricter(&Policy { devices: Some(Devices::None), ..Policy::default() });
        assert_eq!(Reach::of(&capped).devices, Devices::None);
    }

    #[test]
    fn names_read_back() {
        for network in Network::ALL {
            assert_eq!(Network::parse(network.as_str()), Some(network));
        }
        for devices in Devices::ALL {
            assert_eq!(Devices::parse(devices.as_str()), Some(devices));
        }
        for extra in Extra::ALL {
            assert_eq!(Extra::parse(extra.as_str()), Some(extra));
        }
    }
}
