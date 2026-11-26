//! What reaches a sandboxed agent from a plugin or a relayed device (ADR
//! 0093): its requests classified as reads or changes, the canonical form
//! an approval is bound to, the data rules a result must pass, and what of
//! adb one device's relay lets through. Also Auto's check of an agent's call
//! (ADR 0129). Pure: the daemon runs the plugins, relays and checks, and
//! enforces what this decides.

pub mod adb;
mod data;
mod graphql;
pub mod judge;
mod request;
mod schema;

pub use data::{DataClass, Finding, graphql_fields, json, text};
pub use graphql::{Classified, OperationKind, classify};
pub use request::{Canonical, shown};
pub use schema::{INTROSPECTION, Schema};
