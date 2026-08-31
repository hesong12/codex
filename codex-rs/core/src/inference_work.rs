use std::collections::HashMap;
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::Weak;

use async_channel::Sender;
use codex_protocol::ThreadId;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::InferenceWorkCompletedEvent;
use codex_protocol::protocol::InferenceWorkKind;
use codex_protocol::protocol::InferenceWorkOutcome;
use codex_protocol::protocol::InferenceWorkStartedEvent;
use codex_protocol::protocol::InferenceWorkSubtreeIdleEvent;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

const MAX_SCOPE_ID_BYTES: usize = 128;
static INFERENCE_WORK_SCOPES: LazyLock<Mutex<HashMap<String, Weak<InferenceWorkScopeInner>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Opaque, non-secret marker supplied by a host for one causal inference subtree.
///
/// Clones share exact-once lifecycle state. Descendant turns must inherit the clone rather than
/// construct another scope with the same string, or the host cannot rely on subtree-idle evidence.
#[derive(Clone)]
pub struct InferenceWorkScope {
    inner: Arc<InferenceWorkScopeInner>,
}

impl fmt::Debug for InferenceWorkScope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InferenceWorkScope")
            .field("scope_id", &self.inner.scope_id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
#[error("inference work scope must be 1-128 visible ASCII bytes")]
pub struct InvalidInferenceWorkScope;

struct InferenceWorkScopeInner {
    scope_id: String,
    event_sender: OnceLock<Sender<Event>>,
    state: Mutex<InferenceWorkScopeState>,
}

#[derive(Default)]
struct InferenceWorkScopeState {
    root_work_id: Option<String>,
    active: HashMap<String, ActiveInferenceWork>,
    terminal: HashSet<String>,
    subtree_idle_emitted: bool,
}

#[derive(Clone)]
struct ActiveInferenceWork {
    thread_id: ThreadId,
    kind: InferenceWorkKind,
}

impl InferenceWorkScope {
    pub fn validate_scope_id(scope_id: &str) -> Result<(), InvalidInferenceWorkScope> {
        if scope_id.is_empty()
            || scope_id.len() > MAX_SCOPE_ID_BYTES
            || !scope_id.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(InvalidInferenceWorkScope);
        }
        Ok(())
    }

    pub(crate) fn resolve(
        scope_id: String,
        event_sender: Sender<Event>,
    ) -> Result<Self, InvalidInferenceWorkScope> {
        Self::validate_scope_id(&scope_id)?;
        let inner = {
            let mut scopes = INFERENCE_WORK_SCOPES
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            scopes.retain(|_, scope| scope.strong_count() != 0);
            match scopes.get(&scope_id).and_then(Weak::upgrade) {
                Some(scope) => scope,
                None => {
                    let scope = Arc::new(InferenceWorkScopeInner {
                        scope_id: scope_id.clone(),
                        event_sender: OnceLock::new(),
                        state: Mutex::new(InferenceWorkScopeState::default()),
                    });
                    scopes.insert(scope_id, Arc::downgrade(&scope));
                    scope
                }
            }
        };
        let scope = Self { inner };
        scope.bind_event_sender(event_sender);
        Ok(scope)
    }

    #[cfg(test)]
    fn new_for_test(scope_id: impl Into<String>, event_sender: Sender<Event>) -> Self {
        Self::resolve(scope_id.into(), event_sender).expect("valid test inference work scope")
    }

    pub fn scope_id(&self) -> &str {
        &self.inner.scope_id
    }

    pub(crate) fn bind_event_sender(&self, event_sender: Sender<Event>) {
        let _ = self.inner.event_sender.set(event_sender);
    }

    pub(crate) fn start_work(
        &self,
        work_id: String,
        parent_work_id: Option<String>,
        thread_id: ThreadId,
        kind: InferenceWorkKind,
        cancellation_token: CancellationToken,
    ) -> Option<InferenceWorkGuard> {
        if work_id.is_empty() {
            return None;
        }
        let root_work_id = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.subtree_idle_emitted
                || state.terminal.contains(&work_id)
                || state.active.contains_key(&work_id)
            {
                return None;
            }
            let root_work_id = state
                .root_work_id
                .get_or_insert_with(|| work_id.clone())
                .clone();
            state
                .active
                .insert(work_id.clone(), ActiveInferenceWork { thread_id, kind });
            root_work_id
        };
        self.send_event(
            work_id.clone(),
            EventMsg::InferenceWorkStarted(InferenceWorkStartedEvent {
                scope_id: self.inner.scope_id.clone(),
                root_work_id,
                work_id: work_id.clone(),
                parent_work_id,
                thread_id,
                kind,
            }),
        );
        Some(InferenceWorkGuard {
            scope: self.clone(),
            work_id,
            cancellation_token,
            finished: false,
        })
    }

    /// Registers provider-backed work that is not owned by a Core turn task.
    pub fn start_detached_work(
        &self,
        work_id: String,
        parent_work_id: Option<String>,
        thread_id: ThreadId,
        kind: InferenceWorkKind,
    ) -> Option<InferenceWorkGuard> {
        self.start_work(
            work_id,
            parent_work_id,
            thread_id,
            kind,
            CancellationToken::new(),
        )
    }

    fn finish_work(&self, work_id: &str, outcome: InferenceWorkOutcome) {
        let (active, root_work_id, emit_idle) = {
            let mut state = self
                .inner
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(active) = state.active.remove(work_id) else {
                return;
            };
            if !state.terminal.insert(work_id.to_string()) {
                return;
            }
            let root_work_id = state
                .root_work_id
                .clone()
                .unwrap_or_else(|| work_id.to_string());
            let emit_idle = state.active.is_empty() && !state.subtree_idle_emitted;
            if emit_idle {
                state.subtree_idle_emitted = true;
            }
            (active, root_work_id, emit_idle)
        };
        self.send_event(
            work_id.to_string(),
            EventMsg::InferenceWorkCompleted(InferenceWorkCompletedEvent {
                scope_id: self.inner.scope_id.clone(),
                root_work_id: root_work_id.clone(),
                work_id: work_id.to_string(),
                thread_id: active.thread_id,
                kind: active.kind,
                outcome,
            }),
        );
        if emit_idle {
            self.send_event(
                root_work_id.clone(),
                EventMsg::InferenceWorkSubtreeIdle(InferenceWorkSubtreeIdleEvent {
                    scope_id: self.inner.scope_id.clone(),
                    root_work_id,
                }),
            );
        }
    }

    fn send_event(&self, id: String, msg: EventMsg) {
        if let Some(sender) = self.inner.event_sender.get() {
            let _ = sender.try_send(Event { id, msg });
        }
    }
}

/// RAII terminal for one item in an [`InferenceWorkScope`].
///
/// Call [`Self::finish`] on ordinary completion. Dropping an unfinished guard emits Interrupted
/// after cancellation and Crashed otherwise, so task aborts and panics cannot strand a scope.
pub struct InferenceWorkGuard {
    scope: InferenceWorkScope,
    work_id: String,
    cancellation_token: CancellationToken,
    finished: bool,
}

impl InferenceWorkGuard {
    pub fn finish(&mut self, outcome: InferenceWorkOutcome) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.scope.finish_work(&self.work_id, outcome);
    }
}

impl Drop for InferenceWorkGuard {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let outcome = if self.cancellation_token.is_cancelled() {
            InferenceWorkOutcome::Interrupted
        } else {
            InferenceWorkOutcome::Crashed
        };
        self.finished = true;
        self.scope.finish_work(&self.work_id, outcome);
    }
}

#[cfg(test)]
#[path = "inference_work_tests.rs"]
mod tests;
