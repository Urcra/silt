//! This machine's own certificate authority for phones on its network
//! (ADR 0112), and the certificate its LAN listener shows, signed by it.
//! The authority may vouch only for this machine's own `.local` name and
//! private addresses, so a phone that trusts it trusts nothing on the
//! internet with it, nor anything else on the network.

use std::io;
use std::net::IpAddr;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime};

use rcgen::{
    BasicConstraints, CertificateParams, CidrSubnet, DnType, ExtendedKeyUsagePurpose, GeneralSubtree, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, NameConstraints, SanType,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use sha2::{Digest as _, Sha256};
use time::OffsetDateTime;

const KEY_FILE: &str = "ca-key.pem";
const CERT_FILE: &str = "ca.pem";
/// How long the authority lasts: phones keep trusting it, so long.
const AUTHORITY_LIFE: Duration = Duration::from_secs(10 * 365 * 24 * 60 * 60);
/// How long a listener's certificate lasts; it's made again well before.
pub(crate) const LEAF_LIFE: Duration = Duration::from_secs(90 * 24 * 60 * 60);
/// How long before it ends a certificate is made again.
const RENEW_BEFORE: Duration = Duration::from_secs(30 * 24 * 60 * 60);

fn invalid(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

/// The names a certificate may carry: `host` alone, and the private
/// address ranges, IPv4's and IPv6's unique local and link-local ones, as
/// addresses change with the network.
fn constraints(host: &str) -> NameConstraints {
    NameConstraints {
        permitted_subtrees: vec![
            GeneralSubtree::DnsName(host.to_owned()),
            GeneralSubtree::IpAddress(CidrSubnet::from_v4_prefix([10, 0, 0, 0], 8)),
            GeneralSubtree::IpAddress(CidrSubnet::from_v4_prefix([172, 16, 0, 0], 12)),
            GeneralSubtree::IpAddress(CidrSubnet::from_v4_prefix([192, 168, 0, 0], 16)),
            GeneralSubtree::IpAddress(CidrSubnet::from_v6_prefix([0xfc, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], 7)),
            GeneralSubtree::IpAddress(CidrSubnet::from_v6_prefix([0xfe, 0x80, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], 10)),
        ],
        excluded_subtrees: Vec::new(),
    }
}

/// Whether phones on the network can reach `address`: a private one.
pub(crate) fn is_private(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => v4.is_private(),
        IpAddr::V6(v6) => (v6.segments()[0] & 0xfe00) == 0xfc00,
    }
}

/// `silt-<host>-<suffix>.local`: a name of Silt's own, so it clashes neither
/// with whatever answers for the host's name (avahi, Bonjour, Windows) nor
/// with another machine, or another person's Silt, of the same name.
pub(crate) fn local_name(host: &str, suffix: &str) -> String {
    let label: String = host
        .split('.')
        .next()
        .unwrap_or_default()
        .chars()
        .map(|char| if char.is_ascii_alphanumeric() { char.to_ascii_lowercase() } else { '-' })
        .collect();
    let label = label.trim_matches('-');
    let label = if label.is_empty() { "machine" } else { label };
    // A DNS label is at most 63 bytes, `silt-` and the suffix included.
    let room = 63 - "silt-".len() - 1 - suffix.len();
    format!("silt-{}-{suffix}.local", label[..label.len().min(room)].trim_end_matches('-'))
}

/// Four hex digits, made once per name, that tell two machines apart.
pub(crate) fn new_suffix() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..4].to_owned()
}

/// Where the listener is reached: its name and its private addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Names {
    pub(crate) host: String,
    pub(crate) addresses: Vec<IpAddr>,
}

/// The authority, kept in the LAN folder of the data dir.
pub(crate) struct Authority {
    der: CertificateDer<'static>,
    issuer: Issuer<'static, KeyPair>,
}

impl Authority {
    /// The authority kept in `dir`, made the first time for `host`, as
    /// `label` (`Silt on workstation (ada)`).
    pub(crate) fn load_or_create(dir: &Path, host: &str, label: &str, now: SystemTime) -> io::Result<Self> {
        silt_os::create_private_dir(dir)?;
        let (key_path, cert_path) = (dir.join(KEY_FILE), dir.join(CERT_FILE));
        let (key, pem) = match (std::fs::read_to_string(&key_path), std::fs::read_to_string(&cert_path)) {
            (Ok(key), Ok(pem)) => (KeyPair::from_pem(&key).map_err(invalid)?, pem),
            _ => {
                let key = KeyPair::generate().map_err(invalid)?;
                let pem = authority_params(host, label, now)?.self_signed(&key).map_err(invalid)?.pem();
                silt_os::write_private(&key_path, key.serialize_pem().as_bytes())?;
                std::fs::write(&cert_path, &pem)?;
                (key, pem)
            }
        };
        let der = rustls::pki_types::pem::PemObject::from_pem_slice(pem.as_bytes()).map_err(invalid)?;
        let issuer = Issuer::from_ca_cert_pem(&pem, key).map_err(invalid)?;
        Ok(Self { der, issuer })
    }

