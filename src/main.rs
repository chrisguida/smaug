//! This is a plugin used to track a given descriptor
//! wallet or set of descriptor wallets, and send
//! events to other listening processes when coin movements are detected.
#[macro_use]
extern crate serde_json;

use bdk::chain::tx_graph::CanonicalTx;
use bdk::chain::ConfirmationTimeAnchor;
use bitcoincore_rpc::{Auth, Client, RpcApi};
use clap::error::ErrorKind;
use clap::{CommandFactory, Parser, Subcommand};
use cln_plugin::options::{
    ConfigOption, DefaultStringConfigOption, IntegerConfigOption, StringConfigOption,
};
use cln_rpc::model::requests::DatastoreMode;
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
use tokio::sync::Mutex;

use smaug::brpc_auth::{detect_brpc_config, DetectionResult};
use smaug::wallet::{AddArgs, DescriptorWallet, SMAUG_DATADIR, UTXO_DEPOSIT_TAG, UTXO_SPEND_TAG};

use cln_plugin::{anyhow, messages, Builder, Error, Plugin};
use tokio;

use bdk::bitcoin::Transaction;
use smaug::state::{Smaug, State};

const OPT_SMAUG_NETWORK: StringConfigOption = ConfigOption::new_str_no_default(
    "smaug_network",
    "Which network to use: [bitcoin, testnet, signet, regtest, mutinynet]",
);
const OPT_SMAUG_BRPC_HOST: DefaultStringConfigOption = ConfigOption::new_str_with_default(
    "smaug_brpc_host",
    "127.0.0.1",
    "Bitcoind RPC host (default 127.0.0.1)",
);
const OPT_SMAUG_BRPC_PORT: IntegerConfigOption =
    ConfigOption::new_i64_no_default("smaug_brpc_port", "Bitcoind RPC port");
