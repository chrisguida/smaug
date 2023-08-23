//! This is a plugin used to track a given descriptor
//! wallet or set of descriptor wallets, and send
//! events to other listening processes when coin movements are detected.
#[macro_use]
extern crate serde_json;

use bdk::chain::keychain::LocalChangeSet;
use bdk::chain::{ConfirmationTime, ConfirmationTimeAnchor};

use bdk_file_store::Store;
use cln_rpc::model::DatastoreMode;
use cln_rpc::{
    model::requests::{DatastoreRequest, ListdatastoreRequest},
    ClnRpc, Request, Response,
};
use home::home_dir;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

use anyhow::Ok;
use watchdescriptor::wallet::{DescriptorWallet, DATADIR};

use cln_plugin::{anyhow, messages, options, Builder, Error, Plugin};
use tokio;

use bdk::{bitcoin, KeychainKind, TransactionDetails, Wallet};
use watchdescriptor::state::WatchDescriptor;

const UTXO_DEPOSIT_TAG: &str = "utxo_deposit";
const UTXO_SPENT_TAG: &str = "utxo_spent";

// #[tokio::main]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), anyhow::Error> {
    // Create data dir if it does not exist
    fs::create_dir_all(&home_dir().unwrap().join(DATADIR)).unwrap_or_else(|e| {
        log::error!("Cannot create data dir: {e:?}");
        std::process::exit(1);
    });
    // let watch_descriptor = WatchDescriptor::new();
    // let mut watch_descriptor = WatchDescriptor {
    //     // wallets: vec![],
    //     wallets: serde_json::from_reader(fs::File::open(
    //         home_dir().unwrap().join(DATADIR).join(WALLETS_FILE),
    //     )?)?,
    //     // network: bitcoin::Network::Bitcoin,
    //     // network: cln_network.parse::<bitcoin::Network>().unwrap(),
    //     network: bitcoin::Network::Bitcoin,
    // };
    // // watch_descriptor.add_descriptor("tr([af4c5952/86h/0h/0h]xpub6DTzDxFnUS1vriU7fc3VkwdTnArhk6FafoZHRcfwjRqo7vkMnbAiKK9AEhR4feqcdsE36Y4ZCLHBcEszJcvV3pMLhS4D9Ed5VNhH6Cw17Pp/0/*)".to_string()).await;
    // // watch_descriptor.add_descriptor("tr([af4c5952/86h/0h/0h]xpub6DTzDxFnUS1vriU7fc3VkwdTnArhk6FafoZHRcfwjRqo7vkMnbAiKK9AEhR4feqcdsE36Y4ZCLHBcEszJcvV3pMLhS4D9Ed5VNhH6Cw17Pp/1/*)".to_string()).await;
    // let plugin_state = Arc::new(Mutex::new(watch_descriptor.clone()));
    // if let Some(plugin) = Builder::new(tokio::io::stdin(), tokio::io::stdout())
    // if let Some(plugin) = Builder::new(tokio::io::stdin(), tokio::io::stdout())
    let builder = Builder::new(tokio::io::stdin(), tokio::io::stdout())
        .option(options::ConfigOption::new(
            "wd_network",
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
        .dynamic();
    // .start(plugin_state.clone())
    // .configure()
    // .await?
    // {
    let configured_plugin = if let Some(cp) = builder.configure().await? {
        cp
    } else {
        return Ok(());
    };
    log::info!(
        "Configuration from CLN main daemon: {:?}",
        configured_plugin.configuration()
    );
    let cln_network = configured_plugin.configuration().network;
    let rpc_file = configured_plugin.configuration().rpc_file;
    let p = Path::new(&rpc_file);

    let mut rpc = ClnRpc::new(p).await?;
    let lds_response = rpc
        .call(Request::ListDatastore(ListdatastoreRequest {
            key: Some(vec!["watchdescriptor".to_owned()]),
        }))
        .await
        .map_err(|e| anyhow!("Error calling listdatastore: {:?}", e))?;
    let wallets: BTreeMap<String, DescriptorWallet> = match lds_response {
        Response::ListDatastore(r) => match r.datastore.is_empty() {
            true => BTreeMap::new(),
            false => match &r.datastore[0].string {
                Some(deserialized) => match serde_json::from_str(&deserialized) {
                    core::result::Result::Ok(dws) => dws,
                    core::result::Result::Err(e) => {
                        log::error!("{}", e);
                        return Err(e.into());
                    }
                },
                None => BTreeMap::new(),
            },
        },
        _ => panic!(),
    };
    // let wallets: Vec<DescriptorWallet> =
    let watch_descriptor = WatchDescriptor {
        // wallets: vec![],
        wallets,
        network: cln_network.parse::<bitcoin::Network>().unwrap(),
    };
    let plugin_state = Arc::new(Mutex::new(watch_descriptor.clone()));
    plugin_state.lock().await.network = cln_network.parse::<bitcoin::Network>().unwrap();
    // log::info!("Initial Plugin State: {:?}", watch_descriptor);
    // let plugin_state = Arc::new(Mutex::new(watch_descriptor));
    let plugin = configured_plugin.start(plugin_state).await?;
    plugin.join().await
    // } else {
    //     Ok(())
    // }
}

// assume we own all inputs, ie sent from our wallet. all inputs and outputs should generate coin movement bookkeeper events
async fn spend_tx_notify<'a>(
    plugin: &Plugin<State>,
    wallet: &Wallet<Store<'a, LocalChangeSet<KeychainKind, ConfirmationTimeAnchor>>>,
    tx: &TransactionDetails,
) -> Result<(), Error> {
    match tx.transaction.clone() {
        Some(t) => {
            // send spent notification for each input
            for input in t.input.iter() {
                if let Some(po) = wallet.tx_graph().get_txout(input.previous_output) {
                    match tx.confirmation_time {
                        ConfirmationTime::Unconfirmed { .. } => {
                            continue;
                        }
                        ConfirmationTime::Confirmed { height, time } => {
                            let acct = format!(
                                "watchdescriptor:{}",
                                wallet.descriptor_checksum(bdk::KeychainKind::External)
                            );
                            let amount = po.value;
                            let outpoint = format!("{}", input.previous_output.to_string());
                            log::info!("outpoint = {}", format!("{}", outpoint));
                            let onchain_spend = json!({
                                "account": acct,
                                "outpoint": outpoint,
                                "spending_txid": tx.txid.to_string(),
                                "amount_msat": amount,
                                "coin_type": "bcrt",
                                "timestamp": format!("{}", time),
                                "blockheight": format!("{}", height),
                            });
                            log::info!("INSIDE SEND SPEND NOTIFICATION ON WATCHDESCRIPTOR SIDE");
                            let cloned_plugin = plugin.clone();
                            tokio::spawn(async move {
                                if let Err(e) = cloned_plugin
                                    .send_custom_notification(
                                        UTXO_SPENT_TAG.to_string(),
                                        onchain_spend,
                                    )
                                    .await
                                {
                                    log::error!("Error sending custom notification: {:?}", e);
                                }
                            });
                        }
                    }
                } else {
                    log::info!("Transaction prevout not found");
                }
            }

            // send deposit notification for every output, since all of them are spends from our wallet
            for (vout, output) in t.output.iter().enumerate() {
                match tx.confirmation_time {
                    ConfirmationTime::Unconfirmed { .. } => {
                        continue;
                    }
                    ConfirmationTime::Confirmed { height, time } => {
                        let acct: String;
                        let transfer_from: String;
                        if wallet.is_mine(&output.script_pubkey) {
                            acct = format!(
                                "watchdescriptor:{}",
                                wallet.descriptor_checksum(bdk::KeychainKind::External)
                            );
                            transfer_from = "external".to_owned();
                        } else {
                            transfer_from = format!(
                                "watchdescriptor:{}",
                                wallet.descriptor_checksum(bdk::KeychainKind::External)
                            );
                            acct = "external".to_owned();
                        }
                        let amount = output.value;
                        let outpoint = format!("{}:{}", tx.txid.to_string(), vout.to_string());
                        log::info!(
                            "outpoint = {}",
                            format!("{}:{}", tx.txid.to_string(), vout.to_string())
                        );
                        let onchain_deposit = json!({
                                "account": acct,
                                "transfer_from": transfer_from,
                                "outpoint": outpoint,
                                "spending_txid": tx.txid.to_string(),
                                "amount_msat": amount,
                                "coin_type": "bcrt",
                                "timestamp": format!("{}", time),
                                "blockheight": format!("{}", height),
                        });
                        log::info!("INSIDE SEND DEPOSIT NOTIFICATION ON WATCHDESCRIPTOR SIDE");
                        let cloned_plugin = plugin.clone();
                        tokio::spawn(async move {
                            if let Err(e) = cloned_plugin
                                .send_custom_notification(
                                    UTXO_DEPOSIT_TAG.to_string(),
                                    onchain_deposit,
                                )
                                .await
                            {
                                log::error!("Error sending custom notification: {:?}", e);
                            }
                        });
                    }
                }
            }
        }
        None => {
            log::info!("TransactionDetails is missing a Transaction");
        }
    }
    Ok(())
}

