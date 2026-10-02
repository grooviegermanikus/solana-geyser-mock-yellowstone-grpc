use solana_clock::{Epoch, Slot};
use solana_commitment_config::CommitmentLevel;
use solana_pubkey::Pubkey;

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

