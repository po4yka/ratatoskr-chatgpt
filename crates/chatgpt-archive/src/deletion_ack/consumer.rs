//! The supervised consumer of the Edge-provisioned durable of acknowledgements.

use std::future::Future;
use std::path::Path;
use std::time::Duration;

use async_nats::jetstream;
use async_nats::jetstream::AckKind;
use async_nats::jetstream::consumer::pull::MessagesErrorKind;
use futures_util::StreamExt as _;
use ratatoskr_event_envelope::{EventEnvelope, EventPayload as _};
use ratatoskr_operation_contracts::KnowledgeDeletionAcknowledged;
use sqlx::PgPool;

use super::{AckApplication, acknowledge};
use crate::receipt::outbox::connect_options;

/// The events stream Edge provisions.
pub const EVENTS_STREAM: &str = "ratatoskr_events";

/// The Edge-provisioned durable of this service.
pub const DURABLE: &str = "ratatoskr_chatgpt_deletion_acks";

/// The only subject the durable is filtered to.
pub const FILTER_SUBJECT: &str = "evt.knowledge.deletion.acknowledged.v1";

/// Totals from one consumer session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AckSummary {
    /// Acknowledgements that stopped the resend of a tombstone as applied.
    pub applied: u64,
    /// Acknowledgements that stopped the resend of a tombstone as refused.
    pub refused: u64,
    /// Acknowledgements that named no tombstone of this service (acknowledged and ignored).
    pub ignored: u64,
    /// Messages that were not a valid acknowledgement (terminated).
    pub terminated: u64,
    /// Messages left for redelivery after a database failure.
    pub retried: u64,
}

/// Why the consumer could not start or had to stop.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DeletionAckError {
    /// The broker refused the connection, the stream or the durable lookup, or the durable does
    /// not match its specification.
    #[error("the deletion-acknowledgement consumer cannot use JetStream: {0}")]
    Bus(String),
    /// The message stream ended without a shutdown request.
    #[error("the deletion-acknowledgement consumer stopped without a shutdown request")]
    Stopped,
}

/// The acknowledgement wait Edge provisions for the durable.
const ACK_WAIT: Duration = Duration::from_secs(30);

/// How long a transiently failed acknowledgement waits before redelivery.
const NAK_DELAY: Duration = Duration::from_secs(2);

/// Pause after a recoverable stream error, so a broken connection cannot spin the loop.
const RETRY_PAUSE: Duration = Duration::from_secs(1);

/// The only producer an acknowledgement may come from.
const KNOWLEDGE: &str = "ratatoskr-knowledge";

fn bus(error: impl std::fmt::Display) -> DeletionAckError {
    DeletionAckError::Bus(error.to_string())
}

/// Opens the Edge-provisioned durable and verifies its filter, ack policy and ack wait. This
/// service never creates a consumer: that could widen a compromised identity's filter.
async fn verified_consumer(
    context: &jetstream::Context,
) -> Result<jetstream::consumer::Consumer<jetstream::consumer::pull::Config>, DeletionAckError> {
    let consumer = context
        .get_consumer_from_stream(DURABLE, EVENTS_STREAM)
        .await
        .map_err(bus)?;
    let config = &consumer.cached_info().config;
    if config.durable_name.as_deref() != Some(DURABLE)
        || config.filter_subject != FILTER_SUBJECT
        || config.ack_policy != jetstream::consumer::AckPolicy::Explicit
        || config.ack_wait != ACK_WAIT
        || config.deliver_subject.is_some()
    {
        return Err(DeletionAckError::Bus(
            "the Edge-provisioned deletion-acknowledgement durable is mismatched".to_owned(),
        ));
    }
    Ok(consumer)
}

/// What one message asks of the consumer.
enum Handling {
    /// Not a valid acknowledgement from Knowledge: redelivery cannot fix it.
    Terminate,
    /// A valid acknowledgement and what it did to the outbox.
    Applied(AckApplication),
    /// The database failed: redeliver.
    Retry,
}

async fn handle(pool: &PgPool, payload: &[u8]) -> Handling {
    let Ok(envelope) = EventEnvelope::from_json(payload) else {
        return Handling::Terminate;
    };
    if envelope.event_type.to_wire() != KnowledgeDeletionAcknowledged::EVENT_TYPE
        || envelope.producer.as_str() != KNOWLEDGE
    {
        return Handling::Terminate;
    }
    let Ok(acknowledgement) = envelope.payload_as::<KnowledgeDeletionAcknowledged>() else {
        return Handling::Terminate;
    };
    match acknowledge(pool, &acknowledgement).await {
        Ok(application) => Handling::Applied(application),
        Err(error) => {
            tracing::error!(%error, "recording a deletion acknowledgement failed");
            Handling::Retry
        }
    }
}

