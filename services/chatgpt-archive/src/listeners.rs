//! The two serving planes: the operator listener and the optional receipt listener.

use std::net::SocketAddr;

use axum::Router;
use tokio::net::TcpListener;
use tokio::sync::watch;

use crate::{ServiceError, shutdown_signal};

/// The shutdown wiring shared by both planes.
pub(crate) struct Stop {
    /// Sending `true` stops both planes and every background task.
    pub(crate) trigger: watch::Sender<bool>,
    /// Observed by both planes to begin draining.
    pub(crate) observed: watch::Receiver<bool>,
    /// Fires when a background task failed fatally.
    pub(crate) fatal: watch::Receiver<bool>,
}

/// Binds and serves the operator plane and, when the receipt route is mounted, the receipt plane.
///
/// The receipt plane is a second listener and never a second set of routes on the operator one
/// (XR-021 CONTRACTS.md R2-01). Both addresses are bound before either plane serves, and both stop
/// on the same signal.
pub(crate) async fn serve_planes(
    operator: (SocketAddr, Router),
    receipt: Option<(SocketAddr, Router)>,
    stop: Stop,
) -> Result<(), ServiceError> {
    let Stop {
        trigger,
        observed,
        mut fatal,
    } = stop;
    let operator_listener = TcpListener::bind(operator.0)
        .await
        .map_err(ServiceError::Bind)?;
    let receipt_listener = match receipt {
        Some((address, routes)) => Some((
            TcpListener::bind(address)
                .await
                .map_err(ServiceError::Bind)?,
            routes,
        )),
        None => None,
    };

    tracing::info!(
        address = %operator.0,
        receipt_address = receipt_listener
            .as_ref()
            .and_then(|(listener, _)| listener.local_addr().ok())
            .map(|address| address.to_string()),
        "listening"
    );

    let stop_trigger = tokio::spawn(async move {
        tokio::select! {
            () = shutdown_signal() => {},
            _ = fatal.changed() => {},
        }
        let _ = trigger.send(true);
    });
    let served = tokio::try_join!(
        serve_until_stopped(operator_listener, operator.1, observed.clone()),
        async {
            match receipt_listener {
                Some((listener, routes)) => serve_until_stopped(listener, routes, observed).await,
                None => Ok(()),
            }
        },
    );
    stop_trigger.abort();
    served.map(|((), ())| ())
}

async fn serve_until_stopped(
    listener: TcpListener,
    app: Router,
    mut stop: watch::Receiver<bool>,
) -> Result<(), ServiceError> {
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            // A dropped sender also means the service is going away.
            let _ = stop.wait_for(|stopped| *stopped).await;
        })
        .await
        .map_err(ServiceError::Serve)
}
