//! The phones paired over the network (ADR 0112), and pairing itself: a
//! one-time code, the phone's ask, and the person's answer on the machine.
//! Only each phone's token's hash is kept.

use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};
use silt_proto::Capabilities;

const PHONES_FILE: &str = "phones.json";
/// How long a code shows before it stops working.
pub(crate) const CODE_TTL_MS: i64 = 10 * 60 * 1000;
/// How long a phone's ask waits for the person.
pub(crate) const ASK_TTL_MS: i64 = 10 * 60 * 1000;
/// Letters a person reads and types without mixing up: no 0, O, 1, I, L or U.
const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTVWXYZ23456789";
pub(crate) const CODE_LENGTH: usize = 8;

/// Random bytes, from the OS's generator through `uuid`.
fn random_bytes(count: usize) -> Vec<u8> {
    std::iter::repeat_with(|| uuid::Uuid::new_v4().into_bytes()).flatten().take(count).collect()
}

/// A token for a phone or an ask: 256 random bits as hex.
pub(crate) fn new_token() -> String {
    random_bytes(32).iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn hash(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}

pub(crate) fn new_code() -> String {
    // 30 letters: rejecting bytes at 240 and above keeps each equally likely.
    let mut code = String::with_capacity(CODE_LENGTH);
    while code.len() < CODE_LENGTH {
        for byte in random_bytes(16) {
            if byte < 240 && code.len() < CODE_LENGTH {
                code.push(char::from(ALPHABET[usize::from(byte) % ALPHABET.len()]));
            }
        }
    }
    code
}

/// A code as typed: any case, with or without its dash and spaces.
pub(crate) fn normalized(code: &str) -> String {
    code.chars().filter(char::is_ascii_alphanumeric).map(|char| char.to_ascii_uppercase()).collect()
}

/// `K7QM-2XPA`, as the code shows.
pub(crate) fn shown(code: &str) -> String {
    let (first, rest) = code.split_at(code.len() / 2);
    format!("{first}-{rest}")
}

/// What a phone's browser says it is, for the person deciding: `iPhone ·
/// Safari 19`.
pub(crate) fn describe(user_agent: &str) -> String {
    let device = if user_agent.contains("iPhone") {
        "iPhone"
    } else if user_agent.contains("iPad") {
        "iPad"
    } else if user_agent.contains("Android") {
        "Android"
    } else if user_agent.contains("Macintosh") {
        "Mac"
    } else if user_agent.contains("Windows") {
        "Windows"
    } else if user_agent.contains("Linux") {
        "Linux"
    } else {
        "A device"
    };
    let version = |marker: &str| {
        user_agent.split(marker).nth(1).and_then(|rest| rest.split(['.', ' ', ';', ')']).next()).filter(|major| !major.is_empty())
    };
    let browser = if let Some(major) = version("Firefox/").or_else(|| version("FxiOS/")) {
        Some(format!("Firefox {major}"))
    } else if let Some(major) = version("Edg/") {
        Some(format!("Edge {major}"))
    } else if let Some(major) = version("SamsungBrowser/") {
        Some(format!("Samsung Internet {major}"))
    } else if let Some(major) = version("CriOS/").or_else(|| version("Chrome/")) {
        Some(format!("Chrome {major}"))
    } else if user_agent.contains("Safari/") {
        Some(version("Version/").map_or_else(|| "Safari".to_owned(), |major| format!("Safari {major}")))
    } else {
        None
    };
    browser.map_or_else(|| device.to_owned(), |browser| format!("{device} · {browser}"))
}

/// Where a phone's pushes go, as its browser subscribed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PushSubscription {
    pub(crate) endpoint: String,
    pub(crate) p256dh: String,
    pub(crate) auth: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Phone {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) token_hash: String,
    pub(crate) capabilities: Capabilities,
    pub(crate) created_ms: i64,
    pub(crate) last_seen_ms: i64,
    #[serde(default)]
    pub(crate) push: Option<PushSubscription>,
}