    /// Removes the authority kept in `dir`: every phone needs a new one.
    pub(crate) fn forget(dir: &Path) -> io::Result<()> {
        for file in [KEY_FILE, CERT_FILE] {
            match std::fs::remove_file(dir.join(file)) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
                _ => {}
            }
        }
        Ok(())
    }

    pub(crate) fn der(&self) -> &[u8] {
        self.der.as_ref()
    }

    /// Its SHA-256, as a phone shows it when it installs the authority.
    pub(crate) fn fingerprint(&self) -> String {
        fingerprint(self.der.as_ref())
    }

    /// A certificate for `names`, lasting [`LEAF_LIFE`] from `now`.
    pub(crate) fn leaf(&self, names: &Names, now: SystemTime) -> io::Result<Leaf> {
        let mut params = CertificateParams::default();
        params.distinguished_name.push(DnType::CommonName, names.host.as_str());
        let mut alt_names = vec![SanType::DnsName(names.host.as_str().try_into().map_err(invalid)?)];
        alt_names.extend(names.addresses.iter().copied().map(SanType::IpAddress));
        params.subject_alt_names = alt_names;
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.not_before = at(now - Duration::from_secs(60 * 60))?;
        params.not_after = at(now + LEAF_LIFE)?;
        let key = KeyPair::generate().map_err(invalid)?;
        let certificate = params.signed_by(&key, &self.issuer).map_err(invalid)?;
        let private = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
        let signing = rustls::crypto::ring::default_provider().key_provider.load_private_key(private).map_err(invalid)?;
        Ok(Leaf {
            key: Arc::new(CertifiedKey::new(vec![certificate.der().clone(), self.der.clone()], signing)),
            names: names.clone(),
            ends: now + LEAF_LIFE,
        })
    }
}

fn authority_params(host: &str, label: &str, now: SystemTime) -> io::Result<CertificateParams> {
    let mut params = CertificateParams::default();
    params.distinguished_name.push(DnType::CommonName, label);
    params.distinguished_name.push(DnType::OrganizationName, "Silt, on this machine");
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign, KeyUsagePurpose::DigitalSignature];
    params.name_constraints = Some(constraints(host));
    params.not_before = at(now - Duration::from_secs(60 * 60))?;
    params.not_after = at(now + AUTHORITY_LIFE)?;
    Ok(params)
}

fn at(time: SystemTime) -> io::Result<OffsetDateTime> {
    let seconds = time.duration_since(SystemTime::UNIX_EPOCH).map_err(invalid)?.as_secs();
    OffsetDateTime::from_unix_timestamp(i64::try_from(seconds).map_err(invalid)?).map_err(invalid)
}

/// SHA-256 of `der`, in pairs of hex digits, as iOS and Android show it.
pub(crate) fn fingerprint(der: &[u8]) -> String {
    Sha256::digest(der).iter().map(|byte| format!("{byte:02X}")).collect::<Vec<_>>().join(" ")
}

/// The listener's certificate, with what it was made for.
#[derive(Clone)]
pub(crate) struct Leaf {
    pub(crate) key: Arc<CertifiedKey>,
    pub(crate) names: Names,
    pub(crate) ends: SystemTime,
}

impl Leaf {
    /// When it's made again, unless names change first.
    pub(crate) fn renews(&self) -> SystemTime {
        self.ends - RENEW_BEFORE
    }

    /// Whether it's time for a new one: names changed, or the end is near.
    pub(crate) fn stale(&self, names: &Names, now: SystemTime) -> bool {
        self.names != *names || now + RENEW_BEFORE >= self.ends
    }
}

/// Hands every connection the current certificate, which a renewal swaps.
#[derive(Debug)]
pub(crate) struct Current(RwLock<Arc<CertifiedKey>>);

impl Current {
    pub(crate) fn new(key: Arc<CertifiedKey>) -> Self {
        Self(RwLock::new(key))
    }

    pub(crate) fn replace(&self, key: Arc<CertifiedKey>) {
        *self.0.write().expect("lock") = key;
    }
}

impl ResolvesServerCert for Current {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.0.read().expect("lock").clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::client::danger::ServerCertVerifier as _;
    use rustls::pki_types::{ServerName, UnixTime};

