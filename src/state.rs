use std::{collections::BTreeMap, sync::Arc};

use bdk::bitcoin;
use tokio::sync::Mutex;

use crate::wallet::DescriptorWallet;

pub type State = Arc<Mutex<WatchDescriptor>>;

#[derive(Debug, Clone)]
pub struct WatchDescriptor {
    /// A collection of descriptors the plugin is watching.
    pub wallets: BTreeMap<String, DescriptorWallet>,
    pub network: bitcoin::Network,
}

impl WatchDescriptor {
    pub fn new() -> Self {
        Self {
            wallets: BTreeMap::new(),
            network: bitcoin::Network::Bitcoin,
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
