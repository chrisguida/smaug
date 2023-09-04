//! This is a plugin used to track a given descriptor
//! wallet or set of descriptor wallets, and send
//! events to other listening processes when coin movements are detected.
#[macro_use]
extern crate serde_json;

use bdk::bitcoin::Network;
use clap::error::ErrorKind;
use clap::{arg, CommandFactory, Parser, Subcommand};
use cln_rpc::model::DatastoreMode;
use cln_rpc::{
    model::requests::{DatastoreRequest, ListdatastoreRequest},
    ClnRpc, Request, Response,
};
use home::home_dir;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

use anyhow::Ok;
use smaug::wallet::{AddArgs, DescriptorWallet, DATADIR, UTXO_DEPOSIT_TAG, UTXO_SPENT_TAG};

use cln_plugin::{anyhow, messages, options, Builder, Error, Plugin};
use tokio;

use bdk::{bitcoin, TransactionDetails};
use smaug::state::{Smaug, State};

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
            "smaug",
            "Watch one or more external wallet descriptors and emit notifications when coins are moved",
            parse_command,
        )
        .subscribe("block_added", block_added_handler)
        .dynamic();
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
            key: Some(vec!["smaug".to_owned()]),
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
    let watch_descriptor = Smaug { wallets, network };
    let plugin_state = Arc::new(Mutex::new(watch_descriptor.clone()));
    plugin_state.lock().await.network = network;
    let plugin = configured_plugin.start(plugin_state).await?;
    plugin.join().await
}

#[derive(Debug, Parser)]
#[command(
    name = "Smaug",
    bin_name = "lightning-cli smaug --",
    author = "chrisguida",
    version = "0.0.1",
    about = "Smaug: a Rust CLN plugin to monitor your treasury\n\nWatch one or more external wallet descriptors and emit notifications when coins are moved",
    long_about = None,
    no_binary_name = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Start watching a descriptor wallet
    #[command(alias = "watch")]
    Add(AddArgs),
    /// Stop watching a descriptor wallet
    #[command(alias = "del", alias = "delete", alias = "remove")]
    Rm {
        /// Deterministic name (concatenated checksums) of wallet to delete
        // #[arg(short, long)]
        descriptor_name: String,
    },
    /// List descriptor wallets currently being watched
    #[command(alias = "list")]
    Ls,
}

fn to_os_string(v: Value) -> OsString {
    v.as_str().unwrap().to_owned().into()
}

async fn parse_command(
    plugin: Plugin<State>,
    v: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    let arg_vec = match v.clone() {
        serde_json::Value::Array(a) => a,
        _ => {
            return Err(anyhow!("only positional args supported. no keyword args."));
        }
    };

    let matches = Cli::try_parse_from(arg_vec.iter().map(|x| to_os_string(x.clone())));

    match matches {
        core::result::Result::Ok(cli) => {
            log::info!("matches = {:?}", cli);
            match cli.command {
                Some(c) => match c {
                    Commands::Add(args) => return add(plugin, args).await,
                    Commands::Rm { descriptor_name } => {
                        return delete(plugin, descriptor_name).await
                    }
                    Commands::Ls => return list(plugin).await,
                },
                None => {
                    let help_json = json!({
                        "help_msg": format!("\n{}",  <Cli as CommandFactory>::command().render_help()),
                        "format-hint": "simple",
                    });
                    return Ok(json!(help_json));
                }
            }
        }
        core::result::Result::Err(e) => match e.kind() {
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => {
                let help_json = json!({
                    "help_msg": format!("\n{}", e.to_string()),
                    "format-hint": "simple",
                });
                log::info!("{}", help_json);
                return Ok(json!(help_json));
            }
            _ => {
                let error_json = json!({
                    "error_message": e.to_string(),
                    "format-hint": "simple",
                });
                log::info!("args = {:?}", v);
                log::info!("{}", error_json);
                return Ok(json!(error_json));
            }
        },
    }
}

async fn add(
    plugin: Plugin<State>,
    // v: serde_json::Value,
    args: AddArgs,
) -> Result<serde_json::Value, Error> {
    let mut dw = DescriptorWallet::from_args(args, plugin.state().lock().await.network.clone())
        .map_err(|e| anyhow!("error parsing args: {}", e))?;
    // dw.network = );
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
            key: vec!["smaug".to_owned()],
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
struct ListResponseItem {
    pub descriptor: String,
    pub change_descriptor: Option<String>,
    pub birthday: Option<u32>,
    pub gap: Option<u32>,
    pub network: Option<Network>,
}

async fn list(
    plugin: Plugin<State>,
    // _v: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    let wallets = &plugin.state().lock().await.wallets;
    let mut result = BTreeMap::<String, ListResponseItem>::new();
    for (wallet_name, wallet) in wallets {
        result.insert(
            wallet_name.clone(),
            ListResponseItem {
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

async fn delete(
    plugin: Plugin<State>,
    // v: serde_json::Value,
    descriptor_name: String,
) -> Result<serde_json::Value, Error> {
    let wallets = &mut plugin.state().lock().await.wallets;
    let _removed_item: Option<DescriptorWallet>;
    if wallets.contains_key(&descriptor_name) {
        _removed_item = wallets.remove(&descriptor_name);
        let rpc_file = plugin.configuration().rpc_file;
        let p = Path::new(&rpc_file);

        let mut rpc = ClnRpc::new(p).await?;
        let _ds_response = rpc
            .call(Request::Datastore(DatastoreRequest {
                key: vec!["smaug".to_owned()],
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
    log::info!("Smaug state!!! {:?}", plugin.state().lock().await.wallets);

    let descriptor_wallets = &mut plugin.state().lock().await.wallets;
    for (_dw_desc, dw) in descriptor_wallets.iter_mut() {
        let wallet = dw.fetch_wallet().await?;
        let bdk_transactions_iter = wallet.transactions();
        let mut transactions = Vec::<TransactionDetails>::new();
        for bdk_transaction in bdk_transactions_iter {
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
