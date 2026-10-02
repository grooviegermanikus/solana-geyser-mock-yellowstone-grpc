use agave_geyser_plugin_interface::geyser_plugin_interface::{ReplicaAccountInfoV3, SlotStatus};
use solana_account::{AccountSharedData, ReadableAccount};
use solana_commitment_config::CommitmentLevel;
use solana_pubkey::Pubkey;
use solana_transaction::sanitized::SanitizedTransaction;

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

