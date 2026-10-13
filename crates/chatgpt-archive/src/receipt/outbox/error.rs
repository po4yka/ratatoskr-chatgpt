//! Why an outbox pass could not progress.

/// Why an outbox pass could not progress; callers keep the durable row pending.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PublishError {
    /// The broker was unreachable or did not acknowledge the message. A `PubAck`
    /// timeout is indistinguishable from a permission denial: check the NATS server
    /// log for a Publish Violation.
    #[error(
        "the archive event broker did not acknowledge the message (check the NATS server log for a Publish Violation)"
    )]
    Broker(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The local queue could not be read or acknowledged as published.
    #[error("the archive event outbox database operation failed")]
    Database(#[source] sqlx::Error),
    /// A locally stored JSON payload could not be encoded for the bus.
    #[error("the archive event payload could not be encoded")]
    Encode(#[source] serde_json::Error),
    /// A row names an event type this service does not produce. This is a
    /// programming error and never a row to skip.
    #[error("the archive event outbox holds an event type this service does not produce")]
    UnknownEventType,
    /// A row does not hold the complete envelope of its declared event type.
    #[error("the archive event outbox holds a row that is not a complete envelope")]
    InvalidRow,
}

impl PublishError {
    pub(super) fn broker(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Broker(Box::new(error))
    }

    /// Whether the client could not even hand the message to its connection task, which only
    /// happens once the client is closed.
    pub(super) fn is_closed_connection(&self) -> bool {
        match self {
            Self::Broker(source) => source
                .downcast_ref::<async_nats::jetstream::context::PublishError>()
                .is_some_and(|error| {
                    error.kind() == async_nats::jetstream::context::PublishErrorKind::BrokenPipe
                }),
            _ => false,
        }
    }

    /// Whether retrying the pass can ever succeed. A row this service cannot have
    /// produced stays wrong until an operator repairs it, so the worker must stop
    /// instead of retrying forever.
    #[must_use]
    pub const fn is_fatal(&self) -> bool {
        matches!(self, Self::UnknownEventType | Self::InvalidRow)
    }
}
