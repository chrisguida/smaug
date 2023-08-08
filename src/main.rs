//! This is a plugin used to track a given descriptor
//! wallet or set of descriptor wallets, and send
//! events to other listening processes when coin movements are detected.
#[macro_use]
extern crate serde_json;

use bdk::bitcoin::util::bip32::{self, ExtendedPrivKey};
use bdk::bitcoin::{Address, Network};
use bdk::template::Bip84;
use bdk::wallet::AddressIndex;
use bdk_esplora::{esplora_client, EsploraAsyncExt};
use bdk_file_store::Store;
use std::fmt::Debug;
use std::io::Write;
use std::str::FromStr;
use std::sync::Arc;
use std::thread::{self, sleep};
use std::time;
use tokio::sync::Mutex;

use anyhow::Ok;
use watchdescriptor::wallet::DescriptorWallet;

use cln_plugin::{anyhow, messages, options, Builder, Error, Plugin};
use tokio;

use bdk::{bitcoin, descriptor, KeychainKind, SignOptions, TransactionDetails, Wallet};
use watchdescriptor::state::WatchDescriptor;

const UTXO_DEPOSIT_TAG: &str = "utxo_deposit";
const UTXO_SPENT_TAG: &str = "utxo_spent";

const DB_MAGIC: &str = "bdk_wallet_esplora_async_example";
const SEND_AMOUNT: u64 = 5000;
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
    // if tx.sent > 0 {
    //     log::info!("sending spend notif");
    //     send_spend_notification(plugin, &tx).await?;
    // }

    if tx.received > 0 {
        log::info!("sending deposit notif");
        send_deposit_notification(plugin, &tx).await?;
    }
    Ok(())
}

type State = Arc<Mutex<WatchDescriptor>>;

// async fn watchdescriptor(
//     plugin: Plugin<State>,
//     v: serde_json::Value,
// ) -> Result<serde_json::Value, Error> {
//     let mut dw = DescriptorWallet::try_from(v.clone()).map_err(|x| anyhow!(x))?;
//     log::info!("params = {:?}", dw);

//     let mut transactions = Vec::<TransactionDetails>::new();
//     {
//         let wallet = Wallet::new(
//             &dw.descriptor,
//             dw.change_descriptor.as_ref(),
//             bitcoin::Network::Testnet,
//             MemoryDatabase::default(),
//         )?;
//         // holding onto db during later sync call causes RefCell to be carried across await causing runtime panic
//         {
//             let db = wallet.database();
//             let sync_time = db.get_sync_time()?;
//             match sync_time {
//                 Some(st) => {
//                     log::info!("found previous sync time: {:?}", st);
//                 }
//                 None => {
//                     log::info!("no previous sync time found");
//                 }
//             }
//         }
//         log::info!("creating client");
//         let client = Client::new("ssl://electrum.blockstream.info:60002")?;
//         log::info!("creating blockchain");
//         let blockchain = ElectrumBlockchain::from(client);
//         log::info!("syncing wallet");
//         wallet.sync(&blockchain, SyncOptions::default())?;

//         log::info!("retrieving sync time");
//         let db = wallet.database();
//         if let Some(st) = db.get_sync_time()? {
//             log::info!("new previous sync time: {:?}", st);
//             dw.update_last_synced(st.block_time)
//         } else {
//             log::info!("no previous sync time found, even after sync");
//         }

//         transactions = wallet.list_transactions(false)?;
//     }
//     if transactions.len() > 0 {
//         log::info!("found some transactions: {:?}", transactions);
//         let new_txs = dw.update_transactions(transactions);
//         if new_txs.len() > 0 {
//             for tx in new_txs {
//                 log::info!("new tx found!: {:?}", tx);
//                 send_notifications_for_tx(&plugin, tx).await?;
//             }
//         } else {
//             log::info!("no new txs this time");
//         }
//     }
//     log::info!("waiting for wallet lock");
//     plugin.state().lock().await.add_descriptor_wallet(dw);
//     log::info!("wallet added");
//     let message = format!(
//         // "Wallet with checksum {} successfully added",
//         "Wallet successfully added",
//         // wallet.descriptor_checksum(bdk::KeychainKind::External)
//     );
//     // let ten_millis = time::Duration::from_secs(30);
//     // thread::sleep(ten_millis);
//     log::info!("returning");
//     Ok(json!(message))
// }

// async fn watchdescriptor(
//     plugin: Plugin<State>,
//     v: serde_json::Value,
// ) -> Result<serde_json::Value, Error> {
//     let mut dw = DescriptorWallet::try_from(v.clone()).map_err(|x| anyhow!(x))?;
//     log::info!("params = {:?}", dw);

