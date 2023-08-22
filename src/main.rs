//! This is a plugin used to track a given descriptor
//! wallet or set of descriptor wallets, and send
//! events to other listening processes when coin movements are detected.
#[macro_use]
extern crate serde_json;

use bdk::bitcoin::Network;
use bdk::chain::keychain::LocalChangeSet;
use bdk::chain::{ConfirmationTime, ConfirmationTimeAnchor};
use bdk_esplora::{esplora_client, EsploraAsyncExt};
use bdk_file_store::Store;
use std::io::Write;
use std::sync::Arc;
use tokio::sync::Mutex;

use anyhow::Ok;
use watchdescriptor::wallet::DescriptorWallet;

use cln_plugin::{anyhow, messages, options, Builder, Error, Plugin};
use tokio;

use bdk::{bitcoin, KeychainKind, TransactionDetails, Wallet};
use watchdescriptor::state::WatchDescriptor;

const UTXO_DEPOSIT_TAG: &str = "utxo_deposit";
const UTXO_SPENT_TAG: &str = "utxo_spent";

const DB_MAGIC: &str = "bdk_wallet_esplora_async_example";
const STOP_GAP: usize = 50;
const PARALLEL_REQUESTS: usize = 5;

// #[tokio::main]
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), anyhow::Error> {
    let watch_descriptor = WatchDescriptor::new();
    // // watch_descriptor.add_descriptor("tr([af4c5952/86h/0h/0h]xpub6DTzDxFnUS1vriU7fc3VkwdTnArhk6FafoZHRcfwjRqo7vkMnbAiKK9AEhR4feqcdsE36Y4ZCLHBcEszJcvV3pMLhS4D9Ed5VNhH6Cw17Pp/0/*)".to_string()).await;
    // // watch_descriptor.add_descriptor("tr([af4c5952/86h/0h/0h]xpub6DTzDxFnUS1vriU7fc3VkwdTnArhk6FafoZHRcfwjRqo7vkMnbAiKK9AEhR4feqcdsE36Y4ZCLHBcEszJcvV3pMLhS4D9Ed5VNhH6Cw17Pp/1/*)".to_string()).await;
    let plugin_state = Arc::new(Mutex::new(watch_descriptor));
    if let Some(plugin) = Builder::new(tokio::io::stdin(), tokio::io::stdout())
    // if let Some(midstate) = Builder::new(tokio::io::stdin(), tokio::io::stdout())
    // let builder = Builder::new(tokio::io::stdin(), tokio::io::stdout())
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
        .dynamic()
        .start(plugin_state.clone())
        // .configure()
        .await?
    {
        // let midstate = if let Some(midstate) = builder.configure().await? {
        //     midstate
        // } else {
        //     return Ok(());
        // };
        log::info!("Configuration from CLN main daemon: {:?}", plugin.configuration());
        // let cln_network = midstate.configuration().network;
        let cln_network = plugin.configuration().network;
        let watch_descriptor = WatchDescriptor{
            wallets: vec![],
            // network: bitcoin::Network::Bitcoin,
            network: cln_network.parse::<bitcoin::Network>().unwrap(),
        };
        log::info!("Initial Plugin State: {:?}", watch_descriptor);
        // let plugin_state = Arc::new(Mutex::new(watch_descriptor));
        // let plugin = midstate.start(plugin_state).await?;
        plugin.join().await
    } else {
        Ok(())
    }
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

// assume we own some inputs and not others. this tx was generated collaboratively between our wallet and (an)other wallet(s). request manual intervention.
async fn shared_tx_notify<'a>(
    plugin: &Plugin<State>,
    wallet: &Wallet<Store<'a, LocalChangeSet<KeychainKind, ConfirmationTimeAnchor>>>,
    tx: &TransactionDetails,
) -> Result<(), Error> {
    log::error!("Error: shared txs not implemented");
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

async fn fetch_wallet<'a>(
    dw: &DescriptorWallet,
) -> Result<Wallet<Store<'a, LocalChangeSet<KeychainKind, ConfirmationTimeAnchor>>>, Error> {
    let db_path = std::env::temp_dir().join("bdk-esplora-async-example");
    let db = Store::<bdk::wallet::ChangeSet>::new_from_path(DB_MAGIC.as_bytes(), db_path)?;
    // let external_descriptor = "wpkh(tprv8ZgxMBicQKsPdy6LMhUtFHAgpocR8GC6QmwMSFpZs7h6Eziw3SpThFfczTDh5rW2krkqffa11UpX3XkeTTB2FvzZKWXqPY54Y6Rq4AQ5R8L/84'/0'/0'/0/*)";
    // mutinynet_descriptor = "wpkh(tprv8ZgxMBicQKsPdSAgthqLZ5ZWQkm5As4V3qNA5G8KKxGuqdaVVtBhytrUqRGPm4RxTktSdvch8JyUdfWR8g3ddrC49WfZnj4iGZN8y5L8NPZ/*)"
    let mutinynet_descriptor_ext = "wpkh(tprv8ZgxMBicQKsPdSAgthqLZ5ZWQkm5As4V3qNA5G8KKxGuqdaVVtBhytrUqRGPm4RxTktSdvch8JyUdfWR8g3ddrC49WfZnj4iGZN8y5L8NPZ/84'/0'/0'/0/*)";
    let mutinynet_descriptor_int = "wpkh(tprv8ZgxMBicQKsPdSAgthqLZ5ZWQkm5As4V3qNA5G8KKxGuqdaVVtBhytrUqRGPm4RxTktSdvch8JyUdfWR8g3ddrC49WfZnj4iGZN8y5L8NPZ/84'/0'/0'/1/*)";
    // let external_descriptor = "wpkh(tpubEBr4i6yk5nf5DAaJpsi9N2pPYBeJ7fZ5Z9rmN4977iYLCGco1VyjB9tvvuvYtfZzjD5A8igzgw3HeWeeKFmanHYqksqZXYXGsw5zjnj7KM9/*)";
    // let internal_descriptor = "wpkh(tprv8ZgxMBicQKsPdy6LMhUtFHAgpocR8GC6QmwMSFpZs7h6Eziw3SpThFfczTDh5rW2krkqffa11UpX3XkeTTB2FvzZKWXqPY54Y6Rq4AQ5R8L/84'/0'/0'/1/*)";

    // let external_descriptor = mutinynet_descriptor_ext;
    // let internal_descriptor = mutinynet_descriptor_int;
    let external_descriptor = dw.descriptor.clone();
    let internal_descriptor = dw.change_descriptor.clone();
    log::info!("about to create wallet");
    let mut wallet = Wallet::new(
        &external_descriptor,
        internal_descriptor.as_ref(),
        db,
        Network::Testnet,
    )?;

    // let address = wallet.get_address(AddressIndex::New);
    // log::info!("Generated Address: {}", address);

    let balance = wallet.get_balance();
    log::info!("Wallet balance before syncing: {} sats", balance.total());

    log::info!("Syncing...");
    let client =
        // esplora_client::Builder::new("https://blockstream.info/testnet/api").build_async()?;
        esplora_client::Builder::new("https://mutinynet.com/api").build_async()?;

    let local_chain = wallet.checkpoints();
    let keychain_spks = wallet
        .spks_of_all_keychains()
        .into_iter()
        .map(|(k, k_spks)| {
            let mut once = Some(());
            let mut stdout = std::io::stdout();
            let k_spks = k_spks
                .inspect(move |(spk_i, _)| match once.take() {
                    Some(_) => log::info!("\nScanning keychain [{:?}]", k),
                    None => log::info!(" {:<3}", spk_i),
                })
                .inspect(move |_| stdout.flush().expect("must flush"));
            (k, k_spks)
        })
        .collect();
    log::info!("CAG finished scanning");
    let update = client
        .scan(
            local_chain,
            keychain_spks,
            [],
            [],
            STOP_GAP,
            PARALLEL_REQUESTS,
        )
        .await?;
    wallet.apply_update(update)?;
    wallet.commit()?;

    let balance = wallet.get_balance();
    log::info!("Wallet balance after syncing: {} sats", balance.total());
    return Ok(wallet);
}