    const HOST: &str = "silt-workstation-3fa9.local";

    fn names() -> Names {
        Names { host: HOST.into(), addresses: vec!["192.168.1.23".parse().unwrap()] }
    }

    fn made(dir: &Path, now: SystemTime) -> Authority {
        Authority::load_or_create(dir, HOST, "Silt on workstation (ada)", now).unwrap()
    }

    fn verifies(authority: &Authority, leaf: &CertifiedKey, name: &str) -> bool {
        let mut roots = rustls::RootCertStore::empty();
        roots.add(CertificateDer::from(authority.der().to_vec())).unwrap();
        let verifier = rustls::client::WebPkiServerVerifier::builder_with_provider(
            Arc::new(roots),
            Arc::new(rustls::crypto::ring::default_provider()),
        )
        .build()
        .unwrap();
        verifier
            .verify_server_cert(
                &leaf.cert[0],
                &leaf.cert[1..],
                &ServerName::try_from(name.to_owned()).unwrap(),
                &[],
                UnixTime::now(),
            )
            .is_ok()
    }

    #[test]
    fn the_authority_is_kept_and_vouches_for_the_listener_by_name_and_address() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let authority = made(dir.path(), now);
        let again = made(dir.path(), now);
        assert_eq!(
            authority.fingerprint(),
            again.fingerprint(),
            "the same authority across restarts, so phones keep trusting it"
        );
        let leaf = again.leaf(&names(), now).unwrap();
        assert!(verifies(&authority, &leaf.key, HOST));
        assert!(verifies(&authority, &leaf.key, "192.168.1.23"));
        assert!(!verifies(&authority, &leaf.key, "silt-other-0000.local"), "only the names it was made for");
    }

    #[test]
    fn the_authority_can_vouch_for_nothing_on_the_internet() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let authority = made(dir.path(), now);
        // Another machine's name on the network, a printer's, is as far out of reach as the internet.
        assert!(!verifies(
            &authority,
            &authority.leaf(&Names { host: "printer.local".into(), addresses: Vec::new() }, now).unwrap().key,
            "printer.local"
        ));
        for (host, address) in [("example.com", "93.184.215.14"), ("bank.example", "8.8.8.8")] {
            let leaf = authority.leaf(&Names { host: host.into(), addresses: vec![address.parse().unwrap()] }, now).unwrap();
            assert!(!verifies(&authority, &leaf.key, host), "{host} is outside the name constraints");
            assert!(!verifies(&authority, &leaf.key, address), "{address} is outside the name constraints");
        }
    }

    #[test]
    fn forgetting_the_authority_makes_a_new_one() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let first = made(dir.path(), now).fingerprint();
        Authority::forget(dir.path()).unwrap();
        assert_ne!(made(dir.path(), now).fingerprint(), first);
    }

    #[test]
    fn a_certificate_is_made_again_when_names_change_or_its_end_nears() {
        let dir = tempfile::tempdir().unwrap();
        let now = SystemTime::now();
        let leaf = made(dir.path(), now).leaf(&names(), now).unwrap();
        assert!(!leaf.stale(&names(), now));
        let moved = Names { addresses: vec!["192.168.1.40".parse().unwrap()], ..names() };
        assert!(leaf.stale(&moved, now));
        assert!(leaf.stale(&names(), now + LEAF_LIFE - Duration::from_secs(29 * 24 * 60 * 60)));
    }

    #[test]
    fn names_and_addresses() {
        assert_eq!(local_name("Workstation", "3fa9"), "silt-workstation-3fa9.local");
        assert_eq!(local_name("ada's mac.lan", "3fa9"), "silt-ada-s-mac-3fa9.local");
        assert_eq!(local_name("", "3fa9"), "silt-machine-3fa9.local");
        let long = local_name(&"x".repeat(100), "3fa9");
        assert_eq!(long.trim_end_matches(".local").len(), 63, "a DNS label at most");
        assert_ne!(new_suffix(), new_suffix());
        assert!(is_private("192.168.1.2".parse().unwrap()));
        assert!(is_private("10.1.2.3".parse().unwrap()));
        assert!(is_private("fd12::1".parse().unwrap()));
        assert!(!is_private("8.8.8.8".parse().unwrap()));
        assert!(!is_private("127.0.0.1".parse().unwrap()));
        assert!(!is_private("2001:db8::1".parse().unwrap()));
    }

    #[test]
    fn fingerprints_read_as_phones_show_them() {
        assert_eq!(fingerprint(b"").len(), 32 * 3 - 1);
        assert!(fingerprint(b"").starts_with("E3 B0 C4 42"));
    }
}
