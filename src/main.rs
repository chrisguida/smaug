//! This is a plugin used to track a given descriptor
//! wallet or set of descriptor wallets, and send
//! events to other listening processes when coin movements are detected.
#[macro_use]
extern crate serde_json;

use bitcoincore_rpc::bitcoincore_rpc_json::ScanBlocksRequest;
use clap::error::ErrorKind;
use clap::{CommandFactory, Parser, Subcommand};
use cln_rpc::model::DatastoreMode;
use cln_rpc::{
    model::requests::{DatastoreRequest, ListdatastoreRequest},
    ClnRpc, Request, Response,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

use anyhow::Ok;
use smaug::wallet::{AddArgs, DescriptorWallet, SMAUG_DATADIR, UTXO_DEPOSIT_TAG, UTXO_SPENT_TAG};

use cln_plugin::{anyhow, messages, options, Builder, Error, Plugin};
use tokio;

use bdk::TransactionDetails;
use smaug::state::{Smaug, State};

fn scanblocks<'a>(brpc_user: String, brpc_pass: String) -> Result<(), Error> {
    // let external_descriptor = "wpkh(tprv8ZgxMBicQKsPdy6LMhUtFHAgpocR8GC6QmwMSFpZs7h6Eziw3SpThFfczTDh5rW2krkqffa11UpX3XkeTTB2FvzZKWXqPY54Y6Rq4AQ5R8L/84'/0'/0'/0/*)";
    // mutinynet_descriptor = "wpkh(tprv8ZgxMBicQKsPdSAgthqLZ5ZWQkm5As4V3qNA5G8KKxGuqdaVVtBhytrUqRGPm4RxTktSdvch8JyUdfWR8g3ddrC49WfZnj4iGZN8y5L8NPZ/*)"
    let _mutinynet_descriptor_ext = "wpkh(tprv8ZgxMBicQKsPdSAgthqLZ5ZWQkm5As4V3qNA5G8KKxGuqdaVVtBhytrUqRGPm4RxTktSdvch8JyUdfWR8g3ddrC49WfZnj4iGZN8y5L8NPZ/84'/0'/0'/0/*)";
    let _mutinynet_descriptor_int = "wpkh(tprv8ZgxMBicQKsPdSAgthqLZ5ZWQkm5As4V3qNA5G8KKxGuqdaVVtBhytrUqRGPm4RxTktSdvch8JyUdfWR8g3ddrC49WfZnj4iGZN8y5L8NPZ/84'/0'/0'/1/*)";
    let _mutinynet_descriptor_ext_2 = "wpkh(tprv8ZgxMBicQKsPeRye8MhHA8hLxMuomycmGYXyRs7zViNck2VJsCJMTPt81Que8qp3PyPgQRnN7Gb1JyBVBKgj8AKEoEmmYxYDwzZJ63q1yjA/84'/0'/0'/0/*)";
    let _mutinynet_descriptor_int_2 = "wpkh(tprv8ZgxMBicQKsPeRye8MhHA8hLxMuomycmGYXyRs7zViNck2VJsCJMTPt81Que8qp3PyPgQRnN7Gb1JyBVBKgj8AKEoEmmYxYDwzZJ63q1yjA/84'/0'/0'/1/*)";

    extern crate bitcoincore_rpc;

    use bitcoincore_rpc::{Auth, Client, RpcApi};

    let brpc_host = "127.0.0.1";
    let brpc_port = 18443;

    let rpc = Client::new_with_timeout(
        &format!("http://{}:{}", brpc_host, brpc_port),
        Auth::UserPass(brpc_user, brpc_pass), // Auth::CookieFile(PathBuf::from("/home/cguida/.bitcoin/regtest/.cookie"))
        Duration::from_secs(3600),
    )
    .unwrap();
    let descriptor = ScanBlocksRequest::Extended {
        desc: _mutinynet_descriptor_ext.to_string(),
        range: None,
    };
    let descriptors = &[descriptor];
    let res = rpc.scan_blocks_blocking(descriptors);
    log::info!("scanblocks result: {:?}", res.unwrap());

    return Ok(());
}

