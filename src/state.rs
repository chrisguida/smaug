use bdk::bitcoin;

use crate::wallet::DescriptorWallet;

#[derive(Debug, Clone)]
pub struct WatchDescriptor {
    /// A collection of descriptors the plugin is watching.
    pub wallets: Vec<DescriptorWallet>,
    pub network: bitcoin::Network,
}

impl WatchDescriptor {
    pub fn new() -> Self {
        Self {
            wallets: vec![],
            network: bitcoin::Network::Bitcoin,
        }
    }

    pub fn add_descriptor_wallet(&mut self, wallet: DescriptorWallet) {
        self.wallets.push(wallet);
    }
}
