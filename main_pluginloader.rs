use crate::geyser_plugin_util::{
    accountinfo_from_shared_account_data, setup_plugin, slot_status_from_commitment_level,
    MockAccount, MockMessage,
};
use agave_geyser_plugin_interface::geyser_plugin_interface::{
    ReplicaAccountInfoV3, ReplicaAccountInfoVersions, ReplicaBlockInfoV3, ReplicaBlockInfoV4,
    ReplicaBlockInfoVersions, SlotStatus,
};
use clap::Parser;
use log::{info, warn};
use solana_commitment_config::CommitmentLevel;
use solana_transaction_status::RewardsAndNumPartitions;
use std::path::Path;
use tracing::debug;
use tracing_subscriber::EnvFilter;

mod debouncer_instant;
mod geyser_plugin_util;
mod loaded_plugin;
mod mock_service;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    // point to config.json
    #[arg(long)]
    pub geyser_plugin_config: String,
    #[arg(long, default_value = "30000000")]
    pub account_bytes_per_slot: u64,
    #[arg(long, default_value = "0.0")]
    pub compressibility: f64,
    #[arg(long, default_value = "350.0")]
    pub slot_tick_delay: f64,
}

// note: if this channel fills the process will very likely die with OOM at some point!
const MOCK_BUFFER: usize = 102400;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let args = Args::parse();

    assert!(
        args.compressibility >= 0.0 && args.compressibility <= 1.0,
        "compressibility must be in [0.0, 1.0]"
    );

    info!(
        "Loading geyser plugin from config: {}",
        args.geyser_plugin_config
    );
    let config_file = Path::new(&args.geyser_plugin_config);
    assert!(config_file.exists(), "Config file must exist");

    let plugin = setup_plugin(config_file.as_ref()).unwrap();

    let (channel_tx, mut channel_rx) = tokio::sync::mpsc::channel::<MockMessage>(MOCK_BUFFER);

    // tokio::task::spawn(yellowstone_mock_service::helloworld_traffic(channel_tx));
    tokio::task::spawn(mock_service::mainnet_traffic(
        channel_tx,
        args.account_bytes_per_slot,
        args.compressibility,
        args.slot_tick_delay,
    ));

    std::thread::spawn(move || {
        let log_debouncer = debouncer_instant::Debouncer::new(std::time::Duration::from_millis(10));

        'recv_loop: loop {
            match channel_rx.blocking_recv() {
                Some(MockMessage::Account(mock_account)) => {
                    // usually there are some 10-50 messages in the channel
                    if channel_rx.len() > 100 && log_debouncer.can_fire() {
                        info!(
                            "sending account {:?} with data_len={} ({} messags in channel)",
                            mock_account.pubkey,
                            mock_account.data.len(),
                            channel_rx.len()
                        );
                    }

                    let account_v3 = ReplicaAccountInfoV3 {
                        pubkey: mock_account.pubkey.as_ref(),
                        lamports: mock_account.lamports,
                        owner: mock_account.owner.as_ref(),
                        executable: mock_account.executable,
                        rent_epoch: mock_account.rent_epoch,
                        data: mock_account.data.as_ref(),
                        write_version: 999999,
                        txn: None,
                    };

                    let account = ReplicaAccountInfoVersions::V0_0_3(&account_v3);
                    plugin
                        .update_account(account, mock_account.slot, false)
                        .unwrap();
                }
                Some(MockMessage::Slot(mock_slot)) => {
                    debug!(
                        "updating slot to {} with commitment {}",
                        mock_slot.slot, mock_slot.commitment_level
                    );
                    plugin
                        .update_slot_status(
                            mock_slot.slot,
                            None,
                            &slot_status_from_commitment_level(mock_slot.commitment_level),
                        )
                        .unwrap();

                    if mock_slot.commitment_level == CommitmentLevel::Processed {
                        let block_meta = ReplicaBlockInfoV4 {
                            parent_slot: mock_slot.slot - 1,
                            slot: mock_slot.slot,
                            parent_blockhash: "nohash",
                            blockhash: "nohash",
                            rewards: &RewardsAndNumPartitions {
                                rewards: vec![],
                                num_partitions: None,
                            },
                            block_time: None,
                            block_height: None,
                            executed_transaction_count: 0,
                            entry_count: 0,
                        };
                        plugin
                            .notify_block_metadata(ReplicaBlockInfoVersions::V0_0_4(&block_meta))
                            .unwrap();
                    }
                }
                None => {
                    warn!("channel closed - shutting down");
                    break 'recv_loop;
                }
            }
        }
    })
    .join()
    .unwrap();
}
