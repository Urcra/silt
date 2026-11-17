//! What reaches a sandboxed agent from a plugin (ADR 0086): its requests
//! classified as reads or changes, the canonical form an approval is bound
//! to, and the data rules a result must pass. Pure: the daemon runs the
//! plugins and enforces what this decides.

mod data;
mod graphql;
mod request;

pub use data::{DataClass, Finding, graphql_fields, json, text};
pub use graphql::{Classified, OperationKind, classify};
pub use request::{Canonical, shown};