#[tokio::main]
// #[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), anyhow::Error> {
    let builder = Builder::new(tokio::io::stdin(), tokio::io::stdout())
        .option(options::ConfigOption::new(
            "smaug_network",
            options::Value::OptString,
            "Which network to use: [bitcoin, testnet, signet, regtest, mutinynet]",
        ))
        .option(options::ConfigOption::new(
            "smaug_brpc_user",
            options::Value::OptString,
            "Bitcoind RPC user (Required)",
        ))
        .option(options::ConfigOption::new(
            "smaug_brpc_pass",
            options::Value::OptString,
            "Bitcoind RPC password (Required)",
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
    log::debug!(
        "Configuration from CLN main daemon: {:?}",
        configured_plugin.configuration()
    );
    log::debug!(
        "smaug_network = {:?}, cln_network = {}",
        configured_plugin.option("smaug_network"),
        configured_plugin.configuration().network
    );
    let network = match configured_plugin.option("smaug_network") {
        Some(smaug_network) => match smaug_network.as_str() {
            Some(wdn) => wdn.to_owned(),
            None => configured_plugin.configuration().network,
        },
        None => configured_plugin.configuration().network,
    };
    let brpc_user = match configured_plugin.option("smaug_brpc_user") {
        Some(smaug_brpc_user) => match smaug_brpc_user.as_str() {
            Some(wdn) => wdn.to_owned(),
            None => return Err(anyhow!("must specify smaug_brpc_user")),
        },
        None => {
            return Err(anyhow!(
                "must specify smaug_brpc_user (your bitcoind instance rpcuser)"
            ))
        }
    };
    let brpc_pass = match configured_plugin.option("smaug_brpc_pass") {
        Some(smaug_brpc_pass) => match smaug_brpc_pass.as_str() {
            Some(wdn) => wdn.to_owned(),
            None => {
                return Err(anyhow!(
                    "must specify smaug_brpc_pass (your bitcoind instance rpcpassword)"
                ))
            }
        },
        None => return Err(anyhow!("must specify smaug_brpc_user")),
    };
    let ln_dir: PathBuf = configured_plugin.configuration().lightning_dir.into();
    // Create data dir if it does not exist
    fs::create_dir_all(ln_dir.join(SMAUG_DATADIR)).unwrap_or_else(|e| {
        log::error!("Cannot create data dir: {e:?}");
        std::process::exit(1);
    });
    log::debug!("network = {}", network);
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
    let watch_descriptor = Smaug {
        wallets,
        network: network.clone(),
        brpc_user: brpc_user.clone(),
        brpc_pass: brpc_pass.clone(),
        db_dir: ln_dir.join(SMAUG_DATADIR),
    };
    let plugin_state = Arc::new(Mutex::new(watch_descriptor.clone()));
    plugin_state.lock().await.network = network;
    let plugin = configured_plugin.start(plugin_state).await?;
    log::info!("Smaug started");
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
            log::trace!("matches = {:?}", cli);
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
                log::trace!("{}", help_json);
                return Ok(json!(help_json));
            }
            _ => {
                let error_json = json!({
                    "error_message": e.to_string(),
                    "format-hint": "simple",
                });
                log::trace!("args = {:?}", v);
                log::trace!("{}", error_json);
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
    log::trace!("params = {:?}", dw);
    let wallet = dw
        .fetch_wallet(plugin.state().lock().await.db_dir.clone())
        .await?;
    let bdk_transactions_iter = wallet.transactions();
    let mut transactions = Vec::<TransactionDetails>::new();
    for bdk_transaction in bdk_transactions_iter {
        log::trace!("BDK transaction = {:?}", bdk_transaction.node.tx);
        transactions.push(wallet.get_tx(bdk_transaction.node.txid, true).unwrap());
    }

    if transactions.len() > 0 {
        log::trace!("found some transactions: {:?}", transactions);
        let new_txs = dw.update_transactions(transactions);
        if new_txs.len() > 0 {
            for tx in new_txs {
                log::trace!("new tx found!: {:?}", tx);
                dw.send_notifications_for_tx(&plugin, &wallet, tx).await?;
            }
        } else {
            log::debug!("no new txs this time");
        }
    }
    log::trace!("waiting for wallet lock");
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
    log::trace!("wallet added");
    let message = format!(
        "Wallet with deterministic name {} successfully added",
        &dw.get_name()?
    );
    log::trace!("returning");
    Ok(json!(message))
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct ListResponseItem {
    pub descriptor: String,
    pub change_descriptor: Option<String>,
    pub birthday: Option<u32>,
    pub gap: Option<u32>,
    pub network: Option<String>,
}

async fn list(
    plugin: Plugin<State>,
    // _v: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    let state = &plugin.state().lock().await;
    let brpc_user = state.brpc_user.clone();
    let brpc_pass = state.brpc_pass.clone();
    let wallets = state.wallets.clone();
    scanblocks(brpc_user, brpc_pass)?;
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
    log::trace!("Got a block_added notification: {}", v);
    log::trace!("Smaug state!!! {:?}", plugin.state().lock().await.wallets.clone());


    log::trace!("waiting for db_dir lock in block_handler");
    let db_dir = {
        let state = plugin.state().lock().await;
        state.db_dir.clone()
    };

    log::trace!("waiting for wallet lock in block_handler");
    let state = &mut plugin.state().lock().await;
    let descriptor_wallets = &mut state.wallets;

    log::trace!("db_dir in block_handler: {:?}", &db_dir);
    log::trace!("acquired wallet lock in block_handler");
    for (_dw_desc, dw) in descriptor_wallets.iter_mut() {
        log::trace!("fetching wallet in block_handler: {:?}", dw);

        let wallet = dw
        .fetch_wallet(db_dir.clone())
        .await?;
        log::trace!("...fetched wallet in block_handler");
        let bdk_transactions_iter = wallet.transactions();
        let mut transactions = Vec::<TransactionDetails>::new();
        for bdk_transaction in bdk_transactions_iter {
            log::trace!("BDK transaction = {:?}", bdk_transaction.node.tx);
            transactions.push(wallet.get_tx(bdk_transaction.node.txid, true).unwrap());
        }

        if transactions.len() > 0 {
            log::trace!(
                "found some new transactions in new block! : {:?}",
                transactions
            );
            let new_txs = dw.update_transactions(transactions);
            if new_txs.len() > 0 {
                for tx in new_txs {
                    dw.send_notifications_for_tx(&plugin, &wallet, tx).await?;
                }
            } else {
                log::debug!("no new txs this time");
            }
        } else {
            log::debug!("found no transactions");
        }
    }
    log::trace!("returning from block_added_handler");
    Ok(())
}
