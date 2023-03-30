//! This is a plugin used to track a given descriptor
//! wallet or set of descriptor wallets, and send
//! events to other listening processes when coin movements are detected.
use std::fmt;
#[macro_use]
extern crate serde_json;

use cln_plugin::{anyhow, options, Builder, Error, Plugin};
use serde::Serialize;
use tokio;

use bdk::blockchain::ElectrumBlockchain;
use bdk::database::MemoryDatabase;
use bdk::electrum_client::Client;
use bdk::{bitcoin, SyncOptions, Wallet};

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

/// Errors related to the `watchdescriptor` command.
#[derive(Debug)]
pub enum WatchError {
    InvalidDescriptor(String),
    InvalidChangeDescriptor(String),
    InvalidBirthday(String),
    InvalidGap(String),
    InvalidFormat(String),
}

impl std::fmt::Display for WatchError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            WatchError::InvalidDescriptor(x) => write!(f, "{x}"),
            WatchError::InvalidChangeDescriptor(x) => write!(f, "{x}"),
            WatchError::InvalidBirthday(x) => write!(f, "{x}"),
            WatchError::InvalidGap(x) => write!(f, "{x}"),
            WatchError::InvalidFormat(x) => write!(f, "{x}"),
        }
    }
}

/// Parameters related to the `watchdescriptor` command.
#[derive(Debug, Serialize)]
pub struct WatchParams {
    pub descriptor: String,
    pub change_descriptor: Option<String>,
    pub birthday: Option<u32>,
    pub gap: Option<u32>,
}
impl WatchParams {
    fn new(
        descriptor: &str,
        change_descriptor: Option<&str>,
        birthday: Option<u64>,
        gap: Option<u64>,
    ) -> Result<Self, WatchError> {
        let mut params = WatchParams::from_descriptor(descriptor)?;
        if change_descriptor.is_some() {
            params = params.with_change_descriptor(change_descriptor.unwrap())?
        }
        if birthday.is_some() {
            params = params.with_birthday(birthday.unwrap())?
        }
        if gap.is_some() {
            params = params.with_gap(gap.unwrap())?
        }
        Ok(params)
    }

    fn from_descriptor(descriptor: &str) -> Result<Self, WatchError> {
        Ok(Self {
            descriptor: descriptor.to_owned(),
            change_descriptor: None,
            birthday: None,
            gap: None,
        })
    }

    fn with_change_descriptor(self, change_descriptor: &str) -> Result<Self, WatchError> {
        if change_descriptor.is_empty() {
            Err(WatchError::InvalidChangeDescriptor(
                "change_descriptor is empty".to_owned(),
            ))
        } else {
            Ok(Self {
                change_descriptor: Some(String::from(change_descriptor)),
                ..self
            })
        }
    }

    fn with_birthday(self, birthday: u64) -> Result<Self, WatchError> {
        if birthday > u32::MAX as u64 {
            Err(WatchError::InvalidBirthday(format!(
                "birthday must be between 0 and 4294967295. Received: {birthday}"
            )))
        } else {
            Ok(Self {
                birthday: Some(birthday as u32),
                ..self
            })
        }
    }

    fn with_gap(self, gap: u64) -> Result<Self, WatchError> {
        if gap > u32::MAX as u64 / 2 {
            Err(WatchError::InvalidBirthday(format!(
                "gap must be between 0 and 2147483647. Received: {gap}"
            )))
        } else {
            Ok(Self {
                gap: Some(gap as u32),
                ..self
            })
        }
    }
}

