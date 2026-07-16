use crate::model::ProviderInfo;

#[derive(Debug)]
pub struct ProviderStore {
    providers: Vec<ProviderInfo>,
}

impl ProviderStore {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn replace(&mut self, new: Vec<ProviderInfo>) -> bool {
        if self.providers == new {
            return false;
        }
        self.providers = new;
        true
    }

    pub fn providers(&self) -> &[ProviderInfo] {
        &self.providers
    }
}

impl Default for ProviderStore {
    fn default() -> Self {
        Self::new()
    }
}
