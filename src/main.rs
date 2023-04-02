//! This is a plugin used to track a given descriptor
//! wallet or set of descriptor wallets, and send
//! events to other listening processes when coin movements are detected.
#[macro_use]
extern crate serde_json;

use std::sync::{Arc, Mutex};

use watchdescriptor::params::DescriptorWallet;

use cln_plugin::{anyhow, messages, Builder, Error, Plugin};
use tokio;

use bdk::blockchain::ElectrumBlockchain;
use bdk::database::MemoryDatabase;
use bdk::electrum_client::Client;
use bdk::{bitcoin, descriptor, Balance, SyncOptions, Wallet};
use watchdescriptor::watchdescriptor::WatchDescriptor;

const COIN_DEPOSIT_TAG: &str = "coin_onchain_deposit";
const COIN_SPEND_TAG: &str = "coin_onchain_spend";

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let watch_descriptor = WatchDescriptor::new();
    // watch_descriptor.add_descriptor("tr([af4c5952/86h/0h/0h]xpub6DTzDxFnUS1vriU7fc3VkwdTnArhk6FafoZHRcfwjRqo7vkMnbAiKK9AEhR4feqcdsE36Y4ZCLHBcEszJcvV3pMLhS4D9Ed5VNhH6Cw17Pp/0/*)".to_string()).await;
    // watch_descriptor.add_descriptor("tr([af4c5952/86h/0h/0h]xpub6DTzDxFnUS1vriU7fc3VkwdTnArhk6FafoZHRcfwjRqo7vkMnbAiKK9AEhR4feqcdsE36Y4ZCLHBcEszJcvV3pMLhS4D9Ed5VNhH6Cw17Pp/1/*)".to_string()).await;
    let plugin_state = Arc::new(Mutex::new(watch_descriptor));
    if let Some(plugin) = Builder::new(tokio::io::stdin(), tokio::io::stdout())
    // let builder = Builder::new(tokio::io::stdin(), tokio::io::stdout())
        .notification(messages::NotificationTopic::new(COIN_DEPOSIT_TAG))
        .notification(messages::NotificationTopic::new(COIN_SPEND_TAG))
        .rpcmethod(
            "watchdescriptor",
            "Watch one or more external wallet descriptors and emit notifications when coins are moved",
            watchdescriptor,
        )
        .rpcmethod(
            "listdescriptors",
            "List descriptor wallets currently being watched",
            listdescriptors,
        )
        .subscribe("block_added", block_added_handler)
        .dynamic()
        // .configure()
        .start(plugin_state.clone())
        .await?
    {
        plugin.join().await
    } else {
        Ok(())
    }
}

type State = Arc<Mutex<WatchDescriptor>>;

async fn watchdescriptor(
    plugin: Plugin<State>,
    v: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    let params = DescriptorWallet::try_from(v.clone()).map_err(|x| anyhow!(x))?;
    log::info!("params = {:?}", params);
    // let descriptors = plugin.state().lock().unwrap().descriptors;
    let descriptors = plugin.state().lock().unwrap().clone().descriptors;
    for d in descriptors {
        let mut plugin_state = plugin.state().lock().unwrap().clone();
        plugin_state.add_descriptor(d).await;
    }
    // plugin_state
    //     .descriptors
    //     .iter()
    //     .map(|d| plugin_state.add_descriptor(d.to_string()));
    Ok(json!("Wallet successfully added"))
    // Ok(json!({ "Wallet successfully added:": v }))
    // Ok(json!(wallet.get_balance()?))
}

async fn listdescriptors(
    plugin: Plugin<State>,
    _v: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    Ok(json!(plugin.state().lock().unwrap().descriptors))
}

async fn block_added_handler(plugin: Plugin<State>, v: serde_json::Value) -> Result<(), Error> {
    log::info!("Got a block_added notification: {}", v);
    log::info!(
        "WatchDescriptor state!!! {:?}",
        plugin.state().lock().unwrap().descriptors
    );

    let descriptors = plugin.state().lock().unwrap().descriptors.clone();

    let client = Client::new("ssl://electrum.blockstream.info:60002")?;
    let blockchain = ElectrumBlockchain::from(client);
    log::info!("descriptor: {:?}", descriptors[0]);
    log::info!("change descriptor: {:?}", descriptors[1]);

    let mut balance: Balance = Balance {
        immature: 0,
        trusted_pending: 0,
        untrusted_pending: 0,
        confirmed: 0,
    };
    {
        let wallet = Wallet::new(
            // "tr([af4c5952/86h/0h/0h]xpub6DTzDxFnUS1vriU7fc3VkwdTnArhk6FafoZHRcfwjRqo7vkMnbAiKK9AEhR4feqcdsE36Y4ZCLHBcEszJcvV3pMLhS4D9Ed5VNhH6Cw17Pp/0/*)",
            // &params.descriptor,
            &descriptors[0],
            // params.change_descriptor.as_ref(),
            Some(&descriptors[1]),
            bitcoin::Network::Bitcoin,
            MemoryDatabase::default(),
        )?;

        wallet.sync(&blockchain, SyncOptions::default())?;
        balance = wallet.get_balance()?;
    }

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
