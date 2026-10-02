use std::path::PathBuf;
use std::str::FromStr;
use agave_geyser_plugin_interface::geyser_plugin_interface::{ReplicaAccountInfoV3, ReplicaAccountInfoVersions, ReplicaBlockInfoV4, ReplicaBlockInfoVersions};
use clap::Parser;
use log::{debug, info, warn};
use solana_clock::BankId;
use solana_commitment_config::CommitmentLevel;
use solana_transaction_status::RewardsAndNumPartitions;
use tokio::runtime::Runtime;
use crate::geyser_plugin_util::setup_plugin;
use crate::model::MockMessage;
use crate::solana::slot_status_from_commitment_level;

mod geyser_plugin_util;
pub mod solana;
pub mod model;
mod mock_service;
mod debouncer_instant;

// note: if this channel fills the process will very likely die with OOM at some point!
const MOCK_BUFFER: usize = 102400;

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

pub fn main() {

    let args = Args::parse();

    let config_file = PathBuf::from_str(&args.geyser_plugin_config).unwrap();

    let plugin = setup_plugin(&config_file).unwrap();

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
        let mut bank_id: BankId = 1000;

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
                        .update_account_for_bank(account, mock_account.slot, bank_id)
                        .unwrap();
                    bank_id += 1;
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
