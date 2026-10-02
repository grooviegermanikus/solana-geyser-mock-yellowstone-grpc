use agave_geyser_plugin_interface::geyser_plugin_interface::{
    GeyserPlugin, GeyserPluginError, ReplicaAccountInfoV3, SlotStatus,
};
use crate::loaded_plugin::LoadedGeyserPlugin;
use libloading::Library;
use log::{info, warn};
use solana_account::{AccountSharedData, ReadableAccount};
use solana_clock::{Epoch, Slot};
use solana_commitment_config::CommitmentLevel;
use solana_pubkey::Pubkey;
use solana_transaction::sanitized::SanitizedTransaction;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug)]
pub enum MockMessage {
    Slot(MockSlot),
    Account(MockAccount),
}

#[derive(Debug)]
pub struct MockSlot {
    pub slot: Slot,
    pub commitment_level: CommitmentLevel,
}

#[derive(Debug)]
pub struct MockAccount {
    pub slot: Slot,
    pub pubkey: Pubkey,
    pub lamports: u64,
    pub data: Vec<u8>,
    pub owner: Pubkey,
    pub executable: bool,
    pub rent_epoch: Epoch,
}

// see also GeyserPluginManager: load_plugin

pub fn setup_plugin(config_file: &Path) -> Result<Arc<LoadedGeyserPlugin>, GeyserPluginError> {
    let mut new_plugin = load_plugin_from_config(config_file).unwrap();

    setup_logger_for_plugin(new_plugin.as_ref())?;

    // Attempt to on_load with new plugin
    match new_plugin.on_load(config_file.as_os_str().to_str().unwrap(), true) {
        Ok(()) => {
            info!("Successfully loaded plugin: {}", new_plugin.name());
        }
        Err(err) => {
            warn!("Failed to on_load plugin {}: {err}", new_plugin.name());
        }
    }

    Ok(Arc::new(new_plugin))
}

fn load_plugin_from_config(geyser_plugin_config_file: &Path) -> anyhow::Result<LoadedGeyserPlugin> {
    use std::{fs::File, io::Read, path::PathBuf};
    type PluginConstructor = unsafe fn() -> *mut dyn GeyserPlugin;
    use libloading::Symbol;

    let mut file = File::open(geyser_plugin_config_file).map_err(|err| {
        anyhow::anyhow!(
            "Failed to open the plugin config file {geyser_plugin_config_file:?}, error: {err:?}"
        )
    })?;

    let mut contents = String::new();
    file.read_to_string(&mut contents).map_err(|err| {
        anyhow::anyhow!(
            "Failed to read the plugin config file {geyser_plugin_config_file:?}, error: {err:?}"
        )
    })?;

    let result: serde_json::Value = json5::from_str(&contents).map_err(|err| {
        anyhow::anyhow!(
            "The config file {geyser_plugin_config_file:?} is not in a valid Json5 format, error: {err:?}"
        )
    })?;

    let libpath = result["libpath"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("libpath is not set in {geyser_plugin_config_file:?}"))?;
    let mut libpath = PathBuf::from(libpath);
    if libpath.is_relative() {
        let config_dir = geyser_plugin_config_file.parent().ok_or_else(|| {
            anyhow::anyhow!("Failed to resolve parent of {geyser_plugin_config_file:?}")
        })?;
        libpath = config_dir.join(libpath);
    }

    let plugin_name = result["name"].as_str().map(|s| s.to_owned());

    let (plugin, lib) = unsafe {
        let lib = Library::new(libpath)
            .map_err(|e| anyhow::anyhow!("Failed to load plugin library: {e}"))?;
        let constructor: Symbol<PluginConstructor> = lib
            .get(b"_create_plugin")
            .map_err(|e| anyhow::anyhow!("Failed to find _create_plugin symbol: {e}"))?;
        let plugin_raw = constructor();
        (Box::from_raw(plugin_raw), lib)
    };
    Ok(LoadedGeyserPlugin::new(lib, plugin, plugin_name))
}

pub fn accountinfo_from_shared_account_data<'a>(
    account: &'a AccountSharedData,
    txn: &'a Option<&'a SanitizedTransaction>,
    pubkey: &'a Pubkey,
    write_version: u64,
) -> ReplicaAccountInfoV3<'a> {
    ReplicaAccountInfoV3 {
        pubkey: pubkey.as_ref(),
        lamports: account.lamports(),
        owner: account.owner().as_ref(),
        executable: account.executable(),
        rent_epoch: account.rent_epoch(),
        data: account.data(),
        write_version,
        txn: *txn,
    }
}

fn setup_logger_for_plugin(new_plugin: &dyn GeyserPlugin) -> Result<(), GeyserPluginError> {
    new_plugin.setup_logger(log::logger(), log::max_level())
}

pub fn slot_status_from_commitment_level(level: CommitmentLevel) -> SlotStatus {
    match level {
        CommitmentLevel::Processed => SlotStatus::Processed,
        CommitmentLevel::Confirmed => SlotStatus::Confirmed,
        CommitmentLevel::Finalized => SlotStatus::Rooted,
    }
}
