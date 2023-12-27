use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use bdk::bitcoin;
use bitcoincore_rpc::Auth;
use tokio::sync::Mutex;

use crate::wallet::DescriptorWallet;

pub type State = Arc<Mutex<Smaug>>;

#[derive(Debug, Clone)]
pub struct Smaug {
    /// A collection of descriptors the plugin is watching.
    pub wallets: BTreeMap<String, DescriptorWallet>,
    /// The network relevant to our wallets
    pub network: String,
    /// Bitcoind RPC host
    pub brpc_host: String,
    /// Bitcoind RPC port
    pub brpc_port: u16,
    // /// Bitcoind RPC user
    // pub brpc_user: String,
    // /// Bitcoind RPC password
    // pub brpc_pass: String,
    pub brpc_auth: Auth,
    /// The db path relevant to our wallets
    pub db_dir: PathBuf,
}

impl Smaug {
    pub fn new() -> Self {
        Self {
            wallets: BTreeMap::new(),
            network: bitcoin::Network::Bitcoin.to_string(),
            brpc_host: String::from("127.0.0.1"),
            brpc_port: 8332,
            brpc_auth: Auth::None,
            db_dir: PathBuf::new(),
        }
    }

    pub fn add_descriptor_wallet(
        &mut self,
        wallet: &DescriptorWallet,
    ) -> Result<(), anyhow::Error> {
        log::trace!("add_descriptor_wallet called");
        self.wallets.insert(wallet.get_name()?, wallet.clone());
        Ok(())
    }
}
