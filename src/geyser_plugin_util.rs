use agave_geyser_plugin_interface::geyser_plugin_interface::{
    GeyserPlugin, GeyserPluginError, ReplicaAccountInfoV3, SlotStatus,
};
use log::{info, warn};
use solana_account::{AccountSharedData, ReadableAccount};
use solana_clock::{Epoch, Slot};
use solana_commitment_config::CommitmentLevel;
use solana_pubkey::Pubkey;
use solana_transaction::sanitized::SanitizedTransaction;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;


pub fn setup_plugin(config_file: &Path) -> Result<Box<dyn GeyserPlugin>, GeyserPluginError> {
    let mut plugin = yellowstone_grpc_geyser::plugin::entry::Plugin::default();

    setup_logger_for_plugin(&plugin)?;

    match plugin.on_load(config_file.as_os_str().to_str().unwrap(), true) {
        Ok(()) => {
            info!("Successfully loaded plugin: {}", plugin.name());
        }
        Err(err) => {
            warn!("Failed to on_load plugin {}: {err}", plugin.name());
        }
    }

    Ok(Box::new(plugin))
}

fn setup_logger_for_plugin(new_plugin: &dyn GeyserPlugin) -> Result<(), GeyserPluginError> {
    new_plugin.setup_logger(log::logger(), log::max_level())
}

