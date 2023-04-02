use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub struct WatchDescriptor {
    /// A collection of descriptors the plugin is watching.
    pub descriptors: Vec<String>,
}

impl WatchDescriptor {
    pub fn new() -> Self {
        Self {
            descriptors: vec![],
        }
    }

    pub fn with_descriptor() {}

    pub async fn add_descriptor(&mut self, descriptor: String) {
        self.descriptors.push(descriptor);
    }
}
