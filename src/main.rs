//! This is a plugin used to track a given descriptor
//! wallet or set of descriptor wallets, and send
//! events to other listening processes when coin movements are detected.
#[macro_use]
extern crate serde_json;

use bdk::bitcoin::Network;
use cln_rpc::model::DatastoreMode;
use cln_rpc::{
    model::requests::{DatastoreRequest, ListdatastoreRequest},
    ClnRpc, Request, Response,
};
use home::home_dir;
use serde::{Deserialize, Serialize};

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

use anyhow::Ok;
use watchdescriptor::wallet::{DescriptorWallet, DATADIR, UTXO_DEPOSIT_TAG, UTXO_SPENT_TAG};

use cln_plugin::{anyhow, messages, options, Builder, Error, Plugin};
use tokio;

use bdk::{bitcoin, TransactionDetails};
use watchdescriptor::state::{State, WatchDescriptor};

#[tokio::main]
// #[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), anyhow::Error> {
    // Create data dir if it does not exist
    fs::create_dir_all(&home_dir().unwrap().join(DATADIR)).unwrap_or_else(|e| {
        log::error!("Cannot create data dir: {e:?}");
        std::process::exit(1);
    });
    let builder = Builder::new(tokio::io::stdin(), tokio::io::stdout())
        .option(options::ConfigOption::new(
            "wd_network",
            options::Value::OptString,
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
        .rpcmethod(
            "deletedescriptor",
            "Stop wathing a descriptor wallet",
            deletedescriptor,
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
    log::info!(
        "wd_network = {:?}, cln_network = {}",
        configured_plugin.option("wd_network"),
        configured_plugin.configuration().network
    );
    let network = match configured_plugin.option("wd_network") {
        Some(wd_network) => match wd_network.as_str() {
            Some(wdn) => wdn.to_owned(),
            None => configured_plugin.configuration().network,
        },
        None => configured_plugin.configuration().network,
    }
    .parse::<bitcoin::Network>()
    .unwrap();
    log::info!("network = {}", network);
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
    let watch_descriptor = WatchDescriptor { wallets, network };
    let plugin_state = Arc::new(Mutex::new(watch_descriptor.clone()));
    plugin_state.lock().await.network = network;
    let plugin = configured_plugin.start(plugin_state).await?;
    plugin.join().await
}

async fn watchdescriptor<'a>(
    plugin: Plugin<State>,
    v: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    let mut dw = DescriptorWallet::try_from(v.clone()).map_err(|x| anyhow!(x))?;
    dw.network = Some(plugin.state().lock().await.network.clone());
    log::info!("params = {:?}", dw);

    let wallet = dw.fetch_wallet().await?;
    let bdk_transactions_iter = wallet.transactions();
    let mut transactions = Vec::<TransactionDetails>::new();
    for bdk_transaction in bdk_transactions_iter {
        log::info!("BDK transaction = {:?}", bdk_transaction.node.tx);
        transactions.push(wallet.get_tx(bdk_transaction.node.txid, true).unwrap());
    }

    if transactions.len() > 0 {
        log::info!("found some transactions: {:?}", transactions);
        let new_txs = dw.update_transactions(transactions);
        if new_txs.len() > 0 {
            for tx in new_txs {
                log::info!("new tx found!: {:?}", tx);
                dw.send_notifications_for_tx(&plugin, &wallet, tx).await?;
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
        "Wallet with deterministic name {} successfully added",
        &dw.get_name()?
    );
    log::info!("returning");
    Ok(json!(message))
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct ListDescriptorsResponseWallet {
    pub descriptor: String,
    pub change_descriptor: Option<String>,
    pub birthday: Option<u32>,
    pub gap: Option<u32>,
    pub network: Option<Network>,
}

async fn listdescriptors(
    plugin: Plugin<State>,
    _v: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    let wallets = &plugin.state().lock().await.wallets;
    let mut result = BTreeMap::<String, ListDescriptorsResponseWallet>::new();
    for (wallet_name, wallet) in wallets {
        result.insert(
            wallet_name.clone(),
            ListDescriptorsResponseWallet {
                descriptor: wallet.descriptor.clone(),
                change_descriptor: wallet.change_descriptor.clone(),
                birthday: wallet.birthday.clone(),
                gap: wallet.gap.clone(),
                network: wallet.network.clone(),
            },
        );
    }
    Ok(json!(result))
}

async fn deletedescriptor(
    plugin: Plugin<State>,
    v: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    let descriptor_name = match v {
        serde_json::Value::Array(a) => match a.get(0) {
            Some(res) => match res.clone().as_str() {
                Some(r) => r.to_owned(),
                None => return Err(anyhow!("can't parse args")),
            },
            None => return Err(anyhow!("can't parse args")),
        },
        _ => return Err(anyhow!("can't parse args")),
    };
    let wallets = &mut plugin.state().lock().await.wallets;
    let _removed_item: Option<DescriptorWallet>;
    if wallets.contains_key(&descriptor_name) {
        _removed_item = wallets.remove(&descriptor_name);
        let rpc_file = plugin.configuration().rpc_file;
        let p = Path::new(&rpc_file);

        let mut rpc = ClnRpc::new(p).await?;
        let _ds_response = rpc
            .call(Request::Datastore(DatastoreRequest {
                key: vec!["watchdescriptor".to_owned()],
                string: Some(json!(wallets).to_string()),
                hex: None,
                mode: Some(DatastoreMode::CREATE_OR_REPLACE),
                generation: None,
            }))
            .await
            .map_err(|e| anyhow!("Error calling listdatastore: {:?}", e))?;
    } else {
        return Err(anyhow!("can't find wallet {}", descriptor_name));
    }

    Ok(json!(format!("Deleted wallet: {}", descriptor_name)))
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
                    dw.send_notifications_for_tx(&plugin, &wallet, tx).await?;
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