//     let mut transactions = Vec::<TransactionDetails>::new();
//     {
//         // let wallet = Wallet::new(
//         //     &dw.descriptor,
//         //     dw.change_descriptor.as_ref(),
//         //     bitcoin::Network::Testnet,
//         //     MemoryDatabase::default(),
//         // )?;
//         // // holding onto db during later sync call causes RefCell to be carried across await causing runtime panic
//         // {
//         //     let db = wallet.database();
//         //     let sync_time = db.get_sync_time()?;
//         //     match sync_time {
//         //         Some(st) => {
//         //             log::info!("found previous sync time: {:?}", st);
//         //         }
//         //         None => {
//         //             log::info!("no previous sync time found");
//         //         }
//         //     }
//         // }
//         // log::info!("creating client");
//         // let client = Client::new("ssl://electrum.blockstream.info:60002")?;
//         // log::info!("creating blockchain");
//         // let blockchain = EsploraBlockchain::from(client);
//         // log::info!("syncing wallet");
//         // wallet.sync(&blockchain, SyncOptions::default())?;

//         let network = Network::Signet;

//         let xpriv = "tprv8ZgxMBicQKsPcx5nBGsR63Pe8KnRUqmbJNENAfGftF3yuXoMMoVJJcYeUw5eVkm9WBPjWYt6HMWYJNesB5HaNVBaFc1M6dRjWSYnmewUMYy";
//         let xpriv = bip32::ExtendedPrivKey::from_str(xpriv).unwrap();

//         // let esplora_url = "https://explorer.bc-2.jp/api";
//         // let esplora_url = "https://blockstream.info/testnet/api/";
//         let esplora_url = "https://mutinynet.com/api/";

//         let blockchain = EsploraBlockchain::new(esplora_url, 20);

//         let wallet = create_wallet(&network, &xpriv);

//         wallet
//             .sync(&blockchain, SyncOptions::default())
//             .await
//             .unwrap();

//         log::info!("retrieving sync time");
//         let db = wallet.database();
//         if let Some(st) = db.get_sync_time()? {
//             log::info!("new previous sync time: {:?}", st);
//             dw.update_last_synced(st.block_time)
//         } else {
//             log::info!("no previous sync time found, even after sync");
//         }

//         transactions = wallet.list_transactions(false)?;
//     }
//     if transactions.len() > 0 {
//         log::info!("found some transactions: {:?}", transactions);
//         let new_txs = dw.update_transactions(transactions);
//         if new_txs.len() > 0 {
//             for tx in new_txs {
//                 log::info!("new tx found!: {:?}", tx);
//                 send_notifications_for_tx(&plugin, tx).await?;
//             }
//         } else {
//             log::info!("no new txs this time");
//         }
//     }
//     log::info!("waiting for wallet lock");
//     plugin.state().lock().await.add_descriptor_wallet(dw);
//     log::info!("wallet added");
//     let messasge = format!(
//         // "Wallet with checksum {} successfully added",
//         "Wallet successfully added",
//         // wallet.descriptor_checksum(bdk::KeychainKind::External)
//     );
//     // let ten_millis = time::Duration::from_secs(30);
//     // thread::sleep(ten_millis);
//     log::info!("returning");
//     Ok(json!(messasge))
// }

