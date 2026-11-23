//! Advertising the listener over mDNS (ADR 0112): `silt-<host>.local`,
//! which phones open, and a `_silt._tcp` service naming it. Failing to is
//! no reason to stop: phones can still use an address.

use mdns_sd::{ServiceDaemon, ServiceInfo};
use tracing::warn;

const SERVICE: &str = "_silt._tcp.local.";

pub(crate) struct Advertised {
    daemon: ServiceDaemon,
    info: ServiceInfo,
}

/// Starts answering for `host` and advertising the service on `port`, as
/// `label` (`Silt on workstation (ada)`).
pub(crate) fn advertise(label: &str, host: &str, port: u16) -> Option<Advertised> {
    let made = ServiceDaemon::new().and_then(|daemon| {
        let info = ServiceInfo::new(SERVICE, label, &format!("{host}."), "", port, &[("path", "/")][..])?.enable_addr_auto();
        daemon.register(info.clone())?;
        Ok(Advertised { daemon, info })
    });
    made.inspect_err(|error| warn!(%error, "could not advertise this machine on the network; phones can use its address")).ok()
}

impl Advertised {
    /// Announces again, after the addresses changed.
    pub(crate) fn readdress(&self) {
        if let Err(error) = self.daemon.register(self.info.clone()) {
            warn!(%error, "could not announce this machine's new addresses");
        }
    }
}

impl Drop for Advertised {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(self.info.get_fullname());
        let _ = self.daemon.shutdown();
    }
}
