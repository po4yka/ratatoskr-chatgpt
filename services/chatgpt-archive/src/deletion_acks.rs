//! Supervision of the consumer of Knowledge's deletion acknowledgements (XR-021 CONTRACTS.md
//! R5-11 rule 4 and S02 rule 5).
//!
//! The consumer is part of the event-bus surface: it is started only when the event bus is
//! configured, it reports its own readiness check, and a consumer that returns before an orderly
//! shutdown makes the service not ready and exit non-zero instead of logging and staying ready.
//! A broker that is down or a durable Edge has not provisioned yet keeps the check false and is
//! retried. Whether Knowledge ever answers is not a readiness input: the gauges on the operator
//! listener show a Knowledge outage.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use ratatoskr_chatgpt_archive::admin::RuntimeState;
use ratatoskr_chatgpt_archive::config::Config;
use ratatoskr_chatgpt_archive::deletion_ack::consumer::DeletionAckConsumer;
use ratatoskr_chatgpt_archive::persistence::Database;
use secrecy::ExposeSecret as _;
use tokio::sync::watch;

/// How long to wait before looking for the broker or the durable again.
const CONNECT_RETRY: Duration = Duration::from_secs(5);

/// The running consumer and whether it stopped without a shutdown request.
pub(crate) struct AckTask {
    pub(crate) handle: tokio::task::JoinHandle<()>,
    pub(crate) failed: Arc<AtomicBool>,
}

/// Starts the consumer when the event bus is configured.
pub(crate) fn start(
    database: &Database,
    config: &Config,
    state: &RuntimeState,
    fatal: watch::Sender<bool>,
    stopped: watch::Receiver<bool>,
) -> Option<AckTask> {
    let (endpoint, nkey_seed_path) = (
        config
            .receipt
            .event_bus_url
            .as_ref()?
            .expose_secret()
            .to_owned(),
        config.receipt.event_bus_nkey_seed_path.clone()?,
    );
    let ready = Arc::new(AtomicBool::new(false));
    let failed = Arc::new(AtomicBool::new(false));
    let probe = Arc::clone(&ready);
    state.register_check("deletion_ack_consumer", move || {
        let probe = Arc::clone(&probe);
        async move {
            probe
                .load(Ordering::Acquire)
                .then_some(())
                .ok_or_else(|| "deletion acknowledgement consumer unavailable".to_owned())
        }
    });
    let pool = database.pool().clone();
    let flag = Arc::clone(&failed);
    let handle = tokio::spawn(async move {
        supervise(
            &endpoint,
            &nkey_seed_path,
            &pool,
            &ready,
            &flag,
            &fatal,
            stopped,
        )
        .await;
    });
    Some(AckTask { handle, failed })
}

async fn wait_stopped(mut stopped: watch::Receiver<bool>) {
    // A dropped sender also means the service is going away.
    let _ = stopped.wait_for(|stopped| *stopped).await;
}

async fn supervise(
    endpoint: &str,
    nkey_seed_path: &std::path::Path,
    pool: &sqlx::PgPool,
    ready: &AtomicBool,
    failed: &AtomicBool,
    fatal: &watch::Sender<bool>,
    stopped: watch::Receiver<bool>,
) {
    loop {
        if *stopped.borrow() {
            return;
        }
        match DeletionAckConsumer::connect(endpoint, nkey_seed_path, pool).await {
            Ok(consumer) => {
                ready.store(true, Ordering::Release);
                let result = consumer.run(wait_stopped(stopped.clone())).await;
                ready.store(false, Ordering::Release);
                match result {
                    Ok(summary) => {
                        tracing::info!(?summary, "the deletion-acknowledgement consumer stopped");
                    }
                    Err(error) => {
                        tracing::error!(%error, "the deletion-acknowledgement consumer cannot continue");
                        failed.store(true, Ordering::Release);
                        let _ = fatal.send(true);
                    }
                }
                return;
            }
            Err(error) => {
                ready.store(false, Ordering::Release);
                tracing::warn!(%error, "the deletion-acknowledgement durable is not usable yet");
            }
        }
        let mut wake = stopped.clone();
        tokio::select! {
            () = tokio::time::sleep(CONNECT_RETRY) => {},
            _ = wake.wait_for(|stopped| *stopped) => return,
        }
    }
}
