use crate::ensure_layout;
use crate::extensions::seed_extension_instructions;
use crate::guard;
use crate::memory_root;
use crate::metrics::MEMORY_STARTUP;
use crate::phase1;
use crate::phase2;
use crate::runtime::MemoryStartupContext;
use codex_core::CodexThread;
use codex_core::ThreadManager;
use codex_core::config::Config;
use codex_features::Feature;
use codex_login::AuthManager;
use codex_protocol::ThreadId;
use codex_protocol::models::PermissionProfile;
use codex_protocol::protocol::InferenceWorkKind;
use codex_protocol::protocol::InferenceWorkOutcome;
use codex_protocol::protocol::SessionSource;
use std::sync::Arc;
use tracing::warn;

/// Starts the asynchronous startup memory pipeline for an eligible root session.
///
/// The pipeline is skipped for ephemeral sessions, disabled feature flags, and
/// subagent sessions.
pub fn start_memories_startup_task(
    thread_manager: Arc<ThreadManager>,
    auth_manager: Arc<AuthManager>,
    thread_id: ThreadId,
    thread: Arc<CodexThread>,
    config: Arc<Config>,
    parent_permission_profile: PermissionProfile,
    source: &SessionSource,
) {
    if config.ephemeral
        || !config.features.enabled(Feature::MemoryTool)
        || source.is_non_root_agent()
    {
        return;
    }

    let inference_work_scope = thread.inference_work_scope();
    let context = Arc::new(MemoryStartupContext::new(
        thread_manager,
        Arc::clone(&auth_manager),
        thread_id,
        thread,
        config.as_ref(),
        source.clone(),
    ));

    if context.state_db().is_none() {
        warn!("state db unavailable for memories startup pipeline; skipping");
        return;
    }
    let mut inference_work_guard = inference_work_scope.and_then(|scope| {
        scope.start_detached_work(
            format!("memory-pipeline:{}", uuid::Uuid::new_v4()),
            None,
            thread_id,
            InferenceWorkKind::Memory,
        )
    });

    tokio::spawn(async move {
        let result: anyhow::Result<()> = async {
            let root = memory_root(&config.codex_home);
            ensure_layout(&root).await?;
            if let Err(err) = seed_extension_instructions(&root).await {
                warn!("failed seeding memory extension instructions: {err}");
            }

            // Clean memories to make preserve DB size. This does not consume tokens so can be
            // done before the quota check.
            phase1::prune(context.as_ref(), &config).await;

            if !guard::rate_limits_ok(&auth_manager, &config).await {
                context.counter(
                    MEMORY_STARTUP,
                    /*inc*/ 1,
                    &[("status", "skipped_rate_limit")],
                );
                return Ok(());
            }

            // Run phase 1.
            phase1::run(Arc::clone(&context), Arc::clone(&config)).await;
            // Run phase 2.
            phase2::run(context, config, parent_permission_profile).await;
            Ok(())
        }
        .await;
        if let Some(guard) = inference_work_guard.as_mut() {
            guard.finish(if result.is_ok() {
                InferenceWorkOutcome::Completed
            } else {
                InferenceWorkOutcome::Failed
            });
        }
        if let Err(err) = result {
            warn!("failed running memories startup pipeline: {err}");
        }
    });
}