// assume we own no inputs. sent to us from someone else's wallet.
// all outputs we own should generate utxo deposit events.
// outputs we don't own should not generate events.
async fn receive_tx_notify<'a>(
    plugin: &Plugin<State>,
    wallet: &Wallet<Store<'a, LocalChangeSet<KeychainKind, ConfirmationTimeAnchor>>>,
    tx: &TransactionDetails,
) -> Result<(), Error> {
    match tx.transaction.clone() {
        Some(t) => {
            for (vout, output) in t.output.iter().enumerate() {
                if wallet.is_mine(&output.script_pubkey) {
                    match tx.confirmation_time {
                        ConfirmationTime::Unconfirmed { .. } => {
                            continue;
                        }
                        ConfirmationTime::Confirmed { height, time } => {
                            let acct: String;
                            let transfer_from: String;
                            if wallet.is_mine(&output.script_pubkey) {
                                acct = format!(
                                    "watchdescriptor:{}",
                                    wallet.descriptor_checksum(bdk::KeychainKind::External)
                                );
                                transfer_from = "external".to_owned();
                            } else {
                                // transfer_from = format!(
                                //     "watchdescriptor:{}",
                                //     wallet.descriptor_checksum(bdk::KeychainKind::External)
                                // );
                                // acct = "external".to_owned();
                                continue;
                            }
                            let amount = output.value;
                            let outpoint = format!("{}:{}", tx.txid.to_string(), vout.to_string());
                            log::info!(
                                "outpoint = {}",
                                format!("{}:{}", tx.txid.to_string(), vout.to_string())
                            );
                            let onchain_deposit = json!({
                                    "account": acct,
                                    "transfer_from": transfer_from,
                                    "outpoint": outpoint,
                                    "spending_txid": tx.txid.to_string(),
                                    "amount_msat": amount,
                                    "coin_type": "bcrt",
                                    "timestamp": format!("{}", time),
                                    "blockheight": format!("{}", height),
                            });
                            log::info!("INSIDE SEND DEPOSIT NOTIFICATION ON WATCHDESCRIPTOR SIDE");
                            let cloned_plugin = plugin.clone();
                            tokio::spawn(async move {
                                if let Err(e) = cloned_plugin
                                    .send_custom_notification(
                                        UTXO_DEPOSIT_TAG.to_string(),
                                        onchain_deposit,
                                    )
                                    .await
                                {
                                    log::error!("Error sending custom notification: {:?}", e);
                                }
                            });
                        }
                    }
                }
            }
        }
        None => {
            log::info!("TransactionDetails is missing a Transaction");
        }
    }
    Ok(())
}