const OPT_SMAUG_BRPC_USER: StringConfigOption = ConfigOption::new_str_no_default(
    "smaug_brpc_user",
    "Bitcoind RPC user (Required if cookie file unavailable)",
);
const OPT_SMAUG_BRPC_PASS: StringConfigOption = ConfigOption::new_str_no_default(
    "smaug_brpc_pass",
    "Bitcoind RPC password (Required if cookie file unavailable)",
);
const OPT_SMAUG_BRPC_COOKIE_DIR: StringConfigOption = ConfigOption::new_str_no_default(
    "smaug_brpc_cookie_dir",
    "Bitcoind data directory (for cookie file access)",
);

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // std::env::set_var("CLN_PLUGIN_LOG", "cln_plugin=info,cln_rpc=info,debug");
    eprintln!("STARTING SMAUG");
    eprintln!(
        "log set to {}",
        std::env::var("CLN_PLUGIN_LOG").unwrap_or_default()
    );
    let builder = Builder::new(tokio::io::stdin(), tokio::io::stdout())
        .option(OPT_SMAUG_NETWORK)
        .option(OPT_SMAUG_BRPC_HOST)
        .option(OPT_SMAUG_BRPC_PORT)
        .option(OPT_SMAUG_BRPC_USER)
        .option(OPT_SMAUG_BRPC_PASS)
        .option(OPT_SMAUG_BRPC_COOKIE_DIR)
        .notification(messages::NotificationTopic::new(UTXO_DEPOSIT_TAG))
        .notification(messages::NotificationTopic::new(UTXO_SPEND_TAG))
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
    let cln_network = configured_plugin.configuration().network.clone();
    if log::log_enabled!(log::Level::Debug) {
        eprintln!(
            "Configuration from CLN main daemon: {:?}",
            configured_plugin.configuration()
        );
        eprintln!(
            "smaug_network = {:?}, cln_network = {}",
            configured_plugin.option(&OPT_SMAUG_NETWORK).unwrap(),
            &cln_network,
        );
    }
    let network = match configured_plugin.option(&OPT_SMAUG_NETWORK).unwrap() {
        Some(smaug_network) => smaug_network.as_str().to_owned(),
        None => cln_network.clone(),
    };

    // Get plugin options for credential detection
    let brpc_host = configured_plugin.option(&OPT_SMAUG_BRPC_HOST).unwrap();
    let brpc_port_opt = configured_plugin.option(&OPT_SMAUG_BRPC_PORT).unwrap();
    let brpc_user_opt = configured_plugin.option(&OPT_SMAUG_BRPC_USER).unwrap();
    let brpc_pass_opt = configured_plugin.option(&OPT_SMAUG_BRPC_PASS).unwrap();
    let brpc_cookie_dir_opt = configured_plugin.option(&OPT_SMAUG_BRPC_COOKIE_DIR).unwrap();

    let rpc_file = configured_plugin.configuration().rpc_file.clone();
    let rpc_path = Path::new(&rpc_file);

    // Detect bitcoind RPC credentials using the new auto-detection module
    let detection_result = detect_brpc_config(
        &brpc_host,
        brpc_port_opt,
        brpc_user_opt,
        brpc_pass_opt,
        brpc_cookie_dir_opt,
        &network,
        rpc_path,
    )
    .await?;

    // Handle configured vs unconfigured state
    let (brpc_port, brpc_auth, configured, unconfigured_message) = match detection_result {
        DetectionResult::Configured(config) => {
            if log::log_enabled!(log::Level::Debug) {
                eprintln!("using auth info: {:?}", config.auth);
            }

            // Verify connection to bitcoind
            let rpc_client = Client::new(
                &format!("http://{}:{}", config.host, config.port),
                config.auth.clone(),
            )?;

            match rpc_client.get_connection_count() {
                Ok(_) => {
                    log::info!("Successfully connected to bitcoind");
                }
                Err(e) => {
                    log::warn!(
                        "Cannot connect to bitcoind: {}. \
                        Ensure bitcoind is running and credentials are correct.",
                        e
                    );
                    // Still allow plugin to start, but mark as unconfigured
                    let msg = format!(
                        "Cannot connect to bitcoind: {}. \
                        Please ensure bitcoind is running and accepting RPC connections.",
                        e
                    );
                    return Ok(start_unconfigured(
                        configured_plugin,
                        network,
                        brpc_host,
                        msg,
                    )
                    .await?);
                }
            }

            (config.port, config.auth, true, None)
        }
        DetectionResult::Unconfigured(message) => {
            log::warn!("Starting smaug in unconfigured mode - no bitcoind credentials found");
            return Ok(start_unconfigured(
                configured_plugin,
                network,
                brpc_host,
                message,
            )
            .await?);
        }
    };

    let ln_dir: PathBuf = configured_plugin.configuration().lightning_dir.into();
    // Create data dir if it does not exist
    fs::create_dir_all(ln_dir.join(SMAUG_DATADIR)).unwrap_or_else(|e| {
        log::error!("Cannot create data dir: {e:?}");
        std::process::exit(1);
    });
    log::trace!("network = {}", network);
    let rpc_file = configured_plugin.configuration().rpc_file;
    let p = Path::new(&rpc_file);

    let mut rpc = ClnRpc::new(p).await?;
    log::trace!("calling listdatastore");

    let lds_response = rpc
        .call(Request::ListDatastore(ListdatastoreRequest {
            key: Some(vec!["smaug".to_owned()]),
        }))
        .await
        .map_err(|e| anyhow!("Error calling listdatastore: {:?}", e))?;
    log::trace!("fetching wallets from listdatastore response");
    let wallets: BTreeMap<String, DescriptorWallet> = match lds_response {
        Response::ListDatastore(r) => match r.datastore.is_empty() {
            true => BTreeMap::new(),
            false => match &r.datastore[0].string {
                Some(deserialized) => match serde_json::from_str(&deserialized) {
                    core::result::Result::Ok(dws) => dws,
                    core::result::Result::Err(e) => {
                        // sometimes log::error! doesn't execute before plugin is killed, so we use eprintln! here instead
                        eprintln!(
                            "Error parsing wallet from datastore: {:?}",
                            &r.datastore[0].string
                        );
                        eprintln!("{}", e);
                        eprintln!("This is probably due to an outdated wallet format.");
                        eprintln!("Please delete the wallet with `lightning-cli deldatastore smaug` and restart Smaug.");
                        return Err(e.into());
                    }
                },
                None => BTreeMap::new(),
            },
        },
        _ => panic!("Unrecognized type returned from listdatastore call, exiting"),
    };
    log::trace!("creating plugin state");
    let watch_descriptor = Smaug {
        wallets,
        network: network.clone(),
        brpc_host: brpc_host.clone(),
        brpc_port,
        brpc_auth: brpc_auth.clone(),
        db_dir: ln_dir.join(SMAUG_DATADIR),
        configured,
        unconfigured_message,
    };
    let plugin_state = Arc::new(Mutex::new(watch_descriptor.clone()));
    log::trace!("getting lock on state");

    plugin_state.lock().await.network = network;
    log::trace!("starting Smaug");

    let plugin = configured_plugin.start(plugin_state).await?;
    log::info!("Smaug started");
    plugin.join().await
}

