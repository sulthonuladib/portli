//! Concurrent TCP port probing.
//!
//! The network side is hidden behind the [`PortProbe`] trait so the scanning
//! machinery can be pointed at a fake implementation. Actual probing uses
//! `TcpStream::connect_timeout`, which means "open" = a listener accepted (or
//! at least completed) the connection, "closed" = actively refused, and
//! "filtered" = no reply within the timeout.

use std::collections::VecDeque;
use std::fmt;
use std::io::ErrorKind;
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

/// Default per-connection timeout.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_millis(500);

/// Default number of worker threads used for a scan.
pub const DEFAULT_WORKERS: usize = 64;

/// Outcome of probing a single port.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortStatus {
    Open,
    Closed,
    Filtered,
}

impl PortStatus {
    /// Short human readable label used by the UI.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Filtered => "filtered",
        }
    }

    pub const fn is_open(self) -> bool {
        matches!(self, Self::Open)
    }
}

impl fmt::Display for PortStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// A single port result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanResult {
    pub port: u16,
    pub status: PortStatus,
}

/// Everything needed to run a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanParams {
    pub host: String,
    pub ports: Vec<u16>,
    pub timeout: Duration,
    pub workers: usize,
}

impl ScanParams {
    /// Build scan parameters using the default timeout and worker count.
    pub fn new(host: impl Into<String>, ports: Vec<u16>) -> Self {
        Self {
            host: host.into(),
            ports,
            timeout: DEFAULT_TIMEOUT,
            workers: DEFAULT_WORKERS,
        }
    }
}

/// Events emitted by a running scan, in arrival order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanEvent {
    /// The scan started and will report `total` ports.
    Started { total: usize },
    /// A port finished probing.
    Result(ScanResult),
    /// All ports have been probed.
    Finished,
    /// The scan could not start (e.g. host resolution failed).
    Error(String),
}

/// Abstraction over the actual probing so tests can supply a deterministic fake.
pub trait PortProbe: Send + Sync {
    fn probe(&self, addrs: &[SocketAddr], port: u16, timeout: Duration) -> PortStatus;
}

/// Real TCP probe.
#[derive(Debug, Default, Clone, Copy)]
pub struct TcpProbe;

impl PortProbe for TcpProbe {
    fn probe(&self, addrs: &[SocketAddr], port: u16, timeout: Duration) -> PortStatus {
        let mut status = PortStatus::Filtered;
        for addr in addrs {
            let target = SocketAddr::new(addr.ip(), port);
            match TcpStream::connect_timeout(&target, timeout) {
                Ok(_) => return PortStatus::Open,
                Err(err) if err.kind() == ErrorKind::ConnectionRefused => {
                    status = PortStatus::Closed;
                }
                Err(err) if err.kind() == ErrorKind::TimedOut => {
                    // Keep `Filtered` unless something else already proved the
                    // host is reachable and refusing.
                }
                Err(_) => status = PortStatus::Closed,
            }
        }
        status
    }
}

/// Spawn a background scan.
///
/// Results are streamed through `tx`; the returned thread is a coordinator that
/// joins the workers and emits [`ScanEvent::Finished`] when done. The caller is
/// expected to drain `rx` from its event loop via [`try_recv`](mpsc::Receiver::try_recv).
pub fn spawn_scan(params: ScanParams, probe: Arc<dyn PortProbe>, tx: mpsc::Sender<ScanEvent>) {
    thread::spawn(move || run_scan(params, probe, tx));
}

fn run_scan(params: ScanParams, probe: Arc<dyn PortProbe>, tx: mpsc::Sender<ScanEvent>) {
    let addrs: Vec<SocketAddr> = match (params.host.as_str(), 0_u16).to_socket_addrs() {
        Ok(iter) => iter.collect(),
        Err(err) => {
            let _ = tx.send(ScanEvent::Error(format!(
                "could not resolve `{}`: {err}",
                params.host
            )));
            return;
        }
    };

    if addrs.is_empty() {
        let _ = tx.send(ScanEvent::Error(format!(
            "`{}` did not resolve to any address",
            params.host
        )));
        return;
    }

    let total = params.ports.len();
    if total == 0 {
        let _ = tx.send(ScanEvent::Started { total: 0 });
        let _ = tx.send(ScanEvent::Finished);
        return;
    }

    let _ = tx.send(ScanEvent::Started { total });

    let queue = Arc::new(Mutex::new(
        params.ports.into_iter().collect::<VecDeque<_>>(),
    ));
    let worker_count = params.workers.clamp(1, total);

    let mut handles = Vec::with_capacity(worker_count);
    for _ in 0..worker_count {
        let queue = Arc::clone(&queue);
        let probe = Arc::clone(&probe);
        let tx = tx.clone();
        let addrs = addrs.clone();
        let timeout = params.timeout;

        handles.push(thread::spawn(move || {
            loop {
                // The lock is released before probing so workers do not
                // serialize on a potentially slow network call.
                let next = queue.lock().ok().and_then(|mut queue| queue.pop_front());
                let Some(port) = next else { break };

                let result = ScanResult {
                    port,
                    status: probe.probe(&addrs, port, timeout),
                };
                if tx.send(ScanEvent::Result(result)).is_err() {
                    // Receiver hung up: stop early.
                    break;
                }
            }
        }));
    }

    for handle in handles {
        let _ = handle.join();
    }
    let _ = tx.send(ScanEvent::Finished);
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeProbe;

    impl PortProbe for FakeProbe {
        fn probe(&self, _addrs: &[SocketAddr], port: u16, _timeout: Duration) -> PortStatus {
            if port.is_multiple_of(2) {
                PortStatus::Open
            } else {
                PortStatus::Closed
            }
        }
    }

    #[test]
    fn scans_every_port_and_finishes() {
        let params = ScanParams::new("127.0.0.1", vec![1, 2, 3, 4]);
        let (tx, rx) = mpsc::channel();
        run_scan(params, Arc::new(FakeProbe), tx);

        let mut results = Vec::new();
        let mut finished = false;
        while let Ok(event) = rx.recv() {
            match event {
                ScanEvent::Result(result) => results.push(result),
                ScanEvent::Finished => finished = true,
                _ => {}
            }
        }

        assert!(finished);
        results.sort_by_key(|r| r.port);
        assert_eq!(results.len(), 4);
        assert_eq!(results[0].status, PortStatus::Closed);
        assert_eq!(results[1].status, PortStatus::Open);
    }
}
