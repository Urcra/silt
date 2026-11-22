//! Pushes from this machine to the phones paired over the network (ADR
//! 0112), encrypted for each browser as RFC 8291 says, signed with a VAPID
//! key of the machine's own, as the server signs its (ADR 0028).

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Serialize;
use silt_types::ThreadId;
use tracing::warn;
use web_push_native::jwt_simple::algorithms::ES256KeyPair;
use web_push_native::jwt_simple::prelude::ECDSAP256PublicKeyLike as _;
use web_push_native::p256::PublicKey;
use web_push_native::{Auth, WebPushBuilder};

use super::PushSubscription;

const KEY_FILE: &str = "vapid.key";
/// How long a push service keeps a push for a phone that's away.
const PUSH_TTL: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);
/// Who sends the pushes, as push services ask a sender to say.
const CONTACT: &str = "https://github.com/Urcra/silt";

/// What a push carries, for the service worker to show.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct Payload {
    title: String,
    body: String,
    url: String,
    tag: String,
}

impl Payload {
    pub(crate) fn new(title: &str, body: &str, thread: ThreadId) -> Self {
        Self { title: title.to_owned(), body: body.to_owned(), url: format!("t/{thread}"), tag: format!("silt-{thread}") }
    }
}

/// Signs and sends pushes; the key is made the first time it's needed.
pub(crate) struct Pusher {
    dir: PathBuf,
    key: OnceLock<Option<Arc<ES256KeyPair>>>,
    http: reqwest::Client,
}

impl Pusher {
    pub(crate) fn new(dir: &Path) -> Self {
        Self { dir: dir.to_owned(), key: OnceLock::new(), http: reqwest::Client::new() }
    }

    fn key(&self) -> Option<&Arc<ES256KeyPair>> {
        self.key
            .get_or_init(|| {
                load_or_make_key(&self.dir)
                    .map(Arc::new)
                    .inspect_err(|error| warn!(%error, "could not make the key pushes are signed with"))
                    .ok()
            })
            .as_ref()
    }

    /// The key phones subscribe with, as `applicationServerKey` takes it.
    pub(crate) fn public_key(&self) -> Option<String> {
        Some(URL_SAFE_NO_PAD.encode(self.key()?.public_key().public_key().to_bytes_uncompressed()))
    }

    /// Pushes `payload` to each of `targets`, in the background.
    pub(crate) fn send(&self, targets: Vec<PushSubscription>, payload: Payload) {
        let Some(key) = self.key().cloned() else { return };
        let Ok(payload) = serde_json::to_vec(&payload) else { return };
        let http = self.http.clone();
        tokio::spawn(async move {
            for target in targets {
                let Some(builder) = builder(&target) else { continue };
                let request = match builder.with_vapid(&key, CONTACT).build(payload.clone()) {
                    Ok(request) => request,
                    Err(error) => {
                        warn!(%error, "could not encrypt a push");
                        continue;
                    }
                };
                let (parts, body) = request.into_parts();
                let mut sent = http.post(parts.uri.to_string()).body(body);
                for (name, value) in &parts.headers {
                    sent = sent.header(name.as_str(), value.as_bytes());
                }
                match sent.send().await {
                    Ok(response) if !response.status().is_success() => {
                        warn!(status = %response.status(), "a push service refused a push");
                    }
                    Ok(_) => {}
                    Err(error) => warn!(%error, "could not reach a push service"),
                }
            }
        });
    }
}

fn load_or_make_key(dir: &Path) -> std::io::Result<ES256KeyPair> {
    let path = dir.join(KEY_FILE);
    if let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(bytes) = URL_SAFE_NO_PAD.decode(text.trim())
        && let Ok(key) = ES256KeyPair::from_bytes(&bytes)
    {
        return Ok(key);
    }
    let key = ES256KeyPair::generate();
    silt_os::create_private_dir(dir)?;
    silt_os::write_private(&path, URL_SAFE_NO_PAD.encode(key.to_bytes()).as_bytes())?;
    Ok(key)
}

/// Where and how to reach one phone's browser.
fn builder(target: &PushSubscription) -> Option<WebPushBuilder> {
    let public = PublicKey::from_sec1_bytes(&URL_SAFE_NO_PAD.decode(&target.p256dh).ok()?).ok()?;
    let auth: [u8; 16] = URL_SAFE_NO_PAD.decode(&target.auth).ok()?.try_into().ok()?;
    Some(WebPushBuilder::new(target.endpoint.parse().ok()?, public, Auth::from(auth)).with_valid_duration(PUSH_TTL))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let first = Pusher::new(dir.path()).public_key().unwrap();
        assert_eq!(Pusher::new(dir.path()).public_key().unwrap(), first);
        assert_eq!(URL_SAFE_NO_PAD.decode(&first).unwrap().len(), 65, "an uncompressed P-256 point");
    }

    #[test]
    fn a_subscription_that_isnt_one_is_skipped() {
        let bad = PushSubscription { endpoint: "https://push.example/x".into(), p256dh: "nope".into(), auth: "nope".into() };
        assert!(builder(&bad).is_none());
    }
}
