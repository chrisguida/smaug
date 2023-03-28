//! This is a test plugin used to verify that we can compile and run
//! plugins using the Rust API against Core Lightning.
#[macro_use]
extern crate serde_json;
// use bitcoin;
use cln_plugin::{options, Builder, Error, Plugin};
use tokio;

use bdk::blockchain::ElectrumBlockchain;
use bdk::database::MemoryDatabase;
use bdk::electrum_client::Client;
use bdk::{bitcoin, descriptor, SyncOptions, Wallet};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let state = ();

    if let Some(plugin) = Builder::new(tokio::io::stdin(), tokio::io::stdout())
        .option(options::ConfigOption::new(
            "test-option",
            options::Value::Integer(42),
            "a test-option with default 42",
        ))
        .option(options::ConfigOption::new(
            "opt-option",
            options::Value::OptInteger,
            "An optional option",
        ))
        .rpcmethod(
            "watchdescriptor",
            "Watch a wallet descriptor and emit events when coins are moved",
            watchdescriptor,
        )
        .rpcmethod(
            "testoptions",
            "Retrieve options from this plugin",
            testoptions,
        )
        .subscribe("connect", connect_handler)
        .hook("peer_connected", peer_connected_handler)
        .dynamic()
        .start(state)
        .await?
    {
        plugin.join().await
    } else {
        Ok(())
    }
}

async fn testoptions(p: Plugin<()>, _v: serde_json::Value) -> Result<serde_json::Value, Error> {
    Ok(json!({
        "opt-option": format!("{:?}", p.option("opt-option").unwrap())
    }))
}

async fn watchdescriptor(_p: Plugin<()>, v: serde_json::Value) -> Result<serde_json::Value, Error> {
    let mut descriptor = "";
    let mut change_descriptor: Option<&str> = None;
    if v.is_object() {
        log::info!("Detected object: {}", v);
        descriptor = v.get("descriptor").unwrap().as_str().unwrap();
        change_descriptor = match v.get("change_descriptor") {
            Some(cd) => Some(cd.as_str()),
            None => None,
        }
    } else {
        log::info!("Detected array: {}", v);
        // log::info!("Descriptor: {}", v[0]);
        // log::info!("Change descriptor: {}", v[1]);
        if v.as_array().unwrap().len() > 0 {
            descriptor = v[0].as_str().unwrap();
        }
        if v.as_array().unwrap().len() > 1 {
            change_descriptor = Some(v[1].as_str().unwrap());
        }
    }
    let client = Client::new("ssl://electrum.blockstream.info:60002")?;
    let blockchain = ElectrumBlockchain::from(client);
    log::info!("descriptor: {:?}", v[0].as_str());
    // log::info!("change descriptor: {}", change_descriptor);
    let wallet = Wallet::new(
        // "tr([af4c5952/86h/0h/0h]xpub6DTzDxFnUS1vriU7fc3VkwdTnArhk6FafoZHRcfwjRqo7vkMnbAiKK9AEhR4feqcdsE36Y4ZCLHBcEszJcvV3pMLhS4D9Ed5VNhH6Cw17Pp/0/*)",
        descriptor,
        change_descriptor,
        // match change_descriptor.len() {
        //     0 => None,
        //     _ => Some(change_descriptor.as_str()),
        // },
        bitcoin::Network::Bitcoin,
        MemoryDatabase::default(),
    )?;

    wallet.sync(&blockchain, SyncOptions::default())?;

    // println!("Descriptor balance: {} SAT", wallet.get_balance()?);

    // Ok(v)
    Ok(json!(wallet.get_balance()?))
}

async fn testmethod(_p: Plugin<()>, _v: serde_json::Value) -> Result<serde_json::Value, Error> {
    Ok(json!("Hello"))
}

async fn connect_handler(_p: Plugin<()>, v: serde_json::Value) -> Result<(), Error> {
    log::info!("Got a connect notification: {}", v);
    Ok(())
}

async fn peer_connected_handler(
    _p: Plugin<()>,
    v: serde_json::Value,
) -> Result<serde_json::Value, Error> {
    log::info!("Got a connect hook call: {}", v);
    Ok(json!({"result": "continue"}))
}
