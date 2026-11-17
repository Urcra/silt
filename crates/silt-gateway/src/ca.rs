//! The certificate authority the gateway signs its hosts' certificates
//! with, when it reads a request to add a credential. Its key stays on this
//! machine, outside every sandbox; sandboxes trust its certificate only.

use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair, KeyUsagePurpose};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

pub struct Ca {
    certificate: CertificateDer<'static>,
    pem: String,
    issuer: Issuer<'static, KeyPair>,
    leaves: Mutex<HashMap<String, Arc<rustls::sign::CertifiedKey>>>,
}

fn invalid(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

/// The authority's own certificate's fields, the same each time it's made
/// from its key, so certificates signed after a restart still chain to it.
fn authority() -> Result<CertificateParams, rcgen::Error> {
    let mut params = CertificateParams::new(Vec::<String>::new())?;
    params.distinguished_name.push(DnType::CommonName, "Silt sandbox gateway");
    params.distinguished_name.push(DnType::OrganizationName, "Silt, on this machine");
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign, KeyUsagePurpose::DigitalSignature];
    Ok(params)
}

impl Ca {
    /// The authority kept in `dir`, made the first time.
    pub fn load_or_create(dir: &Path) -> io::Result<Self> {
        silt_os::create_private_dir(dir)?;
        let (key_path, cert_path) = (dir.join("ca-key.pem"), dir.join("ca.pem"));
        let key = match std::fs::read_to_string(&key_path) {
            Ok(pem) => KeyPair::from_pem(&pem).map_err(invalid)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let key = KeyPair::generate().map_err(invalid)?;
                silt_os::write_private(&key_path, key.serialize_pem().as_bytes())?;
                let _ = std::fs::remove_file(&cert_path);
                key
            }
            Err(error) => return Err(error),
        };
        let params = authority().map_err(invalid)?;
        let pem = match std::fs::read_to_string(&cert_path) {
            Ok(pem) => pem,
            Err(_) => {
                let pem = params.self_signed(&key).map_err(invalid)?.pem();
                std::fs::write(&cert_path, &pem)?;
                pem
            }
        };
        let certificate = rustls::pki_types::pem::PemObject::from_pem_slice(pem.as_bytes()).map_err(invalid)?;
        Ok(Self { certificate, pem, issuer: Issuer::new(params, key), leaves: Mutex::new(HashMap::new()) })
    }

    /// Its certificate, which a sandbox adds to what it trusts.
    pub fn pem(&self) -> &str {
        &self.pem
    }

    /// A certificate for `host`, signed by the authority, made once per run.
    pub fn leaf(&self, host: &str) -> io::Result<Arc<rustls::sign::CertifiedKey>> {
        let mut leaves = self.leaves.lock().map_err(|_| io::Error::other("the certificates' lock was poisoned"))?;
        if let Some(leaf) = leaves.get(host) {
            return Ok(leaf.clone());
        }
        let mut params = CertificateParams::new(vec![host.to_owned()]).map_err(invalid)?;
        params.distinguished_name.push(DnType::CommonName, host);
        let key = KeyPair::generate().map_err(invalid)?;
        let certificate = params.signed_by(&key, &self.issuer).map_err(invalid)?;
        let private = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
        let signing = rustls::crypto::ring::default_provider().key_provider.load_private_key(private).map_err(invalid)?;
        let leaf = Arc::new(rustls::sign::CertifiedKey::new(vec![certificate.der().clone(), self.certificate.clone()], signing));
        leaves.insert(host.to_owned(), leaf.clone());
        Ok(leaf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_authority_is_kept_and_signs_hosts() {
        let dir = tempfile::tempdir().unwrap();
        let ca = Ca::load_or_create(dir.path()).unwrap();
        let again = Ca::load_or_create(dir.path()).unwrap();
        assert_eq!(ca.pem(), again.pem(), "the same authority across restarts");
        let leaf = again.leaf("api.github.com").unwrap();
        assert_eq!(leaf.cert.len(), 2);
        assert!(Arc::ptr_eq(&leaf, &again.leaf("api.github.com").unwrap()));
    }
}
