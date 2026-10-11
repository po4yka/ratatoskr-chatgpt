//! Log capture for the tests that assert what an operator would read.

use std::sync::Arc;

/// Log lines, keyed by the thread that emitted them.
///
/// A scoped subscriber would race with the other tests of this binary for the tracing callsite
/// cache, so one global subscriber routes each event to the buffer of the emitting thread. A
/// `#[tokio::test]` runs its whole request on its own thread, so a test reads exactly its own
/// lines.
#[derive(Clone, Default)]
pub(crate) struct ThreadLogs(
    Arc<std::sync::Mutex<std::collections::HashMap<std::thread::ThreadId, Vec<u8>>>>,
);

pub(crate) struct ThreadLogWriter(ThreadLogs);

impl std::io::Write for ThreadLogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        (self.0)
            .0
            .lock()
            .expect("log buffer")
            .entry(std::thread::current().id())
            .or_default()
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for ThreadLogs {
    type Writer = ThreadLogWriter;

    fn make_writer(&'a self) -> ThreadLogWriter {
        ThreadLogWriter(self.clone())
    }
}

impl ThreadLogs {
    /// The one capture of this binary, installed as the global subscriber on first use.
    pub(crate) fn install() -> &'static Self {
        static LOGS: std::sync::OnceLock<ThreadLogs> = std::sync::OnceLock::new();
        LOGS.get_or_init(|| {
            let logs = ThreadLogs::default();
            tracing::subscriber::set_global_default(
                tracing_subscriber::fmt()
                    .with_writer(logs.clone())
                    .with_ansi(false)
                    .finish(),
            )
            .expect("this binary installs one global subscriber");
            logs
        })
    }

    /// Returns and clears the lines the current thread has emitted.
    pub(crate) fn take_lines(&self) -> Vec<String> {
        let bytes = self
            .0
            .lock()
            .expect("log buffer")
            .remove(&std::thread::current().id())
            .unwrap_or_default();
        String::from_utf8_lossy(&bytes)
            .lines()
            .map(str::to_owned)
            .collect()
    }
}
