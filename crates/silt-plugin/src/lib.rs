//! Plugins (ADR 0103), the parts with no I/O: a plugin's manifest and its
//! validation, the cards it describes, the effects it declares, the prepared
//! call an approval is bound to, and the rules that decide each call. The
//! daemon's WebAssembly host runs plugins; this crate says what they may do.

mod api;
mod card;
mod effect;
mod glob;
mod manifest;
mod prepared;
mod rules;
mod settings;
mod sha256;

pub use api::{Call, Context, Output, Tool};
pub use card::{Block, Card, FileChange, Item, Tone};
pub use effect::{Channel, Effect, Ledger, Observed, Refused};
pub use glob::{Glob, specificity};
pub use manifest::{
    ActionKind, ActionSpec, Build, Capability, CredentialSpec, Docs, Environments, Manifest, ManifestError, Send, SettingKind,
    SettingScope, SettingSpec, WORLD,
};
pub use prepared::{Prepared, canonical_json};
pub use rules::{Decision, Rule, RuleSource, Rules, ScopedRule, decide, environment_for};
pub use settings::{SettingSource, SettingValue, fits, resolve_scoped, resolve_setting, setting_key};
pub use sha256::sha256_hex;
