//! Write a Silt plugin in Rust (ADR 0100). Implement [`Plugin`], export it
//! with [`export!`], and build for `wasm32-wasip2`:
//!
//! ```ignore
//! struct Hello;
//!
//! impl silt_plugin_api::Plugin for Hello {
//!     fn tools() -> Vec<Tool> { … }
//!     fn prepare(tool: &str, input: &Value, context: &Context) -> Result<Call, String> { … }
//!     fn run(tool: &str, input: &Value, call: &Call, context: &Context) -> Result<Output, String> { … }
//! }
//!
//! silt_plugin_api::export!(Hello);
//! ```
//!
//! Each capability is a module here ([`http`], [`git`], …). Use only those
//! your `silt-plugin.toml` declares: a plugin importing one it wasn't
//! granted doesn't load.

#[doc(hidden)]
pub mod bindings {
    wit_bindgen::generate!({
        world: "silt-plugin",
        path: "wit",
        pub_export_macro: true,
        default_bindings_module: "silt_plugin_api::bindings",
    });
}

pub use bindings::silt::plugin::{git, http, mcp, reach, sockets, ssh, thread};
pub use serde_json::{self, Value, json};
pub use silt_plugin::{Block, Call, Card, Channel, Context, Effect, FileChange, Item, Output, Tone, Tool};

/// What a plugin implements.
pub trait Plugin {
    /// The tools it offers agents.
    fn tools() -> Vec<Tool>;

    /// Prepares a call: which action it is, the fields its rules match on,
    /// the card Silt shows, and every effect it will cause. Nothing may
    /// change here; a prepared call may never run.
    fn prepare(tool: &str, input: &Value, context: &Context) -> Result<Call, String>;

    /// Runs a call as prepared, once it's allowed. Only the effects `call`
    /// declared get through.
    fn run(tool: &str, input: &Value, call: &Call, context: &Context) -> Result<Output, String>;
}

/// Logging, settings and a small store: always there.
pub mod host {
    use crate::bindings::silt::plugin::host as raw;
    pub use crate::bindings::silt::plugin::host::Level;

    pub fn log(level: Level, message: &str) {
        raw::log(level, message);
    }

    /// A setting's value, as the person, an org or the manifest set it.
    pub fn setting(key: &str) -> Option<serde_json::Value> {
        raw::setting(key).and_then(|text| serde_json::from_str(&text).ok())
    }

    pub fn kv_get(key: &str) -> Option<Vec<u8>> {
        raw::kv_get(key)
    }

    pub fn kv_set(key: &str, value: &[u8]) -> Result<(), String> {
        raw::kv_set(key, value)
    }
}

/// The SHA-256 of a body, in hex, as an HTTP effect binds it.
pub use silt_plugin::sha256_hex as body_sha256;

#[doc(hidden)]
pub mod glue {
    //! Turns a [`crate::Plugin`] into the world's exports.

    use crate::{Call, Context, Plugin};

    pub fn describe<P: Plugin>() -> String {
        serde_json::to_string(&P::tools()).unwrap_or_else(|_| "[]".into())
    }

    fn parse<T: serde::de::DeserializeOwned>(what: &str, text: &str) -> Result<T, String> {
        serde_json::from_str(text).map_err(|error| format!("the host sent {what} the plugin can't read: {error}"))
    }

    pub fn prepare<P: Plugin>(tool: String, input: String, context: String) -> Result<String, String> {
        let input: serde_json::Value = parse("input", &input)?;
        let context: Context = parse("a context", &context)?;
        let call = P::prepare(&tool, &input, &context)?;
        serde_json::to_string(&call).map_err(|error| error.to_string())
    }

    pub fn run<P: Plugin>(tool: String, input: String, call: String, context: String) -> Result<String, String> {
        let input: serde_json::Value = parse("input", &input)?;
        let call: Call = parse("a call", &call)?;
        let context: Context = parse("a context", &context)?;
        let output = P::run(&tool, &input, &call, &context)?;
        serde_json::to_string(&output).map_err(|error| error.to_string())
    }
}

/// Exports `$plugin`, a [`Plugin`], as the component's `plugin` interface.
#[macro_export]
macro_rules! export {
    ($plugin:ty) => {
        const _: () = {
            struct Component;

            impl $crate::bindings::exports::silt::plugin::plugin::Guest for Component {
                fn describe() -> String {
                    $crate::glue::describe::<$plugin>()
                }

                fn prepare(tool: String, input: String, context: String) -> Result<String, String> {
                    $crate::glue::prepare::<$plugin>(tool, input, context)
                }

                fn run(tool: String, input: String, call: String, context: String) -> Result<String, String> {
                    $crate::glue::run::<$plugin>(tool, input, call, context)
                }
            }

    $crate::bindings::export!(Component with_types_in $crate::bindings);
        };
    };
}
