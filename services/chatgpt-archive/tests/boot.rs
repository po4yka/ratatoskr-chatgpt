//! Boot contract tests: the acceptance that the service runs locally.
//!
//! Spawns the built binary as a child process with a minimal environment and
//! asserts the observable startup contract end to end.

// Test bodies fail through `panic!` by design; assertions are the contract.
#![allow(clippy::panic, reason = "test failures report through panics")]

use std::io::{BufRead as _, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A free loopback port for this run: bind, read, release.
fn free_loopback_port() -> std::io::Result<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

struct ServiceProcess {
    child: Child,
    stdout_lines: std::sync::mpsc::Receiver<String>,
}

impl Drop for ServiceProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn_service(
    admin_port: u16,
    blob_root: &std::path::Path,
) -> Result<ServiceProcess, Box<dyn std::error::Error>> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_ratatoskr-chatgpt-archive"))
        .env(
            "RATATOSKR__ADMIN__LISTEN_ADDRESS",
            format!("127.0.0.1:{admin_port}"),
        )
        .env("RATATOSKR__STORAGE__BLOB_ROOT", blob_root)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let Some(stdout) = child.stdout.take() else {
        return Err("stdout was not piped".into());
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    Ok(ServiceProcess {
        child,
        stdout_lines: receiver,
    })
}

async fn wait_for_live(admin_port: u16) -> Result<reqwestless::Status, String> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if Instant::now() > deadline {
            return Err("the service never answered /health/live within 20s".to_owned());
        }
        match tokio::time::timeout(
            Duration::from_millis(500),
            tokio::net::TcpStream::connect(("127.0.0.1", admin_port)),
        )
        .await
        {
            Ok(Ok(_)) => return reqwestless::probe_live(admin_port).await,
            _ => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

/// Minimal HTTP probing without a client dependency: hand-rolled requests.
mod reqwestless {
    use std::io::{Read as _, Write as _};
    use std::time::Duration;

    pub(crate) struct Status {
        pub code: u16,
        pub body: String,
    }

    fn request(port: u16, target: &str) -> Result<Status, String> {
        use std::net::TcpStream;

        let mut stream =
            TcpStream::connect(("127.0.0.1", port)).map_err(|error| format!("connect: {error}"))?;
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|error| format!("read timeout: {error}"))?;
        stream
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|error| format!("write timeout: {error}"))?;
        let request =
            format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
        stream
            .write_all(request.as_bytes())
            .map_err(|error| format!("write: {error}"))?;
        let mut buffer = Vec::new();
        stream
            .read_to_end(&mut buffer)
            .map_err(|error| format!("read: {error}"))?;
        if buffer.is_empty() {
            return Err("empty response".to_owned());
        }
        let text = String::from_utf8_lossy(&buffer);
        let code: u16 = text
            .split_whitespace()
            .nth(1)
            .ok_or_else(|| format!("no status line in {text:?}"))?
            .parse()
            .map_err(|error| format!("status parse: {error} from {text:?}"))?;
        let body = text
            .split_once("\r\n\r\n")
            .map(|(_, body)| body.to_owned())
            .unwrap_or_default();
        Ok(Status { code, body })
    }

    pub(crate) async fn probe_live(port: u16) -> Result<Status, String> {
        tokio::task::spawn_blocking(move || request(port, "/health/live"))
            .await
            .map_err(|error| format!("the blocking probe task failed: {error}"))?
    }

    pub(crate) async fn probe_ready(port: u16) -> Result<Status, String> {
        tokio::task::spawn_blocking(move || request(port, "/health/ready"))
            .await
            .map_err(|error| format!("the blocking probe task failed: {error}"))?
    }

    /// A raw `POST /exports` with the receipt's required headers.
    pub(crate) async fn post_export(
        port: u16,
        token: &str,
        body: &'static [u8],
    ) -> Result<Status, String> {
        let head = format!(
            "POST /exports HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nX-Ratatoskr-Acquisition: consumer_export\r\nContent-Type: application/zip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        tokio::task::spawn_blocking(move || {
            use std::io::{Read as _, Write as _};
            use std::net::TcpStream;

            let mut stream = TcpStream::connect(("127.0.0.1", port))
                .map_err(|error| format!("connect: {error}"))?;
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .map_err(|error| format!("read timeout: {error}"))?;
            stream
                .set_write_timeout(Some(Duration::from_secs(10)))
                .map_err(|error| format!("write timeout: {error}"))?;
            stream
                .write_all(head.as_bytes())
                .map_err(|error| format!("write: {error}"))?;
            stream
                .write_all(body)
                .map_err(|error| format!("write body: {error}"))?;
            let mut buffer = Vec::new();
            stream
                .read_to_end(&mut buffer)
                .map_err(|error| format!("read: {error}"))?;
            if buffer.is_empty() {
                return Err("empty response".to_owned());
            }
            let text = String::from_utf8_lossy(&buffer);
            let code: u16 = text
                .split_whitespace()
                .nth(1)
                .ok_or_else(|| format!("no status line in {text:?}"))?
                .parse()
                .map_err(|error| format!("status parse: {error} from {text:?}"))?;
            let response_body = text
                .split_once("\r\n\r\n")
                .map(|(_, body)| body.to_owned())
                .unwrap_or_default();
            Ok(Status {
                code,
                body: response_body,
            })
        })
        .await
        .map_err(|error| format!("the blocking post task failed: {error}"))?
    }
}