async fn settle(message: &jetstream::Message, kind: AckKind) {
    if let Err(error) = message.ack_with(kind).await {
        tracing::warn!(%error, "the deletion-acknowledgement settlement failed");
    }
}

/// A connection to the broker with the verified durable open for fetching.
pub struct DeletionAckConsumer {
    client: async_nats::Client,
    messages: jetstream::consumer::pull::Stream,
    pool: PgPool,
}

impl std::fmt::Debug for DeletionAckConsumer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeletionAckConsumer")
            .finish_non_exhaustive()
    }
}

impl DeletionAckConsumer {
    /// Connects with the service identity and the inbox prefix its ACL stanza allows, verifies the
    /// Edge-provisioned durable and opens its message stream.
    ///
    /// # Errors
    ///
    /// Returns [`DeletionAckError::Bus`] when the broker or the durable is unusable.
    pub async fn connect(
        endpoint: &str,
        nkey_seed_path: &Path,
        pool: &PgPool,
    ) -> Result<Self, DeletionAckError> {
        let seed = tokio::fs::read_to_string(nkey_seed_path)
            .await
            .map_err(bus)?;
        let client = connect_options(&seed)
            .connect(endpoint)
            .await
            .map_err(bus)?;
        let context = jetstream::new(client.clone());
        let consumer = verified_consumer(&context).await?;
        let messages = consumer.messages().await.map_err(bus)?;
        Ok(Self {
            client,
            messages,
            pool: pool.clone(),
        })
    }

    /// Runs the pull loop until `shutdown` resolves.
    ///
    /// A message that is not a valid acknowledgement from Knowledge cannot be attributed, so it
    /// is terminated; an acknowledgement that names no tombstone of this service is acknowledged
    /// and ignored; a database failure is redelivered after a short delay.
    ///
    /// # Errors
    ///
    /// Returns [`DeletionAckError`] when the durable is deleted or the message stream ends
    /// without a shutdown request.
    pub async fn run(
        mut self,
        shutdown: impl Future<Output = ()> + Send,
    ) -> Result<AckSummary, DeletionAckError> {
        let mut summary = AckSummary::default();
        tokio::pin!(shutdown);
        let outcome = loop {
            let next = tokio::select! {
                biased;
                () = &mut shutdown => break Ok(summary),
                next = self.messages.next() => next,
            };
            let message = match next {
                None => break Err(DeletionAckError::Stopped),
                Some(Ok(message)) => message,
                Some(Err(error)) => {
                    if matches!(
                        error.kind(),
                        MessagesErrorKind::ConsumerDeleted | MessagesErrorKind::PushBasedConsumer
                    ) {
                        break Err(bus(error));
                    }
                    tracing::warn!(%error, "the deletion-acknowledgement stream reported a recoverable error");
                    tokio::select! {
                        biased;
                        () = &mut shutdown => break Ok(summary),
                        () = tokio::time::sleep(RETRY_PAUSE) => {}
                    }
                    continue;
                }
            };
            match handle(&self.pool, &message.payload).await {
                Handling::Terminate => {
                    summary.terminated += 1;
                    tracing::warn!(
                        class = "not_a_knowledge_acknowledgement",
                        "a deletion-acknowledgement message was terminated"
                    );
                    settle(&message, AckKind::Term).await;
                }
                Handling::Applied(application) => {
                    match application {
                        AckApplication::Applied => summary.applied += 1,
                        AckApplication::Refused => summary.refused += 1,
                        AckApplication::Unmatched => summary.ignored += 1,
                    }
                    settle(&message, AckKind::Ack).await;
                }
                Handling::Retry => {
                    summary.retried += 1;
                    settle(&message, AckKind::Nak(Some(NAK_DELAY))).await;
                }
            }
        };
        let _ = self.client.drain().await;
        outcome
    }
}

/// Connects and runs until `shutdown` resolves.
///
/// # Errors
///
/// Returns [`DeletionAckError`] when the broker or the durable is unusable, or the message stream
/// ends without a shutdown request.
pub async fn consume_deletion_acks(
    endpoint: &str,
    nkey_seed_path: &Path,
    pool: &PgPool,
    shutdown: impl Future<Output = ()> + Send,
) -> Result<AckSummary, DeletionAckError> {
    DeletionAckConsumer::connect(endpoint, nkey_seed_path, pool)
        .await?
        .run(shutdown)
        .await
}
