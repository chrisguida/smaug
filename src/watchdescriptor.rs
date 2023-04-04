use crate::params::DescriptorWallet;

#[derive(Debug, Clone)]
pub struct WatchDescriptor {
    /// A collection of descriptors the plugin is watching.
    pub wallets: Vec<DescriptorWallet>,
}

impl WatchDescriptor {
    pub fn new() -> Self {
        Self { wallets: vec![] }
    }

    pub fn add_descriptor_wallet(&mut self, wallet: DescriptorWallet) {
        self.wallets.push(wallet);
    }
}
