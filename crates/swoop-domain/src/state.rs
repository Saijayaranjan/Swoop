//! The task state machine.
//!
//! Every transition in the product goes through [`TaskState::can_transition_to`]; the services
//! layer refuses anything else, which keeps persistence, UI and API consistent.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    /// Created but not yet admitted to a queue (e.g. waiting for the user to confirm options).
    Pending,
    /// Waiting for a free slot in its queue.
    Queued,
    /// Waiting for its schedule window.
    Scheduled,
    /// Resolving metadata: HEAD probe, magnet metadata, playlist parsing, mirror selection.
    Resolving,
    /// Opening connections.
    Connecting,
    /// Transferring data.
    Downloading,
    /// Paused by the user, a queue, a schedule or a condition (battery/network).
    Paused,
    /// Waiting for a backoff timer before reconnecting.
    Retrying,
    /// All bytes received; verifying checksum / piece hashes.
    Verifying,
    /// Post-processing (segment merge, remux, rule actions, automation).
    Processing,
    /// Finished successfully.
    Completed,
    /// Failed after exhausting the retry policy or hitting a permanent error.
    Failed,
    /// Cancelled by the user.
    Cancelled,
    /// Torrent finished downloading and is uploading to peers.
    Seeding,
}

impl TaskState {
    pub const ALL: [TaskState; 14] = [
        TaskState::Pending,
        TaskState::Queued,
        TaskState::Scheduled,
        TaskState::Resolving,
        TaskState::Connecting,
        TaskState::Downloading,
        TaskState::Paused,
        TaskState::Retrying,
        TaskState::Verifying,
        TaskState::Processing,
        TaskState::Completed,
        TaskState::Failed,
        TaskState::Cancelled,
        TaskState::Seeding,
    ];

    /// States in which the engine holds a live transfer (and therefore a queue slot).
    pub fn is_active(self) -> bool {
        matches!(
            self,
            TaskState::Resolving
                | TaskState::Connecting
                | TaskState::Downloading
                | TaskState::Retrying
                | TaskState::Verifying
                | TaskState::Processing
                | TaskState::Seeding
        )
    }

    /// States that consume network bandwidth.
    pub fn is_transferring(self) -> bool {
        matches!(self, TaskState::Downloading | TaskState::Seeding)
    }

    /// Terminal states: the task will not change without a user action.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TaskState::Completed | TaskState::Failed | TaskState::Cancelled
        )
    }

    /// The task is waiting for the queue/scheduler to pick it up.
    pub fn is_waiting(self) -> bool {
        matches!(
            self,
            TaskState::Pending | TaskState::Queued | TaskState::Scheduled
        )
    }

    pub fn can_pause(self) -> bool {
        matches!(
            self,
            TaskState::Queued
                | TaskState::Scheduled
                | TaskState::Resolving
                | TaskState::Connecting
                | TaskState::Downloading
                | TaskState::Retrying
                | TaskState::Seeding
        )
    }

    pub fn can_resume(self) -> bool {
        matches!(
            self,
            TaskState::Paused | TaskState::Failed | TaskState::Cancelled | TaskState::Pending
        )
    }

    /// Validates a transition. `to == self` is never a transition (callers that only change
    /// `blocked_by` or details must not call `transition`).
    pub fn can_transition_to(self, to: TaskState) -> bool {
        use TaskState::*;
        if self == to {
            return false;
        }
        match self {
            Pending => matches!(to, Queued | Scheduled | Paused | Cancelled | Failed),
            Queued => matches!(
                to,
                Resolving | Connecting | Scheduled | Paused | Cancelled | Failed
            ),
            Scheduled => matches!(to, Queued | Paused | Cancelled),
            // Resolving → Seeding: resuming an already-complete torrent.
            // Resolving → Queued/Scheduled: pre-empted by the queue or gated by a schedule.
            Resolving => matches!(
                to,
                Connecting
                    | Downloading
                    | Retrying
                    | Paused
                    | Failed
                    | Cancelled
                    | Verifying
                    | Completed
                    | Seeding
                    | Queued
                    | Scheduled
            ),
            Connecting => matches!(
                to,
                Downloading
                    | Retrying
                    | Paused
                    | Failed
                    | Cancelled
                    | Verifying
                    | Queued
                    | Scheduled
            ),
            Downloading => matches!(
                to,
                Paused
                    | Retrying
                    | Verifying
                    | Processing
                    | Completed
                    | Failed
                    | Cancelled
                    | Seeding
                    | Connecting
                    | Queued
                    | Scheduled
            ),
            Paused => matches!(to, Queued | Scheduled | Cancelled | Resolving),
            // Retrying → Queued/Scheduled: the timer fired but the queue is full/paused/gated.
            Retrying => matches!(
                to,
                Connecting | Resolving | Failed | Paused | Cancelled | Queued | Scheduled
            ),
            Verifying => matches!(
                to,
                Processing | Completed | Failed | Cancelled | Retrying | Seeding
            ),
            Processing => matches!(to, Completed | Failed | Cancelled | Seeding | Retrying),
            Completed => matches!(to, Queued | Seeding | Processing | Pending),
            Failed => matches!(to, Queued | Scheduled | Cancelled | Pending),
            Cancelled => matches!(to, Queued | Scheduled | Pending),
            // Seeding → Downloading: the user selected additional files.
            // Seeding → Resolving: recovery re-adds a seeding torrent to the session.
            Seeding => matches!(
                to,
                Completed | Paused | Cancelled | Failed | Downloading | Resolving
            ),
        }
    }

    pub fn label_key(self) -> &'static str {
        match self {
            TaskState::Pending => "state.pending",
            TaskState::Queued => "state.queued",
            TaskState::Scheduled => "state.scheduled",
            TaskState::Resolving => "state.resolving",
            TaskState::Connecting => "state.connecting",
            TaskState::Downloading => "state.downloading",
            TaskState::Paused => "state.paused",
            TaskState::Retrying => "state.retrying",
            TaskState::Verifying => "state.verifying",
            TaskState::Processing => "state.processing",
            TaskState::Completed => "state.completed",
            TaskState::Failed => "state.failed",
            TaskState::Cancelled => "state.cancelled",
            TaskState::Seeding => "state.seeding",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            TaskState::Pending => "pending",
            TaskState::Queued => "queued",
            TaskState::Scheduled => "scheduled",
            TaskState::Resolving => "resolving",
            TaskState::Connecting => "connecting",
            TaskState::Downloading => "downloading",
            TaskState::Paused => "paused",
            TaskState::Retrying => "retrying",
            TaskState::Verifying => "verifying",
            TaskState::Processing => "processing",
            TaskState::Completed => "completed",
            TaskState::Failed => "failed",
            TaskState::Cancelled => "cancelled",
            TaskState::Seeding => "seeding",
        }
    }

    pub fn parse(s: &str) -> Option<TaskState> {
        TaskState::ALL.iter().copied().find(|st| st.as_str() == s)
    }
}

