use agave_geyser_plugin_interface::geyser_plugin_interface::GeyserPlugin;
use libloading::Library;
use std::ops::{Deref, DerefMut};

/// Local stand-in for `solana_geyser_plugin_manager::geyser_plugin_manager::LoadedGeyserPlugin`.
///
/// We reimplement it here so this crate does not need to depend on
/// `solana-geyser-plugin-manager`, which transitively pulls in `solana-ledger`
/// and therefore RocksDB (librocksdb-sys / bindgen / libclang).
#[derive(Debug)]
pub struct LoadedGeyserPlugin {
    name: String,
    plugin: Box<dyn GeyserPlugin>,
    // NOTE: While we do not access the library, the plugin we have loaded most
    // certainly does. To ensure we don't SIGSEGV we must declare the library
    // after the plugin so the plugin is dropped first.
    #[allow(dead_code)]
    library: Library,
}

impl LoadedGeyserPlugin {
    pub fn new(library: Library, plugin: Box<dyn GeyserPlugin>, name: Option<String>) -> Self {
        Self {
            name: name.unwrap_or_else(|| plugin.name().to_owned()),
            plugin,
            library,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Deref for LoadedGeyserPlugin {
    type Target = Box<dyn GeyserPlugin>;

    fn deref(&self) -> &Self::Target {
        &self.plugin
    }
}

impl DerefMut for LoadedGeyserPlugin {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.plugin
    }
}
