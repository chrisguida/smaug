use bdk::Wallet;

#[derive(Debug)]
pub struct WatchDescriptor<D: Clone> {
    /// A collection of descriptors the plugin is watching.
    pub wallets: Vec<Wallet<D>>,
}

impl<D: Clone> WatchDescriptor<D> {
    pub fn new() -> Self {
        Self { wallets: vec![] }
    }

    pub fn add_descriptor_wallet(&mut self, wallet: Wallet<D>) {
        self.wallets.push(wallet);
    }
}
