//! This is a plugin used to track a given descriptor
//! wallet or set of descriptor wallets, and send
//! events to other listening processes when coin movements are detected.
#[macro_use]
extern crate serde_json;

use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use watchdescriptor::wallet::DescriptorWallet;

use cln_plugin::{anyhow, messages, options, Builder, Error, Plugin};
use tokio;

use bdk::blockchain::ElectrumBlockchain;
use bdk::database::{Database, MemoryDatabase};
use bdk::electrum_client::Client;
use bdk::{bitcoin, Balance, SyncOptions, Wallet};
use watchdescriptor::state::WatchDescriptor;

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
        .option(options::ConfigOption::new(
            "network",
            options::Value::String(bitcoin::Network::Bitcoin.to_string()),
            "Which network to use: [bitcoin, testnet, signet, regtest]",
        ))
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
    let mut dw = DescriptorWallet::try_from(v.clone()).map_err(|x| anyhow!(x))?;
    log::info!("params = {:?}", dw);

    let wallet = Wallet::new(
        &dw.descriptor,
        dw.change_descriptor.as_ref(),
        bitcoin::Network::Testnet,
        MemoryDatabase::default(),
    )?;
    {
        // holding onto db during later sync call causes RefCell to be carried across await causing runtime panic
        let db = wallet.database();
        let sync_time = db.get_sync_time()?;
        match sync_time {
            Some(st) => {
                log::info!("found previous sync time: {:?}", st);
            }
            None => {
                log::info!("no previous sync time found");
            }
        }
    }
    log::info!("creating client");
    let client = Client::new("ssl://electrum.blockstream.info:60002")?;
    log::info!("creating blockchain");
    let blockchain = ElectrumBlockchain::from(client);
    log::info!("syncing wallet");
    wallet.sync(&blockchain, SyncOptions::default())?;

    log::info!("retrieving sync time");
    let db = wallet.database();
    if let Some(st) = db.get_sync_time()? {
        log::info!("new previous sync time: {:?}", st);
        dw.update_last_synced(st.block_time)
    } else {
        log::info!("no previous sync time found, even after sync");
    }

    let transactions = wallet.list_transactions(false)?;
    if transactions.len() > 0 {
        log::info!("found some transactions: {:?}", transactions);
        let new_txs = dw.update_transactions(transactions);
        if new_txs.len() > 0 {
            for tx in new_txs {
                log::info!("new tx found!: {:?}", tx);
            }
        } else {
            log::info!("no new txs this time");
        }
    }
    plugin.state().lock().unwrap().add_descriptor_wallet(dw);
    let messasge = format!(
        "Wallet with checksum {} successfully added",
        wallet.descriptor_checksum(bdk::KeychainKind::External)
    );
    Ok(json!(messasge))
}

async fn listdescriptors(
    plugin: Plugin<State>,
    _v: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    Ok(json!(plugin.state().lock().unwrap().wallets))
}

async fn listtransactions(
    plugin: Plugin<State>,
    _v: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    Ok(json!(plugin.state().lock().unwrap().wallets))
}

async fn block_added_handler(plugin: Plugin<State>, v: serde_json::Value) -> Result<(), Error> {
    log::info!("Got a block_added notification: {}", v);
    log::info!(
        "WatchDescriptor state!!! {:?}",
        plugin.state().lock().unwrap().wallets
    );

    {
        // let descriptors = plugin.state().lock().unwrap().wallets.clone();
        let descriptors = &plugin.state().lock().unwrap().wallets;

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

        let wallet = Wallet::new(
            // "tr([af4c5952/86h/0h/0h]xpub6DTzDxFnUS1vriU7fc3VkwdTnArhk6FafoZHRcfwjRqo7vkMnbAiKK9AEhR4feqcdsE36Y4ZCLHBcEszJcvV3pMLhS4D9Ed5VNhH6Cw17Pp/0/*)",
            // "wpkh(tpubEBr4i6yk5nf5DAaJpsi9N2pPYBeJ7fZ5Z9rmN4977iYLCGco1VyjB9tvvuvYtfZzjD5A8igzgw3HeWeeKFmanHYqksqZXYXGsw5zjnj7KM9/*)",
            // &params.descriptor,
            &descriptors[0].descriptor,
            // params.change_descriptor.as_ref(),
            descriptors[1].change_descriptor.as_ref(),
            // bitcoin::Network::Bitcoin,
            bitcoin::Network::Testnet,
            MemoryDatabase::default(),
        )?;

        // balance = wallet.get_balance()?;
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
