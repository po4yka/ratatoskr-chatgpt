//! A deterministic pause inside the eraser, for tests of the blob lock protocol.

use std::sync::Arc;

use tokio::sync::Notify;

/// Holds the eraser at the point where it owns a digest lock, has re-checked that nothing
/// references the blob, and has not yet erased it.
///
/// This is a fault seam like [`super::FinalizationFault`]: production never sets one. A test
/// waits on [`EraseGate::reached`], does whatever it needs to do while the eraser holds the
/// lock, and calls [`EraseGate::resume`].
#[derive(Debug)]
pub struct EraseGate {
    digest_hex: String,
    reached: Notify,
    resume: Notify,
}

impl EraseGate {
    /// A gate that stops the eraser at the blob whose digest is `digest_hex`.
    #[must_use]
    pub fn for_digest(digest_hex: &str) -> Arc<Self> {
        Arc::new(Self {
            digest_hex: digest_hex.to_owned(),
            reached: Notify::new(),
            resume: Notify::new(),
        })
    }

    /// Completes once the eraser holds the digest lock after its re-check.
    pub async fn reached(&self) {
        self.reached.notified().await;
    }

    /// Lets the held eraser continue.
    pub fn resume(&self) {
        self.resume.notify_one();
    }

    /// Called by the eraser; pauses only for the gate's own digest.
    pub(super) async fn pause_if(&self, digest_hex: &str) {
        if self.digest_hex == digest_hex {
            self.reached.notify_one();
            self.resume.notified().await;
        }
    }
}
