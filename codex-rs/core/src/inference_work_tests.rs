use async_channel::Receiver;
use codex_protocol::ThreadId;
use codex_protocol::protocol::Event;
use codex_protocol::protocol::EventMsg;
use codex_protocol::protocol::InferenceWorkCompletedEvent;
use codex_protocol::protocol::InferenceWorkKind;
use codex_protocol::protocol::InferenceWorkOutcome;
use codex_protocol::protocol::InferenceWorkStartedEvent;
use codex_protocol::protocol::InferenceWorkSubtreeIdleEvent;
use pretty_assertions::assert_eq;
use tokio_util::sync::CancellationToken;

use super::InferenceWorkScope;

#[derive(Debug, PartialEq, Eq)]
enum ObservedEvent {
    Started(InferenceWorkStartedEvent),
    Completed(InferenceWorkCompletedEvent),
    SubtreeIdle(InferenceWorkSubtreeIdleEvent),
}

fn receive(receiver: &Receiver<Event>) -> ObservedEvent {
    let event = receiver.try_recv().expect("inference work event");
    match event.msg {
        EventMsg::InferenceWorkStarted(event) => ObservedEvent::Started(event),
        EventMsg::InferenceWorkCompleted(event) => ObservedEvent::Completed(event),
        EventMsg::InferenceWorkSubtreeIdle(event) => ObservedEvent::SubtreeIdle(event),
        event => panic!("unexpected event: {event:?}"),
    }
}

#[test]
fn scope_id_validation_is_header_safe_and_bounded() {
    assert_eq!(InferenceWorkScope::validate_scope_id("route-123"), Ok(()));
    assert!(InferenceWorkScope::validate_scope_id("").is_err());
    assert!(InferenceWorkScope::validate_scope_id("contains space").is_err());
    assert!(InferenceWorkScope::validate_scope_id(&"a".repeat(129)).is_err());
}

#[test]
fn descendants_delay_subtree_idle_and_every_terminal_is_exactly_once() {
    let (sender, receiver) = async_channel::unbounded();
    let scope = InferenceWorkScope::new_for_test("scope-exact-once", sender);
    let root_thread_id = ThreadId::new();
    let child_thread_id = ThreadId::new();
    let mut root = scope
        .start_work(
            "root-work".to_string(),
            None,
            root_thread_id,
            InferenceWorkKind::Turn,
            CancellationToken::new(),
        )
        .expect("root work starts");
    let child_cancel = CancellationToken::new();
    let child = scope
        .start_work(
            "child-work".to_string(),
            Some("root-work".to_string()),
            child_thread_id,
            InferenceWorkKind::Subagent,
            child_cancel.clone(),
        )
        .expect("child work starts");

    assert_eq!(
        receive(&receiver),
        ObservedEvent::Started(InferenceWorkStartedEvent {
            scope_id: "scope-exact-once".to_string(),
            root_work_id: "root-work".to_string(),
            work_id: "root-work".to_string(),
            parent_work_id: None,
            thread_id: root_thread_id,
            kind: InferenceWorkKind::Turn,
        })
    );
    assert_eq!(
        receive(&receiver),
        ObservedEvent::Started(InferenceWorkStartedEvent {
            scope_id: "scope-exact-once".to_string(),
            root_work_id: "root-work".to_string(),
            work_id: "child-work".to_string(),
            parent_work_id: Some("root-work".to_string()),
            thread_id: child_thread_id,
            kind: InferenceWorkKind::Subagent,
        })
    );

    root.finish(InferenceWorkOutcome::Completed);
    root.finish(InferenceWorkOutcome::Failed);
    assert_eq!(
        receive(&receiver),
        ObservedEvent::Completed(InferenceWorkCompletedEvent {
            scope_id: "scope-exact-once".to_string(),
            root_work_id: "root-work".to_string(),
            work_id: "root-work".to_string(),
            thread_id: root_thread_id,
            kind: InferenceWorkKind::Turn,
            outcome: InferenceWorkOutcome::Completed,
        })
    );
    assert!(receiver.is_empty());

    child_cancel.cancel();
    drop(child);
    assert_eq!(
        receive(&receiver),
        ObservedEvent::Completed(InferenceWorkCompletedEvent {
            scope_id: "scope-exact-once".to_string(),
            root_work_id: "root-work".to_string(),
            work_id: "child-work".to_string(),
            thread_id: child_thread_id,
            kind: InferenceWorkKind::Subagent,
            outcome: InferenceWorkOutcome::Interrupted,
        })
    );
    assert_eq!(
        receive(&receiver),
        ObservedEvent::SubtreeIdle(InferenceWorkSubtreeIdleEvent {
            scope_id: "scope-exact-once".to_string(),
            root_work_id: "root-work".to_string(),
        })
    );
    assert!(receiver.is_empty());
    assert!(
        scope
            .start_work(
                "late-work".to_string(),
                None,
                root_thread_id,
                InferenceWorkKind::Turn,
                CancellationToken::new(),
            )
            .is_none()
    );
}

#[test]
fn unexpected_guard_drop_reports_crash_then_subtree_idle() {
    let (sender, receiver) = async_channel::unbounded();
    let scope = InferenceWorkScope::new_for_test("scope-crash", sender);
    let thread_id = ThreadId::new();
    let guard = scope
        .start_work(
            "crashed-work".to_string(),
            None,
            thread_id,
            InferenceWorkKind::Memory,
            CancellationToken::new(),
        )
        .expect("work starts");
    let _ = receive(&receiver);

    drop(guard);

    assert_eq!(
        receive(&receiver),
        ObservedEvent::Completed(InferenceWorkCompletedEvent {
            scope_id: "scope-crash".to_string(),
            root_work_id: "crashed-work".to_string(),
            work_id: "crashed-work".to_string(),
            thread_id,
            kind: InferenceWorkKind::Memory,
            outcome: InferenceWorkOutcome::Crashed,
        })
    );
    assert_eq!(
        receive(&receiver),
        ObservedEvent::SubtreeIdle(InferenceWorkSubtreeIdleEvent {
            scope_id: "scope-crash".to_string(),
            root_work_id: "crashed-work".to_string(),
        })
    );
    assert!(receiver.is_empty());
}
