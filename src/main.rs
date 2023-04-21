//! This is a plugin used to track a given descriptor
//! wallet or set of descriptor wallets, and send
//! events to other listening processes when coin movements are detected.
#[macro_use]
extern crate serde_json;

use std::fmt::Debug;
use std::sync::Arc;
use std::thread::{self, sleep};
use std::time;
use tokio::sync::Mutex;

use anyhow::Ok;
use watchdescriptor::wallet::DescriptorWallet;

use cln_plugin::{anyhow, messages, options, Builder, Error, Plugin};
use tokio;

use bdk::blockchain::ElectrumBlockchain;
use bdk::database::{Database, MemoryDatabase};
use bdk::electrum_client::Client;
use bdk::{bitcoin, descriptor, Balance, SyncOptions, TransactionDetails, Wallet};
use watchdescriptor::state::WatchDescriptor;

const UTXO_DEPOSIT_TAG: &str = "utxo_deposit";
const UTXO_SPENT_TAG: &str = "utxo_spent";

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
        .notification(messages::NotificationTopic::new(UTXO_DEPOSIT_TAG))
        .notification(messages::NotificationTopic::new(UTXO_SPENT_TAG))
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

async fn send_spend_notification(
    plugin: &Plugin<State>,
    tx: &TransactionDetails,
) -> Result<(), Error> {
    let acct = "external";
    let amount = 1000;
    let outpoint = "a18b557b03f2b2d0e25430ef75b70ff5b6bd1f4dd19da3a564502b92623cd8a5:0";
    let txid = "b61d6cd9236bc6cd1ebc38bb3e82d2aeea3502a85feaa7987c63ea9e48537dd5";
    let onchain_spend = json!({
        "account": acct,
        "outpoint": outpoint,
        "spending_txid": txid,
        "amount_msat": amount,
        "coin_type": "bcrt",
        "timestamp": 1679955976,
        "blockheight": 111,
    });
    plugin
        .send_custom_notification(UTXO_SPENT_TAG.to_string(), onchain_spend)
        .await?;
    Ok(())
}

async fn send_deposit_notification(
    plugin: &Plugin<State>,
    tx: &TransactionDetails,
) -> Result<(), Error> {
    let acct = "external";
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
    log::info!("INSIDE SEND DEPOSIT NOTIFICATION ON WATCHDESCRIPTOR SIDE");
    let cloned_plugin = plugin.clone();
    tokio::spawn(async move {
        if let Err(e) = cloned_plugin
            .send_custom_notification(UTXO_DEPOSIT_TAG.to_string(), onchain_deposit)
            .await
        {
            log::error!("Error sending custom notification: {:?}", e);
        }
    });
    Ok(())
}

async fn send_notifications_for_tx(
    plugin: &Plugin<State>,
    tx: TransactionDetails,
) -> Result<(), Error> {
    log::info!("sending notifs for tx: {:?}", tx);
    if tx.sent > 0 {
        log::info!("sending spend notif");
        send_spend_notification(plugin, &tx).await?;
    }

    if tx.received > 0 {
        log::info!("sending deposit notif");
        send_deposit_notification(plugin, &tx).await?;
    }
    Ok(())
}

type State = Arc<Mutex<WatchDescriptor>>;

async fn watchdescriptor(
    plugin: Plugin<State>,
    v: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    let mut dw = DescriptorWallet::try_from(v.clone()).map_err(|x| anyhow!(x))?;
    log::info!("params = {:?}", dw);

    let mut transactions = Vec::<TransactionDetails>::new();
    {
        let wallet = Wallet::new(
            &dw.descriptor,
            dw.change_descriptor.as_ref(),
            bitcoin::Network::Testnet,
            MemoryDatabase::default(),
        )?;
        // holding onto db during later sync call causes RefCell to be carried across await causing runtime panic
        {
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

        transactions = wallet.list_transactions(false)?;
    }
    if transactions.len() > 0 {
        log::info!("found some transactions: {:?}", transactions);
        let new_txs = dw.update_transactions(transactions);
        if new_txs.len() > 0 {
            for tx in new_txs {
                log::info!("new tx found!: {:?}", tx);
                send_notifications_for_tx(&plugin, tx).await?;
            }
        } else {
            log::info!("no new txs this time");
        }
    }
    log::info!("waiting for wallet lock");
    plugin.state().lock().await.add_descriptor_wallet(dw);
    log::info!("wallet added");
    let messasge = format!(
        // "Wallet with checksum {} successfully added",
        "Wallet successfully added",
        // wallet.descriptor_checksum(bdk::KeychainKind::External)
    );
    let ten_millis = time::Duration::from_secs(30);
    thread::sleep(ten_millis);
    log::info!("returning");
    Ok(json!(messasge))
}

async fn listdescriptors(
    plugin: Plugin<State>,
    _v: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    Ok(json!(plugin.state().lock().await.wallets))
}

async fn listtransactions(
    plugin: Plugin<State>,
    _v: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    Ok(json!(plugin.state().lock().await.wallets))
}

async fn block_added_handler(plugin: Plugin<State>, v: serde_json::Value) -> Result<(), Error> {
    log::info!("Got a block_added notification: {}", v);
    log::info!(
        "WatchDescriptor state!!! {:?}",
        plugin.state().lock().await.wallets
    );

    // let descriptor_wallets = &mut plugin.state().lock().unwrap().wallets;
    let client = Client::new("ssl://electrum.blockstream.info:60002")?;
    let blockchain = ElectrumBlockchain::from(client);
    let mut transactions = Vec::<TransactionDetails>::new();
    // for dw in descriptor_wallets.iter_mut() {
    // for dw in plugin.state().lock().await.wallets.iter_mut() {
    for i in 0..1 {
        {
            let wallet = Wallet::new(
                // &dw.descriptor,
                // dw.change_descriptor.as_ref(),
                "wpkh(tpubEBr4i6yk5nf5DAaJpsi9N2pPYBeJ7fZ5Z9rmN4977iYLCGco1VyjB9tvvuvYtfZzjD5A8igzgw3HeWeeKFmanHYqksqZXYXGsw5zjnj7KM9/*)",
                None,
                // bitcoin::Network::Bitcoin,
                bitcoin::Network::Testnet,
                MemoryDatabase::default(),
            )?;
            wallet.sync(&blockchain, SyncOptions::default())?;
            transactions = wallet.list_transactions(false)?;
        }
        if transactions.len() > 0 {
            log::info!("found some transactions: {:?}", transactions);
            for tx in transactions {
                send_notifications_for_tx(&plugin, tx).await?;
            }
            // let new_txs = dw.update_transactions(transactions);
            // if new_txs.len() > 0 {
            //     for tx in new_txs {
            //         send_notifications_for_tx(&plugin, tx).await?;
            //     }
            // } else {
            //     log::info!("no new txs this time");
            // }
        } else {
            log::info!("found no transactions");
        }
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
    log::info!("sending test notification for SANITY");
    plugin
        .send_custom_notification(UTXO_DEPOSIT_TAG.to_string(), onchain_deposit)
        .await?;
    Ok(())
}
