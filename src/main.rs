//! This is a plugin used to track a given descriptor
//! wallet or set of descriptor wallets, and send
//! events to other listening processes when coin movements are detected.
#[macro_use]
extern crate serde_json;

use std::sync::{Arc, Mutex};

use watchdescriptor::params::WatchParams;

use cln_plugin::{anyhow, messages, Builder, Error, Plugin};
use tokio;

use bdk::blockchain::ElectrumBlockchain;
use bdk::database::MemoryDatabase;
use bdk::electrum_client::Client;
use bdk::{bitcoin, SyncOptions, Wallet};
use watchdescriptor::watchdescriptor::WatchDescriptor;

const COIN_DEPOSIT_TAG: &str = "coin_onchain_deposit";
const COIN_SPEND_TAG: &str = "coin_onchain_spend";

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // let watch_descriptor = WatchDescriptor::new();
    // let state = WatchDescriptor::new();

    // if let Some(plugin) = Builder::new(tokio::io::stdin(), tokio::io::stdout())
    let builder = Builder::new(tokio::io::stdin(), tokio::io::stdout())
        .notification(messages::NotificationTopic::new(COIN_DEPOSIT_TAG))
        .notification(messages::NotificationTopic::new(COIN_SPEND_TAG))
        .rpcmethod(
            "watchdescriptor",
            "Watch one or more external wallet descriptors and emit notifications when coins are moved",
            watchdescriptor,
        )
        .subscribe("block_added", block_added_handler)
        .dynamic();
    // .configure()
    // .start(watch_descriptor.clone())
    // .await?
    // {
    //     plugin.join().await
    // } else {
    //     Ok(())
    // }

    let midstate = if let Some(midstate) = builder.configure().await? {
        midstate
    } else {
        return Ok(());
    };

    // let watch_descriptor = WatchDescriptor::new().await;
    let watch_descriptor = Arc::new(Mutex::new(WatchDescriptor::new().await));

    let plugin = midstate.start(watch_descriptor.clone()).await?;
    // tokio::spawn(async move {
    //     RetryManager::new(
    //         wt_client,
    //         rx,
    //         max_elapsed_time,
    //         auto_retry_delay,
    //         max_interval_time,
    //     )
    //     .manage_retry()
    //     .await
    // });
    plugin.join().await
}

async fn watchdescriptor(_p: Plugin<()>, v: serde_json::Value) -> Result<serde_json::Value, Error> {
    let params = WatchParams::try_from(v).map_err(|x| anyhow!(x))?;
    log::info!("params = {:?}", params);
    let client = Client::new("ssl://electrum.blockstream.info:60002")?;
    let blockchain = ElectrumBlockchain::from(client);
    log::info!("descriptor: {:?}", params.descriptor);
    log::info!("change descriptor: {:?}", params.change_descriptor);
    let wallet = Wallet::new(
        // "tr([af4c5952/86h/0h/0h]xpub6DTzDxFnUS1vriU7fc3VkwdTnArhk6FafoZHRcfwjRqo7vkMnbAiKK9AEhR4feqcdsE36Y4ZCLHBcEszJcvV3pMLhS4D9Ed5VNhH6Cw17Pp/0/*)",
        &params.descriptor,
        params.change_descriptor.as_ref(),
        bitcoin::Network::Bitcoin,
        MemoryDatabase::default(),
    )?;

    wallet.sync(&blockchain, SyncOptions::default())?;

    // println!("Descriptor balance: {} SAT", wallet.get_balance()?);

    // Ok(v)
    Ok(json!(wallet.get_balance()?))
}

async fn block_added_handler(plugin: Plugin<()>, v: serde_json::Value) -> Result<(), Error> {
    log::info!("Got a block_added notification: {}", v);
    let acct = "test account";
    let transfer_from: Option<String> = None;
    let amount = 1000;
    let outpoint = "a18b557b03f2b2d0e25430ef75b70ff5b6bd1f4dd19da3a564502b92623cd8a5:0";
    let onchain_deposit = json!({
        "account": acct,
        "transfer_from": transfer_from,
        "outpoint": outpoint,
        "amount_msat": amount,
        "coin_type": "bcrt",
        "timestamp": 1679955976,
        "blockheight": 111,
    });
    plugin
        .send_custom_notification(COIN_DEPOSIT_TAG.to_string(), onchain_deposit)
        .await?;
    Ok(())
}