impl TryFrom<serde_json::Value> for WatchParams {
    type Error = WatchError;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        log::info!("entering try_from");
        match value {
            serde_json::Value::Array(mut a) => {
                log::info!("try_from: array detected = {:?}", a);
                let param_count = a.len();

                match param_count {
                    1 => WatchParams::try_from(a.pop().unwrap()),
                    2..=4 => {
                        let descriptor = a.get(0).unwrap().as_str().ok_or_else(|| WatchError::InvalidDescriptor("descriptor must be a string".to_string()))?;
                        // let change_descriptor = Some(a.get(1).unwrap().as_str().ok_or_else(|| WatchError::InvalidChangeDescriptor("change_descriptor must be a string".to_string()))?);
                        log::info!("try_from array: change_descriptor = {:?}", a.get(1));
                        let change_descriptor = if let Some(cd) = a.get(1) {
                            Some(cd.as_str().ok_or_else(|| WatchError::InvalidChangeDescriptor(format!("change_descriptor must be a string. Received: {cd}")))?)
                        } else {
                            None
                        };
                        let birthday = if let Some(b) = a.get(2) {
                            Some(b.as_u64().ok_or_else(|| WatchError::InvalidBirthday(format!("birthday must be a number. Received: {b}")))?)
                        } else {
                            None
                        };
                        let gap = if let Some(g) = a.get(3) {
                            Some(g.as_u64().ok_or_else(|| WatchError::InvalidGap(format!("gap must be a number. Received: {g}")))?)
                        } else {
                            None
                        };

                        WatchParams::new(descriptor, change_descriptor, birthday, gap)
                    }
                    _ => Err(WatchError::InvalidFormat(format!("Unexpected request format. The request needs 1-4 parameters. Received: {param_count}"))),
                }
            },
            serde_json::Value::Object(m) => {
                log::info!("try_from: object detected");
                let allowed_keys = ["descriptor", "change_descriptor", "birthday", "gap"];
                let param_count = m.len();

                 if m.is_empty() || param_count > allowed_keys.len() {
                    Err(WatchError::InvalidFormat(format!("Unexpected request format. The request needs 1-4 parameters. Received: {param_count}")))
                 } else if !m.contains_key(allowed_keys[0]){
                    Err(WatchError::InvalidDescriptor(format!("{} is mandatory", allowed_keys[0])))
                 } else if !m.iter().all(|(k, _)| allowed_keys.contains(&k.as_str())) {
                    Err(WatchError::InvalidFormat(format!("Invalid named parameter found in request. Allowed named params: ['descriptor', 'change_descriptor', 'birthday', 'gap']")))
                 } else {
                    WatchParams::new(
                        m.get("descriptor").unwrap().as_str().unwrap(),
                        match m.get("change_descriptor") {
                            Some(v) => Some(v.as_str().unwrap()),
                            None => None,
                        },
                        match m.get("birthday") {
                            Some(v) => Some(v.as_u64().unwrap()),
                            None => None,
                        },
                        match m.get("gap") {
                            Some(v) => Some(v.as_u64().unwrap()),
                            None => None,
                        },
                    )
                }
            },
            _ => Err(WatchError::InvalidFormat(
                format!("Unexpected request format. Expected: <descriptor>, [change_descriptor, birthday, gap], either as ordered or keyword args. Received: '{value}'"),
            )),
        }
    }
}

async fn watchdescriptor(_p: Plugin<()>, v: serde_json::Value) -> Result<serde_json::Value, Error> {
    let params = WatchParams::try_from(v).map_err(|x| anyhow!(x))?;
    log::info!("params = {:?}", params);
    let client = Client::new("ssl://electrum.blockstream.info:60002")?;
    let blockchain = ElectrumBlockchain::from(client);
    log::info!("descriptor: {:?}", params.descriptor);
    log::info!("change descriptor: {:?}", params.change_descriptor);
    let wallet = Wallet::new(
        // "tr([af4c5952/86h/0h/0h]xpub6DTzDxFnUS1vriU7fc3VkwdTnArhk6FafoZHRcfwjRqo7vkMnbAiKK9AEhR4feqcdsE36Y4ZCLHBcEszJcvV3pMLhS4D9Ed5VNhH6Cw17Pp/0/*)",
        &params.descriptor,
        params.change_descriptor.as_ref(),
        bitcoin::Network::Bitcoin,
        MemoryDatabase::default(),
    )?;

    wallet.sync(&blockchain, SyncOptions::default())?;

    // println!("Descriptor balance: {} SAT", wallet.get_balance()?);

    // Ok(v)
    Ok(json!(wallet.get_balance()?))
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
