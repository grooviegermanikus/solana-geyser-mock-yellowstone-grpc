//! Example yellowstone-gRPC test client demonstrating the `from_slot` feature.
//!
//! `SubscribeRequest.from_slot` asks the server to *replay* updates it still has
//! buffered starting at the given slot, before switching over to the live stream.
//! This example makes that visible end to end:
//!
//!   1. connect to the endpoint,
//!   2. ask the server for its current slot (the "baseline"),
//!   3. subscribe with `from_slot = baseline - replay_slots`,
//!   4. print every update, tagging each as REPLAY (slot <= baseline) or
//!      LIVE (slot > baseline) so you can watch the stream start in the past
//!      and catch up to the present.
//!
//! Run:
//!   cargo run --example from_slot_client -- --endpoint http://127.0.0.1:10000
//!
//! NOTE: the server only has slots to replay when its geyser config sets
//! `replay_stored_slots` > 0. With the default `replay_stored_slots: 0` the
//! server rejects a `from_slot` request (or has nothing to replay).

use anyhow::Context;
use clap::Parser;
use futures::StreamExt;
use log::{error, info};
use solana_pubkey::Pubkey;
use tokio::runtime::Runtime;
use yellowstone_grpc_client::{ClientTlsConfig, GeyserGrpcClient};
use yellowstone_grpc_proto::geyser::{
    subscribe_update::UpdateOneof, CommitmentLevel, SubscribeRequest,
    SubscribeRequestFilterAccounts, SubscribeRequestFilterSlots,
};

#[derive(Parser, Debug)]
#[command(author, version, about = "yellowstone-gRPC from_slot example client", long_about = None)]
struct Args {
    /// gRPC endpoint of the (mock) yellowstone server.
    #[arg(long, default_value = "http://127.0.0.1:10000")]
    endpoint: String,

    #[arg(long)]
    x_token: Option<String>,

    /// How many slots to rewind: from_slot = current_slot - replay_slots.
    #[arg(long, default_value = "20")]
    replay_slots: u64,

    /// Commitment level for the subscription: processed | confirmed | finalized.
    #[arg(long, default_value = "processed")]
    commitment: Commitment,

    /// Max gRPC decoding message size in bytes (mock accounts can be large).
    #[arg(long, default_value = "67108864")]
    max_decoding_message_size: usize,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum Commitment {
    Processed,
    Confirmed,
    Finalized,
}

impl From<Commitment> for CommitmentLevel {
    fn from(c: Commitment) -> Self {
        match c {
            Commitment::Processed => CommitmentLevel::Processed,
            Commitment::Confirmed => CommitmentLevel::Confirmed,
            Commitment::Finalized => CommitmentLevel::Finalized,
        }
    }
}

fn main() -> anyhow::Result<()> {
    env_logger::init();
    install_rustls_crypto_provider()?;
    let args = Args::parse();
    // Mirror src/main.rs: drive the async work on a manually-built runtime so we
    // don't depend on the tokio `macros` feature.
    Runtime::new()?.block_on(run(args))
}

async fn run(args: Args) -> anyhow::Result<()> {
    let commitment: CommitmentLevel = args.commitment.into();

    info!("connecting to {}", args.endpoint);
    let mut client = GeyserGrpcClient::build_from_shared(args.endpoint.clone())
        .with_context(|| format!("invalid endpoint: {}", args.endpoint))?
        .x_token(args.x_token)?
        .max_decoding_message_size(args.max_decoding_message_size)
        .tls_config(ClientTlsConfig::new().with_native_roots())?
        .connect()
        .await
        .context("failed to connect to gRPC endpoint")?;

    // 1. Learn the server's current slot. Everything at or below this is replay.
    let baseline = client
        .get_slot(Some(commitment))
        .await
        .context("get_slot request failed")?
        .slot;
    let from_slot = baseline.saturating_sub(args.replay_slots);
    info!(
        "current slot = {baseline}; subscribing with from_slot = {from_slot} (rewind {} slots, commitment {commitment:?})",
        args.replay_slots,
    );

    // 2. Subscribe to all accounts + slots, replaying from `from_slot`.
    let request = SubscribeRequest {
        slots: [(
            "sl".to_owned(),
            SubscribeRequestFilterSlots::default(),
        )]
        .into_iter()
        .collect(),
        commitment: Some(commitment as i32),
        from_slot: Some(from_slot),
        ..Default::default()
    };

    let mut stream = client
        .subscribe_once(request)
        .await
        .context("subscribe request failed")?;

    // 3. Stream updates, tagging replay vs live relative to the baseline slot.
    let mut accounts = 0u64;
    let mut slots = 0u64;
    while let Some(message) = stream.next().await {
        let update = message.context("stream error")?;
        match update.update_oneof {
            Some(UpdateOneof::Account(account)) => {
                accounts += 1;
                let slot = account.slot;
                let pubkey = account
                    .account
                    .as_ref()
                    .map(|a| short_pubkey(&a.pubkey))
                    .unwrap_or_else(|| "?".to_owned());
                info!("[{}] account  slot={slot} pubkey={pubkey}", phase(slot, baseline));
            }
            Some(UpdateOneof::Slot(slot_update)) => {
                slots += 1;
                info!(
                    "[{}] slot     slot={} status={}",
                    phase(slot_update.slot, baseline),
                    slot_update.slot,
                    slot_update.status,
                );
            }
            Some(UpdateOneof::Ping(_)) => info!("ping"),
            Some(UpdateOneof::Pong(_)) => info!("pong"),
            Some(other) => info!("other update: {other:?}"),
            None => {}
        }

        let total = accounts + slots;
        if total > 0 && total.is_multiple_of(500) {
            info!("totals so far: {accounts} accounts, {slots} slots");
        }
    }

    error!("stream ended (totals: {accounts} accounts, {slots} slots)");
    Ok(())
}

/// Updates at or below the baseline slot are replayed from the server's buffer;
/// anything newer is the live stream.
fn phase(slot: u64, baseline: u64) -> &'static str {
    if slot <= baseline {
        "REPLAY"
    } else {
        "LIVE"
    }
}

/// Short base58 rendering of a pubkey for readable logs.
fn short_pubkey(pubkey: &[u8]) -> String {
    let s = match Pubkey::try_from(pubkey) {
        Ok(pk) => pk.to_string(),
        Err(_) => return "?".to_owned(),
    };
    format!("{}..{}", &s[..4], &s[s.len() - 4..])
}


fn install_rustls_crypto_provider() -> anyhow::Result<()> {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .map_err(|e| anyhow::anyhow!("failed to install rustls crypto provider: {e:?}"))
}