/// The paired phones, kept in `phones.json`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PhoneBook {
    pub(crate) phones: Vec<Phone>,
}

impl PhoneBook {
    pub(crate) fn load(dir: &Path) -> io::Result<Self> {
        match std::fs::read(dir.join(PHONES_FILE)) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error),
        }
    }

    pub(crate) fn save(&self, dir: &Path) -> io::Result<()> {
        silt_os::create_private_dir(dir)?;
        let json = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        silt_os::write_private(&dir.join(PHONES_FILE), &json)
    }

    /// The phone `token` belongs to.
    pub(crate) fn by_token(&self, token: &str) -> Option<&Phone> {
        let hashed = hash(token);
        self.phones.iter().find(|phone| same(&phone.token_hash, &hashed))
    }

    pub(crate) fn get_mut(&mut self, id: &str) -> Option<&mut Phone> {
        self.phones.iter_mut().find(|phone| phone.id == id)
    }

    pub(crate) fn remove(&mut self, id: &str) -> Option<Phone> {
        let index = self.phones.iter().position(|phone| phone.id == id)?;
        Some(self.phones.remove(index))
    }

    /// A name no other phone has: `iPhone`, then `iPhone 2`.
    fn unique_name(&self, wanted: &str) -> String {
        let taken = |name: &str| self.phones.iter().any(|phone| phone.name == name);
        if !taken(wanted) {
            return wanted.to_owned();
        }
        (2..).map(|count| format!("{wanted} {count}")).find(|name| !taken(name)).expect("an unused name")
    }
}

/// Compares without stopping at the first difference.
fn same(a: &str, b: &str) -> bool {
    let difference = a.bytes().zip(b.bytes()).fold(0u8, |difference, (a, b)| difference | (a ^ b));
    a.len() == b.len() && std::hint::black_box(difference) == 0
}

/// The code on show, and what a phone pairing with it may do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Code {
    pub(crate) code: String,
    pub(crate) capabilities: Capabilities,
    pub(crate) expires_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AskState {
    Waiting,
    /// Allowed: the phone's token, until the phone takes it.
    Allowed {
        token: Option<String>,
        phone: String,
    },
    Denied,
}

/// A phone that opened a code, waiting for the person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Ask {
    pub(crate) id: String,
    /// The hash of what ties the ask to the browser that made it.
    claim_hash: String,
    pub(crate) phone: String,
    pub(crate) from: String,
    pub(crate) code: String,
    pub(crate) capabilities: Capabilities,
    pub(crate) made_ms: i64,
    pub(crate) state: AskState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(crate) enum PairError {
    #[error("this code is used up or has expired")]
    UsedUp,
    #[error("no phone asks to pair with that")]
    NoAsk,
}

/// The code on show and the asks it brought.
#[derive(Debug, Default)]
pub(crate) struct Pairing {
    pub(crate) code: Option<Code>,
    pub(crate) asks: Vec<Ask>,
}

impl Pairing {
    /// Shows a new code, which replaces any other.
    pub(crate) fn start(&mut self, capabilities: Capabilities, now_ms: i64) -> Code {
        let code = Code { code: new_code(), capabilities, expires_ms: now_ms + CODE_TTL_MS };
        self.code = Some(code.clone());
        code
    }

    /// Drops what has expired.
    pub(crate) fn expire(&mut self, now_ms: i64) {
        if self.code.as_ref().is_some_and(|code| code.expires_ms <= now_ms) {
            self.code = None;
        }
        self.asks.retain(|ask| ask.made_ms + ASK_TTL_MS > now_ms);
    }