/// Starts the plugin in unconfigured mode with no bitcoind connection.
/// RPC calls will return helpful error messages until properly configured.
async fn start_unconfigured(
    configured_plugin: cln_plugin::ConfiguredPlugin<State, tokio::io::Stdin, tokio::io::Stdout>,
    network: String,
    brpc_host: String,
    message: String,
) -> Result<(), anyhow::Error> {
    let ln_dir: PathBuf = configured_plugin.configuration().lightning_dir.into();

    // Create data dir if it does not exist
    fs::create_dir_all(ln_dir.join(SMAUG_DATADIR)).unwrap_or_else(|e| {
        log::error!("Cannot create data dir: {e:?}");
        std::process::exit(1);
    });

    let watch_descriptor = Smaug {
        wallets: BTreeMap::new(),
        network,
        brpc_host,
        brpc_port: 0,
        brpc_auth: Auth::None,
        db_dir: ln_dir.join(SMAUG_DATADIR),
        configured: false,
        unconfigured_message: Some(message),
    };

    let plugin_state = Arc::new(Mutex::new(watch_descriptor));
    log::info!("Smaug started in UNCONFIGURED mode - RPC calls will return setup instructions");

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
    // Check if plugin is configured before processing any command
    {
        let state = plugin.state().lock().await;
        if !state.configured {
            let message = state
                .unconfigured_message
                .clone()
                .unwrap_or_else(|| "Smaug is not configured with bitcoind credentials.".to_string());
            return Ok(json!({
                "error": "not_configured",
                "message": message,
                "format-hint": "simple",
            }));
        }
    }

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

async fn add(plugin: Plugin<State>, args: AddArgs) -> Result<serde_json::Value, Error> {
    let mut dw = DescriptorWallet::from_args(args, plugin.state().lock().await.network.clone())
        .map_err(|e| anyhow!("error parsing args: {}", e))?;
    log::trace!("params = {:?}", dw);
    let (db_dir, brpc_host, brpc_port, brpc_auth) = {
        let state = plugin.state().lock().await;
        (
            state.db_dir.clone(),
            // FIXME: actually use the RpcConnection struct instead of this nonsense
            state.brpc_host.clone(),
            state.brpc_port.clone(),
            state.brpc_auth.clone(),
        )
    };
    let mut dw_clone = dw.clone();
    let wallet = dw_clone
        .fetch_wallet(db_dir, brpc_host, brpc_port, brpc_auth)
        .await?;
    let bdk_transactions_iter = wallet.transactions();
    let mut transactions = Vec::<CanonicalTx<'_, Transaction, ConfirmationTimeAnchor>>::new();
    for bdk_transaction in bdk_transactions_iter {
        log::trace!("BDK transaction = {:?}", bdk_transaction.tx_node.tx);
        transactions.push(bdk_transaction);
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
    // FIXME: this is horrible, please find a better way to do this
    dw.update_last_synced(dw_clone.last_synced.unwrap());
    log::trace!("waiting for wallet lock");
    plugin.state().lock().await.add_descriptor_wallet(&dw)?;

    log::trace!("add_descriptor_wallet");
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
    let name = &dw.get_name()?;
    let message = format!("Wallet with deterministic name {} successfully added", name);
    log::info!("{}", message);
    Ok(json!({"name": name, "message": message}))
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct ListResponseItem {
    pub descriptor: String,
    pub change_descriptor: Option<String>,
    pub balance: u64,
    pub birthday: Option<u32>,
    pub gap: Option<u32>,
    pub network: Option<String>,
}

async fn list(plugin: Plugin<State>) -> Result<serde_json::Value, Error> {
    let state = &plugin.state().lock().await;

    let wallets = state.wallets.clone();
    let mut result = BTreeMap::<String, ListResponseItem>::new();
    let (db_dir, brpc_host, brpc_port, brpc_auth) = {
        (
            state.db_dir.clone(),
            state.brpc_host.clone(),
            state.brpc_port.clone(),
            state.brpc_auth.clone(),
        )
    };

    for (wallet_name, wallet) in wallets {
        let mut dw_clone = wallet.clone();
        let bdk_wallet = dw_clone
            .fetch_wallet(
                db_dir.clone(),
                brpc_host.clone(),
                brpc_port.clone(),
                brpc_auth.clone(),
            )
            .await?;

        result.insert(
            wallet_name.clone(),
            ListResponseItem {
                descriptor: wallet.descriptor.clone(),
                change_descriptor: wallet.change_descriptor.clone(),
                balance: bdk_wallet.get_balance().total(),
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
    descriptor_name: String,
) -> Result<serde_json::Value, Error> {
    let db_dir_path = plugin.state().lock().await.db_dir.clone();
    let wallets = &mut plugin.state().lock().await.wallets;

    let removed_item = wallets.remove(&descriptor_name);
    let db_file_path = match removed_item {
        Some(dw) => match dw.get_db_file_path(db_dir_path) {
            Ok(dw) => dw,
            Err(e) => return Err(e),
        },
        None => return Err(anyhow!("Can't find wallet '{}'.", descriptor_name)),
    };
    fs::remove_file(db_file_path.clone())?;
    log::debug!("Deleted smaug db file at {}", db_file_path);
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

    Ok(json!(format!("Deleted wallet: {}", descriptor_name)))
}

async fn block_added_handler(plugin: Plugin<State>, v: serde_json::Value) -> Result<(), Error> {
    log::trace!("Got a block_added notification: {}", v);

    // Skip processing if plugin is not configured
    {
        let state = plugin.state().lock().await;
        if !state.configured {
            log::trace!("Skipping block_added handler - plugin not configured");
            return Ok(());
        }
    }

    log::trace!(
        "Smaug state!!! {:?}",
        plugin.state().lock().await.wallets.clone()
    );
    let (db_dir, brpc_host, brpc_port, brpc_auth) = {
        let state = plugin.state().lock().await;
        (
            state.db_dir.clone(),
            state.brpc_host.clone(),
            state.brpc_port,
            state.brpc_auth.clone(),
        )
    };

    log::trace!("waiting for wallet lock in block_handler");
    let state = &mut plugin.state().lock().await;
    let descriptor_wallets = &mut state.wallets;

    log::trace!("db_dir in block_handler: {:?}", &db_dir);
    log::trace!("acquired wallet lock in block_handler");
    for (_dw_desc, dw) in descriptor_wallets.iter_mut() {
        log::trace!("fetching wallet in block_handler: {:?}", dw);

        let mut dw_clone = dw.clone();
        let wallet = dw_clone
            .fetch_wallet(
                db_dir.clone(),
                brpc_host.clone(),
                brpc_port.clone(),
                brpc_auth.clone(),
            )
            .await?;

        log::trace!("...fetched wallet in block_handler");
        let bdk_transactions_iter = wallet.transactions();
        let mut transactions = Vec::<CanonicalTx<'_, Transaction, ConfirmationTimeAnchor>>::new();
        for bdk_transaction in bdk_transactions_iter {
            log::trace!("BDK transaction = {:?}", bdk_transaction.tx_node.tx);
            transactions.push(bdk_transaction);
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
        log::debug!("scanned up to height {}", dw_clone.last_synced.unwrap());

        // FIXME: this is horrible, please find a better way to do this
        dw.update_last_synced(dw_clone.last_synced.unwrap());
    }
    log::trace!("returning from block_added_handler");
    Ok(())
}

// #[tokio::test]
// async fn test_list() {
//     let builder = Builder::new(tokio::io::stdin(), tokio::io::stdout())
//     .dynamic();
//     let plugin = Plugin {
//         /// The state gets cloned for each request
//         state: S,
//         /// "options" field of "init" message sent by cln
//         options: Vec<ConfigOption>,
//         /// "configuration" field of "init" message sent by cln
//         configuration: Configuration,
//         /// A signal that allows us to wait on the plugin's shutdown.
//         wait_handle: tokio::sync::broadcast::Sender<()>,

//         sender: tokio::sync::mpsc::Sender<serde_json::Value>,
//     };
//     println!("plugin = {:?}", plugin);
// }

// #[cfg(test)]
// mod tests {
//     use super::*;
//     use async_trait::async_trait;
//     use mockall::mock;
//     use std::sync::{Arc, Mutex};

//     // Mocking a simplified version of Wallet and State
//     #[derive(Clone)]
//     struct Wallet {
//         pub descriptor: String,
//         pub change_descriptor: Option<String>,
//         pub birthday: Option<u32>,
//         pub gap: Option<u32>,
//         pub network: Option<String>,
//     }

//     struct State {
//         pub wallets: BTreeMap<String, Wallet>,
//     }

//     #[async_trait]
//     pub trait StateHolder {
//         async fn state(&self) -> Arc<Mutex<State>>;
//     }

//     mock! {
//         Plugin {}

//         #[async_trait]
//         impl StateHolder for Plugin<State> {
//             async fn state(&self) -> Arc<Mutex<State>>;
//         }
//     }

//     #[tokio::test]
//     async fn test_list_function() {
//         // Setup mock state
//         let mut wallets = BTreeMap::new();
//         wallets.insert(
//             "wallet1".to_string(),
//             Wallet {
//                 descriptor: "descriptor1".to_string(),
//                 change_descriptor: Some("change1".to_string()),
//                 birthday: Some(123),
//                 gap: Some(5),
//                 network: Some("network1".to_string()),
//             },
//         );

//         let plugin = MockPlugin::new(wallets);

//         // Call the function
//         let result = list(plugin).await.unwrap();

//         // Assert the expected outcome
//         let expected_json = json!({
//             "wallet1": {
//                 "descriptor": "descriptor1",
//                 "change_descriptor": "change1",
//                 "birthday": 123,
//                 "gap": 5,
//                 "network": "network1"
//             }
//         });

//         assert_eq!(result, expected_json);
//     }
// }