async fn watchdescriptor(
    plugin: Plugin<State>,
    v: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    log::info!("HELLO watchdescriptor");
    let db_path = std::env::temp_dir().join("bdk-esplora-async-example");
    let db = Store::<'_, bdk::wallet::ChangeSet>::new_from_path(DB_MAGIC.as_bytes(), db_path)?;
    let external_descriptor = "wpkh(tprv8ZgxMBicQKsPdy6LMhUtFHAgpocR8GC6QmwMSFpZs7h6Eziw3SpThFfczTDh5rW2krkqffa11UpX3XkeTTB2FvzZKWXqPY54Y6Rq4AQ5R8L/84'/0'/0'/0/*)";
    let internal_descriptor = "wpkh(tprv8ZgxMBicQKsPdy6LMhUtFHAgpocR8GC6QmwMSFpZs7h6Eziw3SpThFfczTDh5rW2krkqffa11UpX3XkeTTB2FvzZKWXqPY54Y6Rq4AQ5R8L/84'/0'/0'/1/*)";

    let mut wallet = Wallet::new(
        external_descriptor,
        Some(internal_descriptor),
        db,
        Network::Testnet,
    )?;

    let address = wallet.get_address(AddressIndex::New);
    // println!("Generated Address: {}", address);
    log::info!("Generated Address: {}", address);

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
    let handle = tokio::runtime::Handle::try_current()?;
    let update = std::thread::scope(|scope| {
        scope
            .spawn(|| {
                handle.block_on(client.scan(
                    local_chain,
                    keychain_spks,
                    [],
                    [],
                    STOP_GAP,
                    PARALLEL_REQUESTS,
                ))
            })
            .join()
        // join() only returns an Err if scan panicked.
    })
    .expect("Propagating scan() panic")?;
    // log::info!();
    wallet.apply_update(update)?;
    wallet.commit()?;

    let balance = wallet.get_balance();
    log::info!("Wallet balance after syncing: {} sats", balance.total());

    if balance.total() < SEND_AMOUNT {
        log::info!(
            "Please send at least {} sats to the receiving address",
            SEND_AMOUNT
        );
        std::process::exit(0);
    }

    let faucet_address = Address::from_str("mkHS9ne12qx9pS9VojpwU5xtRd4T7X7ZUt")?;
    let mut psbt;
    {
        let mut tx_builder = wallet.build_tx();
        tx_builder
            .add_recipient(faucet_address.script_pubkey(), SEND_AMOUNT)
            .enable_rbf();

        (psbt, _) = tx_builder.finish()?;
    }
    let finalized = wallet.sign(&mut psbt, SignOptions::default())?;
    assert!(finalized);

    let tx = psbt.extract_tx();
    client.broadcast(&tx).await?;
    log::info!("Tx broadcasted! Txid: {}", tx.txid());
    Ok(json!("Wallet sucessfully added"))
    // Ok(())
}

async fn listdescriptors(
    plugin: Plugin<State>,
    _v: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    log::info!("HELLO LISTDESCRIPTORS");
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

    let descriptor_wallets = &mut plugin.state().lock().await.wallets;
    // let client = Client::new("ssl://electrum.blockstream.info:60002")?;
    // let blockchain = ElectrumBlockchain::from(client);
    let mut transactions = Vec::<TransactionDetails>::new();
    // for dw in descriptor_wallets.iter_mut() {
    //     // for dw in plugin.state().lock().await.wallets.iter_mut() {
    //     // for i in 0..1 {
    //     // let wallet;
    //     {
    //         let wallet = Wallet::new(
    //             &dw.descriptor,
    //             dw.change_descriptor.as_ref(),
    //             // "wpkh(tpubEBr4i6yk5nf5DAaJpsi9N2pPYBeJ7fZ5Z9rmN4977iYLCGco1VyjB9tvvuvYtfZzjD5A8igzgw3HeWeeKFmanHYqksqZXYXGsw5zjnj7KM9/*)",
    //             // None,
    //             // bitcoin::Network::Bitcoin,
    //             bitcoin::Network::Testnet,
    //             MemoryDatabase::default(),
    //         )?;
    //         // wallet.sync(&blockchain, SyncOptions::default())?;
    //         transactions = wallet.list_transactions(false)?;
    //     }
    //     if transactions.len() > 0 {
    //         log::info!("found some transactions: {:?}", transactions);
    //         for tx in transactions {
    //             send_notifications_for_tx(&plugin, tx).await?;
    //         }
    //         // let new_txs = plugin
    //         //     .state()
    //         //     .lock()
    //         //     .await
    //         //     .update_transactions(transactions);
    //         // if new_txs.len() > 0 {
    //         //     for tx in new_txs {
    //         //         send_notifications_for_tx(&plugin, tx).await?;
    //         //     }
    //         // } else {
    //         //     log::info!("no new txs this time");
    //         // }
    //     } else {
    //         log::info!("found no transactions");
    //     }
    // }

    // let acct = "test account";
    // let transfer_from: Option<String> = None;
    // let amount = 1000;
    // let outpoint = "a18b557b03f2b2d0e25430ef75b70ff5b6bd1f4dd19da3a564502b92623cd8a5:0";
    // let onchain_deposit = json!({
    //     "account": acct,
    //     "transfer_from": transfer_from,
    //     "outpoint": outpoint,
    //     "amount_msat": amount,
    //     "coin_type": "bcrt",
    //     "timestamp": 1679955976,
    //     "blockheight": 111,
    // });
    // log::info!("sending test notification for SANITY");
    // plugin
    //     .send_custom_notification(UTXO_DEPOSIT_TAG.to_string(), onchain_deposit)
    //     .await?;
    Ok(())
}
