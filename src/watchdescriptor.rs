#[derive(Clone)]
pub struct WatchDescriptor {
    /// A collection of descriptors the plugin is watching.
    pub descriptors: Vec<String>,
}

impl WatchDescriptor {
    pub async fn new() -> Self {
        Self {
            descriptors: vec![],
        }
    }
}