// assume we own some inputs and not others.
// this tx was generated collaboratively between our wallet and (an)other wallet(s).
// send events for all our owned inputs.
// request manual intervention to identify which outputs are ours. send them to bkpr in a temporary account?
async fn shared_tx_notify<'a>(
    plugin: &Plugin<State>,
    wallet: &Wallet<Store<'a, LocalChangeSet<KeychainKind, ConfirmationTimeAnchor>>>,
    tx: &TransactionDetails,
) -> Result<(), Error> {
    match tx.transaction.clone() {
        Some(t) => {
            // send spent notification for each input that spends one of our outputs
            for input in t.input.iter() {
                if let Some(po) = wallet.tx_graph().get_txout(input.previous_output) {
                    match tx.confirmation_time {
                        ConfirmationTime::Unconfirmed { .. } => {
                            continue;
                        }
                        ConfirmationTime::Confirmed { height, time } => {
                            if wallet.is_mine(&po.script_pubkey) {
                                let acct = format!(
                                    "watchdescriptor:{}",
                                    wallet.descriptor_checksum(bdk::KeychainKind::External)
                                );
                                let amount = po.value;
                                let outpoint = format!("{}", input.previous_output.to_string());
                                log::info!("outpoint = {}", format!("{}", outpoint));
                                let onchain_spend = json!({
                                    "account": acct,
                                    "outpoint": outpoint,
                                    "spending_txid": tx.txid.to_string(),
                                    "amount_msat": amount,
                                    "coin_type": "bcrt",
                                    "timestamp": format!("{}", time),
                                    "blockheight": format!("{}", height),
                                });
                                log::info!(
                                    "INSIDE SEND SPEND NOTIFICATION ON WATCHDESCRIPTOR SIDE"
                                );
                                let cloned_plugin = plugin.clone();
                                tokio::spawn(async move {
                                    if let Err(e) = cloned_plugin
                                        .send_custom_notification(
                                            UTXO_SPENT_TAG.to_string(),
                                            onchain_spend,
                                        )
                                        .await
                                    {
                                        log::error!("Error sending custom notification: {:?}", e);
                                    }
                                });
                            }
                        }
                    }
                } else {
                    log::info!("Transaction prevout not found");
                }
            }

            // send deposit notification for every output, since all of them *might be* spends from our wallet.
            // store them in a temp account and let the user update later as needed.
            for (vout, output) in t.output.iter().enumerate() {
                match tx.confirmation_time {
                    ConfirmationTime::Unconfirmed { .. } => {
                        continue;
                    }
                    ConfirmationTime::Confirmed { height, time } => {
                        let acct: String;
                        let transfer_from: String;
                        let our_acct = format!(
                            "watchdescriptor:{}:shared_outputs",
                            wallet.descriptor_checksum(bdk::KeychainKind::External)
                        );
                        let ext_acct = "external".to_owned();
                        if wallet.is_mine(&output.script_pubkey) {
                            acct = our_acct;
                            transfer_from = ext_acct;
                        } else {
                            acct = ext_acct;
                            transfer_from = our_acct;
                        }
                        let amount = output.value;
                        let outpoint = format!("{}:{}", tx.txid.to_string(), vout.to_string());
                        log::info!(
                            "outpoint = {}",
                            format!("{}:{}", tx.txid.to_string(), vout.to_string())
                        );
                        let onchain_deposit = json!({
                                "account": acct,
                                "transfer_from": transfer_from,
                                "outpoint": outpoint,
                                "spending_txid": tx.txid.to_string(),
                                "amount_msat": amount,
                                "coin_type": "bcrt",
                                "timestamp": format!("{}", time),
                                "blockheight": format!("{}", height),
                        });
                        log::info!("INSIDE SEND DEPOSIT NOTIFICATION ON WATCHDESCRIPTOR SIDE");
                        let cloned_plugin = plugin.clone();
                        tokio::spawn(async move {
                            if let Err(e) = cloned_plugin
                                .send_custom_notification(
                                    UTXO_DEPOSIT_TAG.to_string(),
                                    onchain_deposit,
                                )
                                .await
                            {
                                log::error!("Error sending custom notification: {:?}", e);
                            }
                        });
                    }
                }
            }
        }
        None => {
            log::info!("TransactionDetails is missing a Transaction");
        }
    }
    Ok(())
}