    /// A phone opened `code`: the code is used up, and the person is asked.
    /// Answers with the claim that ties the phone's browser to its ask.
    pub(crate) fn ask(&mut self, code: &str, phone: String, from: String, now_ms: i64) -> Result<(String, Ask), PairError> {
        self.expire(now_ms);
        let wanted = normalized(code);
        let Some(shown) = self.code.take_if(|shown| same(&shown.code, &wanted)) else {
            return Err(PairError::UsedUp);
        };
        let claim = new_token();
        let ask = Ask {
            id: new_token()[..16].to_owned(),
            claim_hash: hash(&claim),
            phone,
            from,
            code: shown.code,
            capabilities: shown.capabilities,
            made_ms: now_ms,
            state: AskState::Waiting,
        };
        self.asks.push(ask.clone());
        Ok((claim, ask))
    }

    /// The ask `claim` made.
    pub(crate) fn claimed(&mut self, claim: &str, now_ms: i64) -> Option<&mut Ask> {
        self.expire(now_ms);
        let hashed = hash(claim);
        self.asks.iter_mut().find(|ask| same(&ask.claim_hash, &hashed))
    }

    /// The person's answer to ask `id`. Allowed, the phone joins `phones`
    /// with a new token, which its next look at the ask takes.
    pub(crate) fn answer(
        &mut self,
        id: &str,
        allow: bool,
        phones: &mut PhoneBook,
        now_ms: i64,
    ) -> Result<Option<Phone>, PairError> {
        self.expire(now_ms);
        let ask = self.asks.iter_mut().find(|ask| ask.id == id && ask.state == AskState::Waiting).ok_or(PairError::NoAsk)?;
        if !allow {
            ask.state = AskState::Denied;
            return Ok(None);
        }
        let token = new_token();
        let phone = Phone {
            id: new_token()[..16].to_owned(),
            name: phones.unique_name(ask.phone.split(" · ").next().unwrap_or("Phone")),
            token_hash: hash(&token),
            capabilities: ask.capabilities,
            created_ms: now_ms,
            last_seen_ms: now_ms,
            push: None,
        };
        ask.state = AskState::Allowed { token: Some(token), phone: phone.id.clone() };
        phones.phones.push(phone.clone());
        Ok(Some(phone))
    }

