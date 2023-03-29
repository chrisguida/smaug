//! This is a test plugin used to verify that we can compile and run
//! plugins using the Rust API against Core Lightning.
use std::fmt;
#[macro_use]
extern crate serde_json;

// use anyhow;
use bdk::bitcoin::secp256k1::Secp256k1;
use cln_plugin::{anyhow, options, Builder, Error, Plugin};
use serde::Serialize;
use tokio;

use bdk::blockchain::ElectrumBlockchain;
use bdk::database::MemoryDatabase;
use bdk::electrum_client::Client;
use bdk::{bitcoin, descriptor, SyncOptions, Wallet};

// use crate::descriptor::{
//     calc_checksum, into_wallet_descriptor_checked

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

/// Errors related to the `registertower` command.
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
        birthday: Option<u32>,
        gap: Option<u32>,
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
            // descriptor: TowerId::from_str(tower_id)
            //     .map_err(|_| WatchError::InvalidId("Invalid tower id".to_owned()))?,
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

    fn with_birthday(self, birthday: u32) -> Result<Self, WatchError> {
        Ok(Self {
            birthday: Some(birthday as u32),
            ..self
        })
    }

    fn with_gap(self, gap: u32) -> Result<Self, WatchError> {
        if gap > u32::MAX / 2 {
            Err(WatchError::InvalidBirthday(format!(
                "birthday must be between 0 and 2147483647. Received: {gap}"
            )))
        } else {
            Ok(Self {
                gap: Some(gap),
                ..self
            })
        }
    }
}

impl TryFrom<serde_json::Value> for WatchParams {
    type Error = WatchError;

    fn try_from(value: serde_json::Value) -> Result<Self, Self::Error> {
        match value {
            serde_json::Value::String(s) => {
                let s = s.trim();
                // let mut v = s.split('@');
                let secp = Secp256k1::new();
                // let descriptor = bdk::descriptor::into_wallet_descriptor(s, &secp, bitcoin::Network::Bitcoin);
                let descriptor = s;

                // match v.next() {
                //     Some(x) => {
                //         let mut v = x.split(':');
                //         let host = v.next();
                //         let port = if let Some(p) = v.next() {
                //             p.parse()
                //                 .map(Some)
                //                 .map_err(|_| WatchError::InvalidPort(format!("Port is not a number: {p}")))?
                //         } else {
                //             None
                //         };

                WatchParams::new(descriptor.to_string().as_str(), None, None, None)
                //     }
                //     None => WatchParams::from_id(tower_id),
                // }
            },
            serde_json::Value::Array(mut a) => {
                let param_count = a.len();

                match param_count {
                    1 => WatchParams::try_from(a.pop().unwrap()),
                    2..=5 => {
                        let descriptor = a.get(0).unwrap().as_str().ok_or_else(|| WatchError::InvalidDescriptor("descriptor must be a string".to_string()))?;
                        let change_descriptor = Some(a.get(1).unwrap().as_str().ok_or_else(|| WatchError::InvalidChangeDescriptor("change_descriptor must be a string".to_string()))?);
                        let birthday = if let Some(b) = a.get(2) {
                            Some(b.as_u64().ok_or_else(|| WatchError::InvalidBirthday(format!("birthday must be a number. Received: {b}")))? as u32)
                        } else {
                            None
                        };
                        let gap = if let Some(g) = a.get(2) {
                            Some(g.as_u64().ok_or_else(|| WatchError::InvalidGap(format!("gap must be a number. Received: {g}")))? as u32)
                        } else {
                            None
                        };

                        WatchParams::new(descriptor, change_descriptor, birthday, gap)
                    }
                    _ => Err(WatchError::InvalidFormat(format!("Unexpected request format. The request needs 1-3 parameters. Received: {param_count}"))),
                }
            },
            serde_json::Value::Object(mut m) => {
                let allowed_keys = ["descriptor", "change_descriptor", "birthday", "gap"];
                let param_count = m.len();

                 if m.is_empty() || param_count > allowed_keys.len() {
                    Err(WatchError::InvalidFormat(format!("Unexpected request format. The request needs 1-4 parameters. Received: {param_count}")))
                 } else if !m.contains_key(allowed_keys[0]){
                    Err(WatchError::InvalidDescriptor(format!("{} is mandatory", allowed_keys[0])))
                 } else if !m.iter().all(|(k, _)| allowed_keys.contains(&k.as_str())) {
                    Err(WatchError::InvalidFormat("Invalid named parameter found in request".to_owned()))
                 } else {
                    let mut params = Vec::with_capacity(allowed_keys.len());
                    for k in allowed_keys {
                        if let Some(v) = m.remove(k) {
                            params.push(v);
                        }
                    }

                    WatchParams::try_from(json!(params))
                }
            },
            _ => Err(WatchError::InvalidFormat(
                format!("Unexpected request format. Expected: 'tower_id[@host][:port]' or 'tower_id [host] [port]'. Received: '{value}'"),
            )),
        }
    }
}

async fn watchdescriptor(_p: Plugin<()>, v: serde_json::Value) -> Result<serde_json::Value, Error> {
    let params = WatchParams::try_from(v).map_err(|x| anyhow!(x))?;
    // let mut descriptor = "";
    // let mut change_descriptor: Option<&str> = None;
    // if v.is_object() {
    //     // log::info!("Detected object: {}", v);
    //     // descriptor = v.get("descriptor").unwrap().as_str().unwrap();
    //     // change_descriptor = match v.get("change_descriptor") {
    //     //     Some(cd) => Some(cd.as_str()),
    //     //     None => None,
    //     // };
    // } else {
    //     log::info!("Detected array: {}", v);
    //     // log::info!("Descriptor: {}", v[0]);
    //     // log::info!("Change descriptor: {}", v[1]);
    //     if v.as_array().unwrap().len() > 0 {
    //         descriptor = v[0].as_str().unwrap();
    //     }
    //     if v.as_array().unwrap().len() > 1 {
    //         change_descriptor = Some(v[1].as_str().unwrap());
    //     }
    // }
    let client = Client::new("ssl://electrum.blockstream.info:60002")?;
    let blockchain = ElectrumBlockchain::from(client);
    // log::info!("descriptor: {:?}", v[0].as_str());
    // log::info!("change descriptor: {}", change_descriptor);
    if let Some(cd) = params.change_descriptor {
        cd.as_str()
    } else {
        ""
    };
    let wallet = Wallet::new(
        // "tr([af4c5952/86h/0h/0h]xpub6DTzDxFnUS1vriU7fc3VkwdTnArhk6FafoZHRcfwjRqo7vkMnbAiKK9AEhR4feqcdsE36Y4ZCLHBcEszJcvV3pMLhS4D9Ed5VNhH6Cw17Pp/0/*)",
        params.descriptor.as_str(),
        match change_descriptor {
            "" => None,
            _ => Some(change_descriptor),
        },
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