async fn send_notifications_for_tx<'a>(
    plugin: &Plugin<State>,
    wallet: &Wallet<Store<'a, LocalChangeSet<KeychainKind, ConfirmationTimeAnchor>>>,
    tx: TransactionDetails,
) -> Result<(), Error> {
    log::info!("sending notifs for txid/tx: {:?} {:?}", tx.txid, tx);
    // we own all inputs
    if tx.clone().transaction.unwrap().input.iter().all(|x| {
        match wallet.tx_graph().get_txout(x.previous_output) {
            Some(o) => {
                log::info!(
                    "output is mine?: {:?} {:?}",
                    o,
                    wallet.is_mine(&o.script_pubkey)
                );
                wallet.is_mine(&o.script_pubkey)
            }
            None => {
                log::info!("output not found in tx graph: {:?}", x.previous_output);
                false
            }
        }
    }) {
        log::info!("sending spend notif");
        spend_tx_notify(plugin, wallet, &tx).await?;
    } else
    // we own no inputs
    if !tx.clone().transaction.unwrap().input.iter().any(|x| {
        match wallet.tx_graph().get_txout(x.previous_output) {
            Some(o) => {
                log::info!(
                    "output is mine?: {:?} {:?}",
                    o,
                    wallet.is_mine(&o.script_pubkey)
                );
                wallet.is_mine(&o.script_pubkey)
            }
            None => {
                log::info!("output not found in tx graph: {:?}", x.previous_output);
                false
            }
        }
    }) {
        log::info!("sending deposit notif");
        receive_tx_notify(plugin, wallet, &tx).await?;
    }
    // we own some inputs but not others
    else {
        log::info!("sending shared notif");
        shared_tx_notify(plugin, wallet, &tx).await?;
    }

    // if tx.sent > 0 {

    // }

    // if tx.received > 0 {

    // }
    Ok(())
}