impl fmt::Display for TaskState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a task is not running. A task may be blocked by several reasons at once (user pause
/// during a closed schedule window); it resumes only when the set is empty, and a user pause is
/// never cleared by an automatic event.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "reason", content = "detail")]
pub enum PauseReason {
    User,
    Queue(String),
    Schedule(String),
    Condition(String),
    Shutdown,
    DiskSpace,
    NetworkUnavailable,
    VolumeUnavailable,
}

impl PauseReason {
    /// Automatic reasons are cleared by the services layer when their cause goes away.
    pub fn is_automatic(&self) -> bool {
        !matches!(self, PauseReason::User)
    }
    pub fn label_key(&self) -> &'static str {
        match self {
            PauseReason::User => "pause.user",
            PauseReason::Queue(_) => "pause.queue",
            PauseReason::Schedule(_) => "pause.schedule",
            PauseReason::Condition(_) => "pause.condition",
            PauseReason::Shutdown => "pause.shutdown",
            PauseReason::DiskSpace => "pause.disk_space",
            PauseReason::NetworkUnavailable => "pause.network",
            PauseReason::VolumeUnavailable => "pause.volume",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_state_has_round_trip_name() {
        for s in TaskState::ALL {
            assert_eq!(TaskState::parse(s.as_str()), Some(s));
        }
    }

    #[test]
    fn transitions_are_sane() {
        assert!(TaskState::Queued.can_transition_to(TaskState::Resolving));
        assert!(TaskState::Downloading.can_transition_to(TaskState::Paused));
        assert!(TaskState::Paused.can_transition_to(TaskState::Queued));
        assert!(!TaskState::Completed.can_transition_to(TaskState::Downloading));
        assert!(!TaskState::Downloading.can_transition_to(TaskState::Downloading));
        assert!(TaskState::Failed.can_transition_to(TaskState::Queued));
        assert!(TaskState::Seeding.can_transition_to(TaskState::Completed));
        assert!(!TaskState::Cancelled.can_transition_to(TaskState::Downloading));
        assert!(TaskState::Resolving.can_transition_to(TaskState::Seeding));
        assert!(TaskState::Seeding.can_transition_to(TaskState::Downloading));
        assert!(TaskState::Retrying.can_transition_to(TaskState::Queued));
        assert!(TaskState::Downloading.can_transition_to(TaskState::Queued));
        assert!(TaskState::Pending.can_transition_to(TaskState::Failed));
        assert!(TaskState::Downloading.can_transition_to(TaskState::Scheduled));
    }

    #[test]
    fn terminal_states_are_not_active() {
        for s in TaskState::ALL {
            if s.is_terminal() {
                assert!(!s.is_active(), "{s}");
            }
        }
    }
}
