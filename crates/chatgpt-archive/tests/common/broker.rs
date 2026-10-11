//! A private `nats-server` for the tests that must observe the broker itself: its connection
//! count, or the authorization the deployed ACL gives the service.
//!
//! The server runs with `JetStream` and its monitoring endpoint. [`PrivateBroker::plain`] has no
//! authentication; [`PrivateBroker::with_service_stanza`] runs exactly the stanza shipped in
//! `deploy/nats/identity.conf` (only its public-key placeholder replaced by a generated key) next
//! to an administrator that stands in for Edge (XR-021 CONTRACTS.md S03 and R3-01).

use std::collections::HashSet;
use std::fmt::Write as _;
use std::io::{Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The prefix of the placeholder the stanza carries in place of the public nkey.
const PLACEHOLDER: &str = "UREPLACE_ME_WITH_THE_PUBLIC_NKEY_OF_RATATOSKR_CHATGPT_";

/// Ports this process already handed to a broker, so two tests never race for one port.
fn claimed_ports() -> &'static Mutex<HashSet<u16>> {
    static CLAIMED: OnceLock<Mutex<HashSet<u16>>> = OnceLock::new();
    CLAIMED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// A free loopback port. `CHATGPT_TEST_PORT_BASE` confines the search to twenty ports from that
/// base; without it the operating system picks one.
fn free_port() -> u16 {
    #[allow(
        clippy::disallowed_methods,
        reason = "the integration harness may confine its private broker to an operator-chosen port range"
    )]
    let base = std::env::var("CHATGPT_TEST_PORT_BASE")
        .ok()
        .and_then(|value| value.parse::<u16>().ok());
    let mut claimed = claimed_ports().lock().expect("the port registry is intact");
    let candidates: Box<dyn Iterator<Item = u16>> = match base {
        Some(base) => Box::new(base..base.saturating_add(20)),
        None => Box::new(std::iter::once(0)),
    };
    for candidate in candidates {
        if claimed.contains(&candidate) {
            continue;
        }
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", candidate)) {
            let port = listener.local_addr().expect("a bound address").port();
            claimed.insert(port);
            return port;
        }
    }
    panic!("no free loopback port for the private broker");
}

/// A running broker; the process and its files are removed on drop.
pub(crate) struct PrivateBroker {
    child: Child,
    directory: PathBuf,
    port: u16,
    monitor_port: u16,
    administrator_seed: String,
    service_seed_path: PathBuf,
}

/// The shipped stanza with its placeholder replaced by `public_key`.
fn shipped_stanza(public_key: &str) -> String {
    let stanza = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../deploy/nats/identity.conf"
    ))
    .expect("deploy/nats/identity.conf is shipped");
    let start = stanza
        .find(PLACEHOLDER)
        .expect("the shipped stanza carries the `ChatGPT` public-key placeholder");
    let length = stanza.get(start..).map_or(0, |rest| {
        rest.chars()
            .take_while(|character| character.is_ascii_uppercase() || *character == '_')
            .count()
    });
    format!(
        "{}{public_key}{}",
        stanza.get(..start).unwrap_or_default(),
        stanza.get(start + length..).unwrap_or_default()
    )
}

impl PrivateBroker {
    /// A broker with `JetStream`, monitoring and no authentication.
    pub(crate) fn plain() -> Self {
        Self::start(false)
    }

    /// A broker whose only service user is the shipped `ChatGPT` stanza, plus an administrator.
    pub(crate) fn with_service_stanza() -> Self {
        Self::start(true)
    }

    fn start(with_stanza: bool) -> Self {
        let port = free_port();
        let monitor_port = free_port();
        let directory =
            std::env::temp_dir().join(format!("chatgpt-broker-{}-{port}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("the broker directory is created");
        let administrator = nkeys::KeyPair::new_user();
        let service = nkeys::KeyPair::new_user();
        let mut config = format!(
            "listen: 127.0.0.1:{port}\nhttp: 127.0.0.1:{monitor_port}\n\
             jetstream {{ store_dir: \"{store}\" }}\n",
            store = directory.join("js").display(),
        );
        if with_stanza {
            write!(
                config,
                "authorization {{\n  users = [\n{stanza}\n    {{ nkey: {admin} }}\n  ]\n}}\n",
                stanza = shipped_stanza(&service.public_key()),
                admin = administrator.public_key(),
            )
            .expect("a string accepts formatted text");
        }
        let config_path = directory.join("nats.conf");
        std::fs::write(&config_path, config).expect("the broker config is written");
        let service_seed_path = directory.join("chatgpt.nkey");
        std::fs::write(&service_seed_path, service.seed().expect("a service seed"))
            .expect("the seed is written");
        let child = Command::new("nats-server")
            .arg("-c")
            .arg(&config_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("nats-server is on PATH (the gate copies it out of the pinned NATS image)");
        let broker = Self {
            child,
            directory,
            port,
            monitor_port,
            administrator_seed: administrator.seed().expect("an administrator seed"),
            service_seed_path,
        };
        broker.wait_until_listening();
        broker
    }

    /// The client URL of this broker.
    pub(crate) fn url(&self) -> String {
        format!("nats://127.0.0.1:{}", self.port)
    }

    /// The seed of the administrator that provisions topology (authorized broker only).
    pub(crate) fn administrator_seed(&self) -> &str {
        &self.administrator_seed
    }

    /// The file holding the `ChatGPT` identity's seed.
    pub(crate) fn service_seed_path(&self) -> &Path {
        &self.service_seed_path
    }

    /// How many client connections the server has accepted since it started, read from its
    /// monitoring endpoint.
    pub(crate) fn total_connections(&self) -> u64 {
        let mut stream = TcpStream::connect(("127.0.0.1", self.monitor_port))
            .expect("the monitoring port accepts connections");
        stream
            .write_all(b"GET /varz HTTP/1.0\r\n\r\n")
            .expect("the monitoring request is written");
        let mut response = String::new();
        stream
            .read_to_string(&mut response)
            .expect("the monitoring response is read");
        let body = response
            .split_once("\r\n\r\n")
            .map(|(_, body)| body)
            .expect("the monitoring response has a body");
        serde_json::from_str::<serde_json::Value>(body)
            .expect("the monitoring body is JSON")
            .get("total_connections")
            .and_then(serde_json::Value::as_u64)
            .expect("varz reports total_connections")
    }

    /// Stops the server process but keeps its port reserved.
    pub(crate) fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn wait_until_listening(&self) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if TcpStream::connect(("127.0.0.1", self.port)).is_ok()
                && TcpStream::connect(("127.0.0.1", self.monitor_port)).is_ok()
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("the private broker did not start listening");
    }
}

impl Drop for PrivateBroker {
    fn drop(&mut self) {
        self.stop();
        let _ = std::fs::remove_dir_all(&self.directory);
        if let Ok(mut claimed) = claimed_ports().lock() {
            claimed.remove(&self.port);
            claimed.remove(&self.monitor_port);
        }
    }
}
