//! What a call sends, declared when it's prepared and checked while it runs.
//! The host sees every effect a plugin causes, since each goes through a
//! capability; the ledger lets through only those that were approved.

use serde::{Deserialize, Serialize};

/// A channel whose traffic Silt can't check: the card says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "kebab-case")]
pub enum Channel {
    Sockets,
    Ssh,
    McpProcess,
}

impl Channel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Sockets => "TCP connections",
            Self::Ssh => "SSH commands",
            Self::McpProcess => "a local MCP server",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Effect {
    /// An HTTP request through the gateway. A body is bound by its hash.
    Http {
        method: String,
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body_sha256: Option<String>,
    },
    /// One commit pushed to one ref, only if the ref is still at `old`.
    GitPush { remote: String, reference: String, old: String, new: String },
    /// An agent's sandbox widened.
    Reach { path: String, writable: bool },
    /// Traffic on a channel Silt can't check, described for the card.
    Opaque { channel: Channel, what: String },
}

impl Effect {
    pub fn is_opaque(&self) -> bool {
        matches!(self, Self::Opaque { .. })
    }
}

/// What a plugin did while running, as its host saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observed {
    Http { method: String, url: String, body_sha256: Option<String> },
    GitPush { remote: String, reference: String, old: String, new: String },
    Reach { path: String, writable: bool },
    Opaque { channel: Channel },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Refused {
    #[error("{method} {url} wasn't among the requests you approved")]
    Http { method: String, url: String },
    #[error("pushing {new} to {reference} on {remote} wasn't what you approved")]
    GitPush { remote: String, reference: String, new: String },
    #[error("widening the sandbox to {path} wasn't what you approved")]
    Reach { path: String },
    #[error("the call used {}, which its card didn't say it would", channel.label())]
    Opaque { channel: Channel },
}

/// The approved effects of one call, used up as they happen.
#[derive(Debug, Clone)]
pub struct Ledger {
    left: Vec<Effect>,
    opaque: Vec<Channel>,
    /// A read's requests that only fetch need no declaring, beyond the
    /// hosts its capability grants: pagination can't be known beforehand.
    reads: bool,
}

const SAFE_METHODS: [&str; 3] = ["GET", "HEAD", "OPTIONS"];

impl Ledger {
    pub fn new(approved: &[Effect], reads: bool) -> Self {
        let opaque = approved
            .iter()
            .filter_map(|effect| match effect {
                Effect::Opaque { channel, .. } => Some(*channel),
                _ => None,
            })
            .collect();
        Self { left: approved.iter().filter(|effect| !effect.is_opaque()).cloned().collect(), opaque, reads }
    }

    /// Lets `observed` through if it was approved, using it up.
    pub fn check(&mut self, observed: &Observed) -> Result<(), Refused> {
        match observed {
            Observed::Opaque { channel } => {
                if self.opaque.contains(channel) {
                    Ok(())
                } else {
                    Err(Refused::Opaque { channel: *channel })
                }
            }
            Observed::Http { method, url, body_sha256 } => {
                let method = method.to_ascii_uppercase();
                let found = self.take(|effect| match effect {
                    Effect::Http { method: approved, url: approved_url, body_sha256: approved_body } => {
                        approved.eq_ignore_ascii_case(&method) && approved_url == url && approved_body == body_sha256
                    }
                    _ => false,
                });
                if found || (self.reads && SAFE_METHODS.contains(&method.as_str())) {
                    Ok(())
                } else {
                    Err(Refused::Http { method, url: url.clone() })
                }
            }
            Observed::GitPush { remote, reference, old, new } => {
                let found = self.take(|effect| {
                    matches!(effect, Effect::GitPush { remote: r, reference: f, old: o, new: n } if r == remote && f == reference && o == old && n == new)
                });
                if found {
                    Ok(())
                } else {
                    Err(Refused::GitPush { remote: remote.clone(), reference: reference.clone(), new: new.clone() })
                }
            }
            Observed::Reach { path, writable } => {
                let found =
                    self.take(|effect| matches!(effect, Effect::Reach { path: p, writable: w } if p == path && w == writable));
                if found { Ok(()) } else { Err(Refused::Reach { path: path.clone() }) }
            }
        }
    }

    fn take(&mut self, wanted: impl Fn(&Effect) -> bool) -> bool {
        match self.left.iter().position(wanted) {
            Some(index) => {
                self.left.remove(index);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(url: &str, body: &str) -> Effect {
        Effect::Http { method: "POST".into(), url: url.into(), body_sha256: Some(body.into()) }
    }

    #[test]
    fn only_the_approved_request_gets_through_and_only_once() {
        let mut ledger = Ledger::new(&[post("https://deploy.acme.internal/v1/rollouts", "4c1e")], false);
        let sent = Observed::Http {
            method: "post".into(),
            url: "https://deploy.acme.internal/v1/rollouts".into(),
            body_sha256: Some("4c1e".into()),
        };
        assert_eq!(ledger.check(&sent), Ok(()));
        assert!(ledger.check(&sent).is_err(), "used up");
        let other = Observed::Http {
            method: "POST".into(),
            url: "https://deploy.acme.internal/v1/rollouts".into(),
            body_sha256: Some("9999".into()),
        };
        assert!(
            Ledger::new(&[post("https://deploy.acme.internal/v1/rollouts", "4c1e")], false).check(&other).is_err(),
            "another body"
        );
        let get =
            Observed::Http { method: "GET".into(), url: "https://deploy.acme.internal/v1/status".into(), body_sha256: None };
        assert!(Ledger::new(&[], false).check(&get).is_err(), "a change declares even its reads");
        assert!(Ledger::new(&[], true).check(&get).is_ok(), "a read may fetch");
        let delete =
            Observed::Http { method: "DELETE".into(), url: "https://deploy.acme.internal/v1/x".into(), body_sha256: None };
        assert!(Ledger::new(&[], true).check(&delete).is_err(), "but never change");
    }

    #[test]
    fn a_push_must_be_the_approved_commit_from_the_approved_one() {
        let approved = Effect::GitPush {
            remote: "acme/api".into(),
            reference: "refs/heads/ada/x".into(),
            old: "9c1e".into(),
            new: "5b7d".into(),
        };
        let mut ledger = Ledger::new(std::slice::from_ref(&approved), false);
        let other = Observed::GitPush {
            remote: "acme/api".into(),
            reference: "refs/heads/ada/x".into(),
            old: "9c1e".into(),
            new: "dead".into(),
        };
        assert!(ledger.check(&other).is_err());
        let same = Observed::GitPush {
            remote: "acme/api".into(),
            reference: "refs/heads/ada/x".into(),
            old: "9c1e".into(),
            new: "5b7d".into(),
        };
        assert!(ledger.check(&same).is_ok());
    }

    #[test]
    fn opaque_channels_are_only_those_the_card_named() {
        let mut ledger = Ledger::new(&[Effect::Opaque { channel: Channel::Ssh, what: "uptime on car-12".into() }], false);
        assert!(ledger.check(&Observed::Opaque { channel: Channel::Ssh }).is_ok());
        assert!(ledger.check(&Observed::Opaque { channel: Channel::Ssh }).is_ok(), "a channel, not a single use");
        assert_eq!(
            ledger.check(&Observed::Opaque { channel: Channel::Sockets }),
            Err(Refused::Opaque { channel: Channel::Sockets })
        );
    }
}
