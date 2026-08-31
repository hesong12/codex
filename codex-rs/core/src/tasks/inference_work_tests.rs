use codex_protocol::ThreadId;
use codex_protocol::protocol::InferenceWorkKind;
use codex_protocol::protocol::InternalSessionSource;
use codex_protocol::protocol::SessionSource;
use codex_protocol::protocol::SubAgentSource;
use pretty_assertions::assert_eq;

use super::inference_work_kind;
use crate::state::TaskKind;

#[test]
fn native_work_kinds_cover_turn_goal_resident_review_compaction_and_memory() {
    let resident = SessionSource::SubAgent(SubAgentSource::ThreadSpawn {
        parent_thread_id: ThreadId::new(),
        depth: 1,
        agent_path: None,
        agent_nickname: None,
        agent_role: None,
    });
    let cases = [
        (
            TaskKind::Regular,
            SessionSource::Exec,
            None,
            InferenceWorkKind::Turn,
        ),
        (
            TaskKind::Regular,
            SessionSource::Exec,
            Some("goal"),
            InferenceWorkKind::GoalContinuation,
        ),
        (
            TaskKind::Regular,
            resident,
            None,
            InferenceWorkKind::Subagent,
        ),
        (
            TaskKind::Regular,
            SessionSource::SubAgent(SubAgentSource::Review),
            None,
            InferenceWorkKind::Review,
        ),
        (
            TaskKind::Regular,
            SessionSource::SubAgent(SubAgentSource::Compact),
            None,
            InferenceWorkKind::Compaction,
        ),
        (
            TaskKind::Regular,
            SessionSource::Internal(InternalSessionSource::MemoryConsolidation),
            None,
            InferenceWorkKind::Memory,
        ),
        (
            TaskKind::Review,
            SessionSource::Exec,
            None,
            InferenceWorkKind::Review,
        ),
        (
            TaskKind::Compact,
            SessionSource::Exec,
            None,
            InferenceWorkKind::Compaction,
        ),
    ];

    for (task_kind, session_source, turn_trigger, expected) in cases {
        assert_eq!(
            inference_work_kind(task_kind, &session_source, turn_trigger),
            expected
        );
    }
}
