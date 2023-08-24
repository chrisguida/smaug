use std::collections::BTreeMap;

use bdk::bitcoin;

use crate::wallet::DescriptorWallet;

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