type State = Arc<Mutex<WatchDescriptor>>;
async fn watchdescriptor<'a>(
    plugin: Plugin<State>,
    v: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    let mut dw = DescriptorWallet::try_from(v.clone()).map_err(|x| anyhow!(x))?;
    log::info!("params = {:?}", dw);

    let wallet = fetch_wallet(&dw).await?;

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
    plugin.state().lock().await.add_descriptor_wallet(dw);
    log::info!("wallet added");
    let messasge = format!(
        "Wallet with checksum {} successfully added",
        // "Wallet successfully added",
        wallet.descriptor_checksum(bdk::KeychainKind::External)
    );
    log::info!("returning");
    Ok(json!(messasge))
}

async fn listdescriptors(
    plugin: Plugin<State>,
    _v: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    log::info!("HELLO LISTDESCRIPTORS");
    Ok(json!(plugin.state().lock().await.wallets))
}

async fn block_added_handler(plugin: Plugin<State>, v: serde_json::Value) -> Result<(), Error> {
    log::info!("Got a block_added notification: {}", v);
    log::info!(
        "WatchDescriptor state!!! {:?}",
        plugin.state().lock().await.wallets
    );

    let descriptor_wallets = &mut plugin.state().lock().await.wallets;
    for dw in descriptor_wallets.iter_mut() {
        let wallet = fetch_wallet(&dw).await?;
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
