use agave_geyser_plugin_interface::geyser_plugin_interface::{ReplicaAccountInfoV3, SlotStatus};
use solana_account::{AccountSharedData, ReadableAccount};
use solana_commitment_config::CommitmentLevel;
use solana_pubkey::Pubkey;
use solana_transaction::sanitized::SanitizedTransaction;
use std::str::FromStr;

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

pub fn slot_status_from_commitment_level(level: CommitmentLevel) -> SlotStatus {
    match level {
        CommitmentLevel::Processed => SlotStatus::Processed,
        CommitmentLevel::Confirmed => SlotStatus::Confirmed,
        CommitmentLevel::Finalized => SlotStatus::Rooted,
    }
}

/// Sysvar accounts the yellowstone block-reconstruction state machine requires to
/// have been written for a bank before it will seal that bank's block (see
/// `MUST_HAVE_SYSVAR_ACCOUNTS` in yellowstone-grpc-geyser's `block_reconstruction_v2`).
/// Without all four, a slot never seals and its consensus slot-status updates
/// (Processed/Confirmed/Finalized) are never delivered to subscribers.
pub fn must_have_sysvar_accounts() -> [Pubkey; 4] {
    [
        "SysvarC1ock11111111111111111111111111111111",
        "SysvarS1otHashes111111111111111111111111111",
        "SysvarS1otHistory11111111111111111111111111",
        "SysvarRecentB1ockHashes11111111111111111111",
    ]
    .map(|s| Pubkey::from_str(s).expect("valid sysvar pubkey"))
}

