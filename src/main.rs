use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;
use agave_geyser_plugin_interface::geyser_plugin_interface::{GeyserPlugin, ReplicaAccountInfoV3, ReplicaAccountInfoVersions, ReplicaBlockInfoV4, ReplicaBlockInfoVersions, SlotStatus};
use clap::Parser;
use log::{debug, info, warn};
use solana_clock::{BankId, Slot};
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
    /// Milliseconds over which to spread each slot's account updates, within the
    /// fixed ~400ms slot. Keep below 400; larger values stretch account delivery and
    /// slow slot production.
    #[arg(long, default_value = "200.0")]
    pub slot_tick_delay_ms: f64,
}

pub fn main() {

    let args = Args::parse();

    let config_file = PathBuf::from_str(&args.geyser_plugin_config).unwrap();

    let plugin = setup_plugin(&config_file).unwrap();

    let runtime = Runtime::new().unwrap();

    let (channel_tx, mut channel_rx) = tokio::sync::mpsc::channel::<MockMessage>(MOCK_BUFFER);

    // runtime.spawn(yellowstone_mock_service::helloworld_traffic(channel_tx));
    runtime.spawn(mock_service::mainnet_traffic(
        channel_tx,
        args.account_bytes_per_slot,
        args.compressibility,
        Duration::from_micros((args.slot_tick_delay_ms * 1000.0) as u64),
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
                    // Tag every account with its slot as the bank_id so it is attributed to
                    // that slot's bank in the block-reconstruction state machine (one bank
                    // per slot). The slot's lifecycle messages below use the same bank_id.
                    plugin
                        .update_account_for_bank(account, mock_account.slot, mock_account.slot)
                        .unwrap();
                }
                Some(MockMessage::Slot(mock_slot)) => {
                    let slot = mock_slot.slot;
                    // One bank per slot: the bank_id a slot's consensus updates target must
                    // match the bank_id its block was reconstructed under.
                    let bank_id: BankId = slot;
                    debug!(
                        "updating slot {} to commitment {} (bank_id {})",
                        slot, mock_slot.commitment_level, bank_id
                    );

                    // On the first (Processed) update for a slot, drive the block to seal so
                    // the yellowstone block-reconstruction state machine actually forwards
                    // the slot-status updates to subscribers.
                    if mock_slot.commitment_level == CommitmentLevel::Processed {
                        seal_block_for_slot(plugin.as_ref(), slot, bank_id);
                    }

                    // Consensus slot statuses (Processed/Confirmed/Rooted) must be emitted via
                    // the bank-aware `update_bank_status` callback in Agave 4.3; `update_slot_status`
                    // is only for lifecycle statuses (FirstShredReceived/Completed/Dead) and
                    // panics with `unreachable!` if given a consensus status.
                    plugin
                        .update_bank_status(
                            slot,
                            Some(slot.saturating_sub(1)),
                            &slot_status_from_commitment_level(mock_slot.commitment_level),
                            bank_id,
                        )
                        .unwrap();
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

/// Drive the yellowstone block-reconstruction state machine far enough that the slot
/// seals and its consensus slot-status updates are delivered to subscribers.
///
/// In the Agave 4.3 geyser model, consensus slot statuses are no longer forwarded
/// directly to `slots` subscribers; they flow through block reconstruction, which only
/// emits a slot once the block seals. Sealing requires, all tagged with the same
/// `bank_id`: a `CreatedBank` status, the must-have sysvar account writes, and block
/// metadata. Transaction/entry counts of 0 mean no transactions or entries are needed.
fn seal_block_for_slot(plugin: &dyn GeyserPlugin, slot: Slot, bank_id: BankId) {
    let parent = Some(slot.saturating_sub(1));

    plugin
        .update_bank_status(slot, parent, &SlotStatus::CreatedBank, bank_id)
        .unwrap();

    for sysvar_pubkey in solana::must_have_sysvar_accounts() {
        let account_v3 = ReplicaAccountInfoV3 {
            pubkey: sysvar_pubkey.as_ref(),
            lamports: 1,
            owner: &[0u8; 32],
            executable: false,
            rent_epoch: 0,
            data: &[],
            write_version: 1,
            txn: None,
        };
        plugin
            .update_account_for_bank(
                ReplicaAccountInfoVersions::V0_0_3(&account_v3),
                slot,
                bank_id,
            )
            .unwrap();
    }

    // blockhash is not parsed anywhere on the reconstruction path; a plain unique string
    // is enough to satisfy the block-meta requirement.
    let blockhash = format!("mockhash{slot}");
    let block_meta = ReplicaBlockInfoV4 {
        parent_slot: slot.saturating_sub(1),
        slot,
        parent_blockhash: "nohash",
        blockhash: &blockhash,
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
        .notify_block_metadata_for_bank(ReplicaBlockInfoVersions::V0_0_4(&block_meta), bank_id)
        .unwrap();
}
