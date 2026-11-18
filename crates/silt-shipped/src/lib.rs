//! The plugins shipped with Silt (ADR 0100), built from `plugins/` and
//! embedded: each its manifest, README and component, as any plugin is.

pub struct Shipped {
    pub manifest: &'static str,
    pub readme: &'static str,
    /// Empty in a Silt built without them (`SILT_SKIP_PLUGIN_BUILD=1`).
    pub wasm: &'static [u8],
    /// Turned on in every workspace when it's first installed.
    pub everywhere: bool,
}

pub const PLUGINS: [Shipped; 1] = [Shipped {
    manifest: include_str!("../../../plugins/folders/silt-plugin.toml"),
    readme: include_str!("../../../plugins/folders/README.md"),
    wasm: include_bytes!(concat!(env!("OUT_DIR"), "/folders.wasm")),
    everywhere: true,
}];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_imports_only_what_its_manifest_declares() {
        let host = silt_plugin_host::Host::new(silt_plugin_host::Limits::default()).unwrap();
        for shipped in PLUGINS.iter().filter(|shipped| !shipped.wasm.is_empty()) {
            let manifest = silt_plugin::Manifest::parse(shipped.manifest).unwrap();
            assert_eq!(manifest.build, silt_plugin::Build::Shipped, "{}", manifest.id);
            let declared = manifest.capabilities.iter().map(|capability| capability.name().to_owned()).collect();
            let id = manifest.id.clone();
            host.load(manifest, shipped.wasm, &declared).unwrap_or_else(|error| panic!("{id}: {error}"));
        }
    }
}
