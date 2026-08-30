use super::provider_cache_path;
use pretty_assertions::assert_eq;
use std::path::Path;

#[test]
fn provider_cache_paths_are_partitioned_and_path_safe() {
    let codex_home = Path::new("/tmp/codex-home");

    assert_eq!(
        provider_cache_path(codex_home, "openai"),
        codex_home.join("models_cache/6f70656e6169.json")
    );
    assert_eq!(
        provider_cache_path(codex_home, "../openrouter"),
        codex_home.join("models_cache/2e2e2f6f70656e726f75746572.json")
    );
}
