use super::ModelsManagerRegistry;
use crate::manager::SharedModelsManager;
use crate::manager::StaticModelsManager;
use codex_protocol::openai_models::ModelsResponse;
use pretty_assertions::assert_eq;
use std::collections::HashMap;
use std::sync::Arc;

fn manager(slug: &str) -> SharedModelsManager {
    let mut model = crate::bundled_models_response()
        .expect("bundled models should parse")
        .models
        .into_iter()
        .next()
        .expect("bundled catalog should contain a model");
    model.slug = slug.to_string();
    Arc::new(StaticModelsManager::new(
        /*auth_manager*/ None,
        ModelsResponse {
            models: vec![model],
        },
    ))
}

#[tokio::test]
async fn registry_resolves_exact_provider_catalogs() {
    let registry = ModelsManagerRegistry::new(
        "openai",
        HashMap::from([
            ("openai".to_string(), manager("gpt-openai")),
            ("openrouter".to_string(), manager("openrouter/model")),
        ]),
    )
    .expect("default provider exists");

    assert_eq!(registry.default_provider_id(), "openai");
    assert_eq!(
        registry
            .default_manager()
            .get_remote_models()
            .await
            .into_iter()
            .map(|model| model.slug)
            .collect::<Vec<_>>(),
        vec!["gpt-openai"]
    );
    assert_eq!(
        registry
            .get("openrouter")
            .expect("provider manager")
            .get_remote_models()
            .await
            .into_iter()
            .map(|model| model.slug)
            .collect::<Vec<_>>(),
        vec!["openrouter/model"]
    );
    assert!(registry.get("missing").is_none());
}

#[test]
fn registry_rejects_a_missing_default_provider() {
    assert!(ModelsManagerRegistry::new("missing", HashMap::new()).is_none());
}