    /// The asks still waiting for the person.
    pub(crate) fn waiting(&self) -> impl Iterator<Item = &Ask> {
        self.asks.iter().filter(|ask| ask.state == AskState::Waiting)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_easy_to_read_and_type() {
        let code = new_code();
        assert_eq!(code.len(), CODE_LENGTH);
        assert!(code.bytes().all(|byte| ALPHABET.contains(&byte)), "{code}");
        assert_eq!(shown("K7QM2XPA"), "K7QM-2XPA");
        assert_eq!(normalized(" k7qm-2xpa "), "K7QM2XPA");
        assert_ne!(new_code(), new_code());
    }

    #[test]
    fn a_code_pairs_one_phone_once_the_person_allows_it() {
        let (mut pairing, mut phones, now) = (Pairing::default(), PhoneBook::default(), 1_000);
        let code = pairing.start(Capabilities::PHONE, now);
        let (claim, ask) =
            pairing.ask(&shown(&code.code).to_lowercase(), "iPhone · Safari 19".into(), "192.168.1.41".into(), now).unwrap();
        assert_eq!(pairing.ask(&code.code, "Android".into(), "192.168.1.9".into(), now), Err(PairError::UsedUp), "used once");
        assert_eq!(pairing.claimed(&claim, now).unwrap().state, AskState::Waiting);
        assert!(pairing.claimed("someone else's", now).is_none());
        let phone = pairing.answer(&ask.id, true, &mut phones, now).unwrap().unwrap();
        assert_eq!((phone.name.as_str(), phone.capabilities), ("iPhone", Capabilities::PHONE));
        let AskState::Allowed { token: Some(token), .. } = pairing.claimed(&claim, now).unwrap().state.clone() else {
            panic!("allowed")
        };
        assert_eq!(phones.by_token(&token).unwrap().id, phone.id);
        assert!(phones.by_token(&new_token()).is_none());
        assert!(!phones.phones[0].token_hash.contains(&token), "only the token's hash is kept");
        assert_eq!(pairing.answer(&ask.id, true, &mut phones, now), Err(PairError::NoAsk), "answered once");
    }

    #[test]
    fn a_denied_ask_pairs_nothing() {
        let (mut pairing, mut phones, now) = (Pairing::default(), PhoneBook::default(), 1_000);
        let code = pairing.start(Capabilities::PHONE, now);
        let (claim, ask) = pairing.ask(&code.code, "Android".into(), "192.168.1.9".into(), now).unwrap();
        assert_eq!(pairing.answer(&ask.id, false, &mut phones, now), Ok(None));
        assert_eq!(pairing.claimed(&claim, now).unwrap().state, AskState::Denied);
        assert!(phones.phones.is_empty());
    }

    #[test]
    fn codes_and_asks_expire() {
        let (mut pairing, mut phones) = (Pairing::default(), PhoneBook::default());
        let code = pairing.start(Capabilities::PHONE, 0);
        assert_eq!(pairing.ask(&code.code, "iPhone".into(), "x".into(), CODE_TTL_MS), Err(PairError::UsedUp));
        let code = pairing.start(Capabilities::PHONE, 0);
        let (_, ask) = pairing.ask(&code.code, "iPhone".into(), "x".into(), 10).unwrap();
        assert_eq!(pairing.answer(&ask.id, true, &mut phones, 10 + ASK_TTL_MS), Err(PairError::NoAsk));
    }

    #[test]
    fn a_new_code_replaces_the_one_on_show() {
        let mut pairing = Pairing::default();
        let first = pairing.start(Capabilities::PHONE, 0);
        pairing.start(Capabilities::ALL, 0);
        assert_eq!(pairing.ask(&first.code, "iPhone".into(), "x".into(), 1), Err(PairError::UsedUp));
    }

    #[test]
    fn phones_get_names_of_their_own() {
        let (mut pairing, mut phones) = (Pairing::default(), PhoneBook::default());
        for _ in 0..2 {
            let code = pairing.start(Capabilities::PHONE, 0);
            let (_, ask) = pairing.ask(&code.code, "iPhone · Safari 19".into(), "x".into(), 0).unwrap();
            pairing.answer(&ask.id, true, &mut phones, 0).unwrap();
        }
        let names: Vec<_> = phones.phones.iter().map(|phone| phone.name.as_str()).collect();
        assert_eq!(names, ["iPhone", "iPhone 2"]);
    }

    #[test]
    fn the_phone_book_is_kept_privately() {
        let dir = tempfile::tempdir().unwrap();
        let mut phones = PhoneBook::default();
        phones.phones.push(Phone {
            id: "p1".into(),
            name: "iPhone".into(),
            token_hash: hash("t"),
            capabilities: Capabilities::PHONE,
            created_ms: 1,
            last_seen_ms: 2,
            push: None,
        });
        phones.save(dir.path()).unwrap();
        assert_eq!(PhoneBook::load(dir.path()).unwrap(), phones);
        assert_eq!(PhoneBook::load(&dir.path().join("none")).unwrap(), PhoneBook::default());
    }

    #[test]
    fn browsers_are_described_as_people_know_them() {
        let iphone = "Mozilla/5.0 (iPhone; CPU iPhone OS 19_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/19.0 Mobile/15E148 Safari/604.1";
        let pixel = "Mozilla/5.0 (Linux; Android 15; Pixel 8) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Mobile Safari/537.36";
        let firefox = "Mozilla/5.0 (Android 15; Mobile; rv:144.0) Gecko/144.0 Firefox/144.0";
        assert_eq!(describe(iphone), "iPhone · Safari 19");
        assert_eq!(describe(pixel), "Android · Chrome 142");
        assert_eq!(describe(firefox), "Android · Firefox 144");
        assert_eq!(describe("curl/8"), "A device");
    }
}