/// The full local acceptance: live answers, ready answers without a database,
/// and stdout carries structured JSON lines with a level field.
#[tokio::test(flavor = "multi_thread")]
async fn boot_serves_health_and_logs_structured_lines() -> Result<(), Box<dyn std::error::Error>> {
    let admin_port = free_loopback_port()?;
    let blob_root = tempfile::tempdir()?;
    let service = spawn_service(admin_port, blob_root.path())?;

    let live = wait_for_live(admin_port)
        .await
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    assert_eq!(live.code, 200, "liveness must be 200");
    let json = serde_json::from_str::<serde_json::Value>(&live.body)?;
    assert_eq!(json.get("state").and_then(|s| s.as_str()), Some("live"));

    let ready = reqwestless::probe_ready(admin_port)
        .await
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    assert_eq!(ready.code, 200, "no database configured means ready");
    let json =
        serde_json::from_str::<serde_json::Value>(&ready.body).expect("ready body must be JSON");
    assert_eq!(json.get("state").and_then(|s| s.as_str()), Some("ready"));

    // Structured output: every stdout line so far parses as JSON with a level.
    let mut seen_startup_line = false;
    while let Ok(line) = service.stdout_lines.try_recv() {
        let record = serde_json::from_str::<serde_json::Value>(&line).unwrap_or_else(|error| {
            panic!("every stdout line must be JSON, got {line:?}: {error}")
        });
        assert!(
            record.get("level").is_some(),
            "every stdout line must carry a level field: {line}"
        );
        // tracing-subscriber nests the event message under `fields`.
        let message = record.get("message").and_then(|m| m.as_str()).or_else(|| {
            record
                .get("fields")
                .and_then(|fields| fields.get("message"))
                .and_then(|m| m.as_str())
        });
        if message == Some("starting") {
            seen_startup_line = true;
        }
    }
    assert!(
        seen_startup_line,
        "the process must log one structured startup record"
    );
    Ok(())
}

/// A synthetic archive body: opaque bytes at the receipt stage.
const BODY: &[u8] = b"PK\x03\x04 ratatoskr synthetic export fixture";

/// The full receipt acceptance: with staging, tenant tokens, blob root and a
/// database configured, `POST /exports` stores fresh content once and then
/// answers duplicate for the identical re-upload. Skips without the database
/// URL exactly like the schema integration tests.
#[tokio::test(flavor = "multi_thread")]
async fn receipt_route_serves_end_to_end_when_configured() -> Result<(), Box<dyn std::error::Error>>
{
    #[allow(
        clippy::disallowed_methods,
        reason = "the boot harness reads the database URL the runner exports"
    )]
    let database_url = std::env::var("CHATGPT_TEST_DATABASE_URL").ok();
    let Some(database_url) = database_url.filter(|url| !url.trim().is_empty()) else {
        eprintln!("skipping: CHATGPT_TEST_DATABASE_URL is not set");
        return Ok(());
    };

    let admin_port = free_loopback_port()?;
    let blob_root = tempfile::tempdir()?;
    let staging_root = tempfile::tempdir()?;
    let tenant_tokens = format!("e2e-token=acc-e2e-{admin_port}");
    let mut command = Command::new(env!("CARGO_BIN_EXE_ratatoskr-chatgpt-archive"));
    let service = {
        let child = command
            .env(
                "RATATOSKR__ADMIN__LISTEN_ADDRESS",
                format!("127.0.0.1:{admin_port}"),
            )
            .env("RATATOSKR__STORAGE__BLOB_ROOT", blob_root.path())
            .env(
                "RATATOSKR__STORAGE__RECEIPT_STAGING_ROOT",
                staging_root.path(),
            )
            .env("RATATOSKR__RECEIPT__TENANT_TOKENS", tenant_tokens)
            .env("RATATOSKR__STORAGE__DATABASE_URL", database_url)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?;
        ServiceProcess {
            child,
            stdout_lines: std::sync::mpsc::channel().1,
        }
    };
    wait_for_live(admin_port)
        .await
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;

    let first = reqwestless::post_export(admin_port, "e2e-token", BODY).await?;
    assert_eq!(
        first.code, 201,
        "fresh content answers stored: {}",
        first.body
    );
    let json = serde_json::from_str::<serde_json::Value>(&first.body)?;
    assert_eq!(json.get("outcome").and_then(|v| v.as_str()), Some("stored"));

    let second = reqwestless::post_export(admin_port, "e2e-token", BODY).await?;
    assert_eq!(second.code, 200, "identical content answers duplicate");
    let json = serde_json::from_str::<serde_json::Value>(&second.body)?;
    assert_eq!(
        json.get("outcome").and_then(|v| v.as_str()),
        Some("duplicate")
    );
    drop(service);
    Ok(())
}
