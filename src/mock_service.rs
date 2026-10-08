use bytes::{Bytes, BytesMut};
use rand::distributions::Standard;
use rand::{random, thread_rng, Rng, RngCore};
use solana_clock::UnixTimestamp;
use solana_pubkey::Pubkey;
use agave_geyser_plugin_interface::geyser_plugin_interface::ReplicaAccountInfoV3;
use log::{debug, error, info, warn};
use solana_account::{Account, AccountSharedData};
use solana_clock::Slot;
use solana_commitment_config::CommitmentLevel::{Confirmed, Finalized, Processed};
use std::ops::Add;
use std::path::Path;
use std::thread::{sleep, spawn};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::mpsc::{Sender, UnboundedSender};
use tokio::time::Instant;
use crate::debouncer_instant;
use crate::model::{MockAccount, MockMessage, MockSlot};

// - 20-80 MiB per Slot
// 4000 updates per Slot
pub async fn mainnet_traffic(
    geyser_channel: Sender<MockMessage>,
    bytes_per_slot: u64,
    compressibility: f64,
    slot_tick_delay: Duration,
) {
    info!(
        "Setup mainnet-like traffic source with {} bytes per slot and compressibility {} and slot_tick_delay {:?}",
        bytes_per_slot, compressibility, slot_tick_delay
    );
    let owner = Pubkey::new_unique();
    let account_pubkeys: Vec<Pubkey> = (0..100).map(|_| Pubkey::new_unique()).collect();

    let mut dropped_total = 0;
    let debouncer = debouncer_instant::Debouncer::new(std::time::Duration::from_millis(10));

    for slot in 42_000_000.. {
        let slot_started_at = Instant::now();

        let sizes = vec![
            // mainnet distribution
            0, 8, 8, 165, 165, 165, 165, 11099, 11099, 11099, 11099, 11099,
            11099,
            // shape with a lot larger sizes
            // 200000, 220000, 230000,
        ];
        // 10MB -> stream buffer size peaks at 30
        // 30MB -> stream buffer size peaks at 10000th and more
        // per slot
        let mut bytes_total = 0;

        let mut requested_sizes: Vec<u64> = Vec::new();

        for i in 0..99_999_999 {
            let data_size = sizes[i % sizes.len()];

            if bytes_total + data_size > bytes_per_slot {
                break;
            }

            requested_sizes.push(data_size);
            bytes_total += data_size;
        }

        debug!(
            "will send account updates for slot {} down the stream ({} bytes) in {} messages",
            slot,
            bytes_total,
            requested_sizes.len()
        );

        // Spread the account updates across the first `slot_tick_delay` seconds of
        // the slot, leaving some headroom before the 400ms slot boundary below.
        let avg_delay_sec = slot_tick_delay.as_secs_f64() / requested_sizes.len() as f64;

        for (i, data_bytes) in requested_sizes.into_iter().enumerate() {
            let next_message_at = slot_started_at
                .add(Duration::from_secs_f64(avg_delay_sec * i as f64));

            let account_build_started_at = Instant::now();
            let mut data = vec![0; data_bytes as usize];
            let entropy_bytes = (data_bytes as f64 * (1.0 - compressibility)) as usize;
            assert!(
                entropy_bytes <= data_bytes as usize,
                "entropy_bytes overflow"
            );
            fill_with_xor_prng(&mut data[0..entropy_bytes]);
            let data = data.to_vec();

            let account_pubkey = account_pubkeys[i % sizes.len()];

            let epoch_us = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_micros() as u64;

            let account = MockAccount {
                slot,
                pubkey: account_pubkey,
                lamports: epoch_us,
                data,
                owner,
                executable: false,
                rent_epoch: 0,
            };

            let sent_result = geyser_channel.try_send(MockMessage::Account(account));

            match sent_result {
                Ok(_) => {}
                Err(TrySendError::Full(_)) => {
                    dropped_total += 1;
                    if debouncer.can_fire() {
                        warn!(
                            "channel is full (total drops: {}) - dropping message",
                            dropped_total
                        );
                    }
                }
                Err(TrySendError::Closed(_)) => {
                    error!("channel was closed - shutting down");
                    return;
                }
            }

            tokio::time::sleep_until(next_message_at).await;
        }

        let block_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as UnixTimestamp;

        let mut sent_results = vec![];

        let sent_result = geyser_channel.try_send(MockMessage::Slot(MockSlot {
            slot,
            commitment_level: Processed,
        }));
        sent_results.push(sent_result);

        {
            let slot_confirmed = slot - 2;
            let sent_result = geyser_channel.try_send(MockMessage::Slot(MockSlot {
                slot: slot_confirmed,
                commitment_level: Confirmed,
            }));
            sent_results.push(sent_result);
        }

        {
            let slot_finalized = slot - 32;
            let sent_result = geyser_channel.try_send(MockMessage::Slot(MockSlot {
                slot: slot_finalized,
                commitment_level: Finalized,
            }));
            sent_results.push(sent_result);
        }

        for sent_result in sent_results {
            match sent_result {
                Ok(_) => {}
                Err(TrySendError::Full(_)) => {
                    dropped_total += 1;
                    if debouncer.can_fire() {
                        warn!(
                            "channel is full (total drops: {}) - dropping message",
                            dropped_total
                        );
                    }
                }
                Err(TrySendError::Closed(_)) => {
                    error!("channel was closed - shutting down");
                    return;
                }
            }
        }

        tokio::time::sleep_until(slot_started_at.add(Duration::from_millis(400))).await;
    }
}

#[allow(dead_code)]
pub async fn helloworld_traffic(grpc_channel: UnboundedSender<MockAccount>) {
    loop {
        let account_mock = MockAccount {
            slot: 999_888,
            pubkey: Pubkey::new_unique(),
            lamports: 0,
            owner: Pubkey::new_unique(),
            executable: false,
            rent_epoch: 0,
            data: vec![1, 2, 3],
        };

        grpc_channel.send(account_mock).expect("send");
        debug!("sent account update down the stream");

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn fill_with_xor_prng(binary: &mut [u8]) {
    let seed_n = binary.len();
    let mut state: u32 = 0xdeadbeef;
    for i_word in 0..seed_n / 4 {
        let mut x = state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        state = x;

        binary[i_word * 4 + 0] = (x >> 0) as u8;
        binary[i_word * 4 + 1] = (x >> 8) as u8;
        binary[i_word * 4 + 2] = (x >> 16) as u8;
        binary[i_word * 4 + 3] = (x >> 24) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_with_xor_prng_test() {
        let mut data_full_entropy = vec![0; 1000];
        fill_with_xor_prng(&mut data_full_entropy);
        let compressed_size = lz4_flex::compress(&data_full_entropy).len();
        assert_eq!(compressed_size, 1005);
    }

    #[test]
    fn fill_with_xor_prng_lowentropy_test() {
        let mut data_low_entropy = vec![0; 1000];
        fill_with_xor_prng(&mut data_low_entropy[0..200]);
        let compressed_size = lz4_flex::compress(&data_low_entropy).len();
        assert_eq!(compressed_size, 219);
    }
}
