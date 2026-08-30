use crate::manager::SharedModelsManager;
use std::collections::HashMap;
use std::sync::Arc;

/// Immutable process registry of model managers keyed by configured provider ID.
///
/// Each manager owns only one provider's catalog and cache identity. Callers must resolve a
/// manager from the effective thread provider instead of sharing a process-global catalog.
#[derive(Clone, Debug)]
pub struct ModelsManagerRegistry {
    default_provider_id: Arc<str>,
    managers: Arc<HashMap<String, SharedModelsManager>>,
}

impl ModelsManagerRegistry {
    /// Builds a registry when `default_provider_id` has an exact manager entry.
    pub fn new(
        default_provider_id: impl Into<String>,
        managers: HashMap<String, SharedModelsManager>,
    ) -> Option<Self> {
        let default_provider_id = default_provider_id.into();
        managers.contains_key(&default_provider_id).then(|| Self {
            default_provider_id: default_provider_id.into(),
            managers: Arc::new(managers),
        })
    }

    /// Builds a registry containing one provider manager.
    pub fn from_default(
        default_provider_id: impl Into<String>,
        manager: SharedModelsManager,
    ) -> Self {
        let default_provider_id = default_provider_id.into();
        Self {
            default_provider_id: Arc::from(default_provider_id.as_str()),
            managers: Arc::new(HashMap::from([(default_provider_id, manager)])),
        }
    }

    /// Returns the manager for an exact provider ID.
    pub fn get(&self, provider_id: &str) -> Option<SharedModelsManager> {
        self.managers.get(provider_id).cloned()
    }

    /// Returns every registered provider manager.
    pub fn all(&self) -> Vec<SharedModelsManager> {
        self.managers.values().cloned().collect()
    }

    /// Returns the process default manager.
    pub fn default_manager(&self) -> SharedModelsManager {
        self.managers[self.default_provider_id.as_ref()].clone()
    }

    /// Returns the configured default provider ID.
    pub fn default_provider_id(&self) -> &str {
        &self.default_provider_id
    }
}

#[cfg(test)]
#[path = "registry_tests.rs"]
mod tests;