type State = Arc<Mutex<WatchDescriptor>>;
async fn watchdescriptor<'a>(
    plugin: Plugin<State>,
    v: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    let mut dw = DescriptorWallet::try_from(v.clone()).map_err(|x| anyhow!(x))?;
    dw.network = Some(plugin.state().lock().await.network.clone());
    log::info!("params = {:?}", dw);

    let wallet = dw.fetch_wallet().await?;

    // transactions = wallet.list_transactions(false)?;
    let bdk_transactions_iter = wallet.transactions();
    // let bdk_transactions = bdk_transactions_iter.collect::<CanonicalTx<T, A>>();
    // let mut transactions = Vec::<CanonicalTx<'a, Transaction, ConfirmationTimeAnchor>>::new();
    let mut transactions = Vec::<TransactionDetails>::new();
    for bdk_transaction in bdk_transactions_iter {
        // log::info!("BDK transaction = {:?}", bdk_transaction);
        log::info!("BDK transaction = {:?}", bdk_transaction.node.tx);
        transactions.push(wallet.get_tx(bdk_transaction.node.txid, true).unwrap());
    }

    if transactions.len() > 0 {
        log::info!("found some transactions: {:?}", transactions);
        let new_txs = dw.update_transactions(transactions);
        if new_txs.len() > 0 {
            for tx in new_txs {
                log::info!("new tx found!: {:?}", tx);
                send_notifications_for_tx(&plugin, &wallet, tx).await?;
            }
        } else {
            log::info!("no new txs this time");
        }
    }
    log::info!("waiting for wallet lock");
    plugin.state().lock().await.add_descriptor_wallet(&dw)?;

    let wallets_str = json!(plugin.state().lock().await.wallets).to_string();
    let rpc_file = plugin.configuration().rpc_file;
    let p = Path::new(&rpc_file);

    let mut rpc = ClnRpc::new(p).await?;
    let _ds_response = rpc
        .call(Request::Datastore(DatastoreRequest {
            key: vec!["watchdescriptor".to_owned()],
            string: Some(wallets_str),
            hex: None,
            mode: Some(DatastoreMode::CREATE_OR_REPLACE),
            generation: None,
        }))
        .await
        .map_err(|e| anyhow!("Error calling listdatastore: {:?}", e))?;
    log::info!("wallet added");
    let message = format!(
        "Wallet with checksum {} successfully added",
        &dw.get_name()?
    );
    log::info!("returning");
    Ok(json!(message))
}

async fn listdescriptors(
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

    let descriptor_wallets = &mut plugin.state().lock().await.wallets;
    for (_dw_desc, dw) in descriptor_wallets.iter_mut() {
        let wallet = dw.fetch_wallet().await?;
        let bdk_transactions_iter = wallet.transactions();
        let mut transactions = Vec::<TransactionDetails>::new();
        for bdk_transaction in bdk_transactions_iter {
            // log::info!("BDK transaction = {:?}", bdk_transaction);
            log::info!("BDK transaction = {:?}", bdk_transaction.node.tx);
            transactions.push(wallet.get_tx(bdk_transaction.node.txid, true).unwrap());
        }

        if transactions.len() > 0 {
            log::info!(
                "found some new transactions in new block! : {:?}",
                transactions
            );
            let new_txs = dw.update_transactions(transactions);
            if new_txs.len() > 0 {
                for tx in new_txs {
                    send_notifications_for_tx(&plugin, &wallet, tx).await?;
                }
            } else {
                log::info!("no new txs this time");
            }
        } else {
            log::info!("found no transactions");
        }
    }
    Ok(())
}
