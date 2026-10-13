//! Deterministic pauses and one fault inside the pump, for tests of the first-publication
//! protocol (XR-021 CONTRACTS.md R6-03 rule 7).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Notify;

/// A named point in the first publication of a row where a [`PublishProbe`] can hold the pump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PublishPause {
    /// The candidates are chosen and no row lock has been taken yet.
    AfterCandidates,
    /// The broker acknowledged the message; the row lock is held and the sequence is not yet
    /// written or committed.
    AfterPubAck,
}

/// What a [`PublishProbe`] does to the pump. Production never sets one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Plan {
    Pause(PublishPause),
    FailCommitOnce,
}

/// Holds the pump at a named point, or fails one commit after the broker acknowledged.
///
/// This is a test seam like [`crate::privacy_deletion::EraseGate`]: production never sets one. A
/// test waits on [`PublishProbe::reached`], does whatever it needs to do while the pump is held,
/// and calls [`PublishProbe::resume`]. A pause holds the pump once; later passes run freely.
#[derive(Debug)]
pub struct PublishProbe {
    plan: Plan,
    armed: AtomicBool,
    reached: Notify,
    resume: Notify,
}

impl PublishProbe {
    /// A probe that holds the pump the first time it reaches `pause`.
    #[must_use]
    pub fn pausing(pause: PublishPause) -> Arc<Self> {
        Self::new(Plan::Pause(pause))
    }

    /// A probe that makes the commit of the first acknowledged publication fail, so the broker
    /// holds a message whose sequence the database never recorded.
    #[must_use]
    pub fn failing_commit_once() -> Arc<Self> {
        Self::new(Plan::FailCommitOnce)
    }

    fn new(plan: Plan) -> Arc<Self> {
        Arc::new(Self {
            plan,
            armed: AtomicBool::new(true),
            reached: Notify::new(),
            resume: Notify::new(),
        })
    }

    /// Completes once the pump is held at the probe's pause.
    pub async fn reached(&self) {
        self.reached.notified().await;
    }

    /// Lets the held pump continue.
    pub fn resume(&self) {
        self.resume.notify_one();
    }

    /// Called by the pump at `point`; holds it only at the probe's own pause, and only once.
    pub(super) async fn pause_at(&self, point: PublishPause) {
        if self.plan == Plan::Pause(point) && self.armed.swap(false, Ordering::AcqRel) {
            self.reached.notify_one();
            self.resume.notified().await;
        }
    }

    /// Called by the pump before it commits an acknowledged publication; true once for a probe
    /// built with [`PublishProbe::failing_commit_once`].
    pub(super) fn take_commit_failure(&self) -> bool {
        self.plan == Plan::FailCommitOnce && self.armed.swap(false, Ordering::AcqRel)
    }
}
