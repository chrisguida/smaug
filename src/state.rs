use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

use bdk::bitcoin;
use tokio::sync::Mutex;

use crate::wallet::DescriptorWallet;

pub type State = Arc<Mutex<Smaug>>;

#[derive(Debug, Clone)]
pub struct Smaug {
    /// A collection of descriptors the plugin is watching.
    pub wallets: BTreeMap<String, DescriptorWallet>,
    // The network relevant to our wallets
    pub network: String,
    // The db path relevant to our wallets
    pub db_dir: PathBuf,
}

impl Smaug {
    pub fn new() -> Self {
        Self {
            wallets: BTreeMap::new(),
            network: bitcoin::Network::Bitcoin.to_string(),
            db_dir: PathBuf::new(),
        }
    }

    pub fn add_descriptor_wallet(
        &mut self,
        wallet: &DescriptorWallet,
    ) -> Result<(), anyhow::Error> {
        self.wallets.insert(wallet.get_name()?, wallet.clone());
        Ok(())
    }
}
