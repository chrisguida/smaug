//! This is a plugin used to track a given descriptor
//! wallet or set of descriptor wallets, and send
//! events to other listening processes when coin movements are detected.
#[macro_use]
extern crate serde_json;

use bdk::bitcoin::Network;
use clap::error::ErrorKind;
use clap::{arg, Command, Parser, Subcommand};
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
use std::path::{Path, PathBuf};
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
            "Stop watching a descriptor wallet",
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

#[derive(Debug, Deserialize, Serialize, Clone, Parser)]
#[command(author, version, about, long_about = None)]
pub struct WatchDescriptorArgs {
    pub descriptor: String,
    pub change_descriptor: Option<String>,
    pub birthday: Option<u32>,
    pub gap: Option<u32>,
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Optional name to operate on
    name: Option<String>,

    /// Sets a custom config file
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    /// Turn debugging information on
    #[arg(short, long, action = clap::ArgAction::Count)]
    debug: u8,
    // #[command(subcommand)]
    // command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// does testing things
    Test {
        /// lists test values
        #[arg(short, long)]
        list: bool,
    },
}

fn convert_to_args(json_value: &Value) -> Vec<String> {
    let mut args = vec![String::from("program_name")];

    if let Some(command) = json_value.get("command").and_then(|v| v.as_str()) {
        args.push(command.to_string());
    }

    if let Some(options) = json_value.get("options").and_then(|v| v.as_object()) {
        for (key, value) in options {
            let arg_key = format!("--{}", key);
            args.push(arg_key);

            if let Some(str_val) = value.as_str() {
                args.push(str_val.to_string());
            }
        }
    }

    args
}

fn to_os_string(v: Value) -> OsString {
    v.as_str().unwrap().to_owned().into()
}

async fn watchdescriptor<'a>(
    plugin: Plugin<State>,
    v: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    // log::info!("1");
    // let mut arg_vec = vec![json!("watchdescriptor")];
    // log::info!("2");
    let arg_vec = match v {
        serde_json::Value::Array(a) => a,
        _ => {
            return Err(anyhow!("only positional args supported. no keyword args."));
        }
    };

    // let matches = Command::new("watchdescriptor")
    //     // Args and options go here...
    //     .get_matches_from(arg_vec.iter().map(|x| to_os_string(x.clone())));

    // log::info!("arg_vec = {:?}", arg_vec);
    // log::info!(
    //     "converted arg_vec = {:?}",
    //     arg_vec
    //         .iter()
    //         .map(|x| to_os_string(x.clone()))
    //         .collect::<Vec<_>>()
    // );

    let mut command = Command::new("Watchdescriptor")
        .bin_name("lightning-cli watchdescriptor --") // for usage string; proper invocation to avoid passing args to lightning-cli
        .no_binary_name(true) // to avoid having clap cut off the first arg
        .author("chrisguida")
        .version("0.0.1")
        .about("Watchdescriptor: a Rust CLN plugin to monitor your treasury")
        .after_help(
            "Longer explanation to appear after the options when \
                    displaying the help information from --help or -h",
        )
        .subcommand(
            Command::new("add")
                .alias("watch")
                .about("Watch one or more external wallet descriptors and emit notifications when coins are moved")
                .arg(arg!(<config> "Required configuration file to use")),
        )
        .subcommand(
            Command::new("rm")
                .alias("remove")
                .alias("del")
                .alias("delete")
                .about("Stop watching a descriptor wallet")
                .arg(arg!(<config> "Required configuration file to use")),
        )
        .subcommand(
            Command::new("ls")
                .alias("list")
                .about("List descriptor wallets currently being watched")
                .arg(arg!(<config> "Required configuration file to use")),
        );

    let matches = command
        .clone()
        .try_get_matches_from(arg_vec.iter().map(|x| to_os_string(x.clone())));

    match matches {
        core::result::Result::Ok(res) => {
            // Handle the parsed arguments
            // ...
            log::info!("matches = {:?}", res);
            if !res.args_present() {
                let help_json = json!({
                    "help_msg": format!("\n{}", command.render_help()),
                    "format-hint": "simple",
                });
                return Ok(json!(help_json));
            }
            return Ok(json!(format!("{:?}", res)));
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
                log::info!("{}", error_json);
                return Ok(json!(error_json));
            }
        },
    }

    // if let Some(sub_m) = matches.subcommand_matches("subcommand") {
    //     if sub_m.is_present("flag1") {
    //         println!("Flag1 is set!");
    //     }
    //     if let Some(opt2_val) = sub_m.value_of("opt2") {
    //         println!("opt2 has value: {}", opt2_val);
    //     }
    // }

    // // You can see how many times a particular flag or argument occurred
    // // Note, only flags can have multiple occurrences
    // match cli.debug {
    //     0 => log::info!("Debug mode is off"),
    //     1 => log::info!("Debug mode is kind of on"),
    //     2 => log::info!("Debug mode is on"),
    //     _ => log::info!("Don't be crazy"),
    // }

    // You can check for the existence of subcommands, and if found use their
    // matches just as you would the top level cmd
    // match &cli.command {
    //     Some(Commands::Test { list }) => {
    //         if *list {
    //             log::info!("Printing testing lists...");
    //         } else {
    //             log::info!("Not printing testing lists...");
    //         }
    //     }
    //     None => {}
    // }

    // let mut dw = DescriptorWallet::try_from(v.clone()).map_err(|x| anyhow!(x))?;
    // dw.network = Some(plugin.state().lock().await.network.clone());
    // log::info!("params = {:?}", dw);

    // let wallet = dw.fetch_wallet().await?;
    // let bdk_transactions_iter = wallet.transactions();
    // let mut transactions = Vec::<TransactionDetails>::new();
    // for bdk_transaction in bdk_transactions_iter {
    //     log::info!("BDK transaction = {:?}", bdk_transaction.node.tx);
    //     transactions.push(wallet.get_tx(bdk_transaction.node.txid, true).unwrap());
    // }

    // if transactions.len() > 0 {
    //     log::info!("found some transactions: {:?}", transactions);
    //     let new_txs = dw.update_transactions(transactions);
    //     if new_txs.len() > 0 {
    //         for tx in new_txs {
    //             log::info!("new tx found!: {:?}", tx);
    //             dw.send_notifications_for_tx(&plugin, &wallet, tx).await?;
    //         }
    //     } else {
    //         log::info!("no new txs this time");
    //     }
    // }
    // log::info!("waiting for wallet lock");
    // plugin.state().lock().await.add_descriptor_wallet(&dw)?;

    // let wallets_str = json!(plugin.state().lock().await.wallets).to_string();
    // let rpc_file = plugin.configuration().rpc_file;
    // let p = Path::new(&rpc_file);

    // let mut rpc = ClnRpc::new(p).await?;
    // let _ds_response = rpc
    //     .call(Request::Datastore(DatastoreRequest {
    //         key: vec!["watchdescriptor".to_owned()],
    //         string: Some(wallets_str),
    //         hex: None,
    //         mode: Some(DatastoreMode::CREATE_OR_REPLACE),
    //         generation: None,
    //     }))
    //     .await
    //     .map_err(|e| anyhow!("Error calling listdatastore: {:?}", e))?;
    // log::info!("wallet added");
    // let message = format!(
    //     "Wallet with deterministic name {} successfully added",
    //     &dw.get_name()?
    // );
    // log::info!("returning");
    // Ok(json!(message))
    // Ok(json!("success"))
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
