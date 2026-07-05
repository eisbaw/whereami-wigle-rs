//! End-to-end coverage for the typed CLI protocol-version gate.
//!
//! A bounded one-shot TCP peer supplies a deliberately newer response. Human
//! mode must reject it before rendering defaulted coordinates, while `--json`
//! deliberately preserves the daemon line byte-for-byte.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::process::{Command, Output};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const NEWER_RESPONSE: &str = r#"{"ok":true,"v":2}"#;

struct OneShotServer {
    address: SocketAddr,
    handle: Option<JoinHandle<Result<String, String>>>,
}

impl OneShotServer {
    fn spawn(response: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind one-shot test server");
        let address = listener.local_addr().expect("read one-shot server address");
        listener
            .set_nonblocking(true)
            .expect("make test accept loop bounded");

        let handle = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if Instant::now() >= deadline {
                            return Err("whereami did not connect within 5 seconds".to_string());
                        }
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => return Err(format!("accepting whereami connection: {error}")),
                }
            };

            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .map_err(|error| format!("setting server read timeout: {error}"))?;
            stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .map_err(|error| format!("setting server write timeout: {error}"))?;

            let mut request = String::new();
            stream
                .read_to_string(&mut request)
                .map_err(|error| format!("reading whereami request: {error}"))?;
            stream
                .write_all(response.as_bytes())
                .and_then(|()| stream.write_all(b"\n"))
                .and_then(|()| stream.flush())
                .map_err(|error| format!("writing one-shot response: {error}"))?;
            Ok(request)
        });

        Self {
            address,
            handle: Some(handle),
        }
    }

    fn run_whereami(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_whereami"))
            .args(["--timeout-secs", "2"])
            .args(args)
            .env_remove("WHEREAMI_SOCKET")
            .env("WHEREAMI_ADDR", self.address.to_string())
            .output()
            .expect("run whereami against one-shot daemon")
    }

    fn finish(mut self) -> String {
        let handle = self.handle.take().expect("server handle present");
        handle
            .join()
            .expect("one-shot server thread panicked")
            .expect("one-shot server failed")
    }
}

impl Drop for OneShotServer {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[test]
fn human_locate_rejects_newer_protocol_without_fabricating_coordinates() {
    let server = OneShotServer::spawn(NEWER_RESPONSE);
    let address = server.address;
    let output = server.run_whereami(&["locate"]);
    let request = server.finish();

    assert_eq!(request, "{\"cmd\":\"locate\"}\n");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "human failure wrote to stdout");
    assert!(!output.stdout.windows(8).any(|bytes| bytes == b"0.000000"));
    assert_eq!(
        String::from_utf8(output.stderr).expect("stderr must be UTF-8"),
        format!(
            "error: whereamid at TCP endpoint {address} uses unsupported protocol v2; this client \
             supports through v1; upgrade the whereami client or connect to a daemon using supported \
             protocol v1\n"
        )
    );
}

#[test]
fn json_locate_passes_newer_protocol_through_byte_identically() {
    let server = OneShotServer::spawn(NEWER_RESPONSE);
    let output = server.run_whereami(&["--json", "locate"]);
    let request = server.finish();

    assert_eq!(request, "{\"cmd\":\"locate\"}\n");
    assert!(
        output.status.success(),
        "--json failed: status={:?}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty(), "--json wrote to stderr");
    assert_eq!(output.stdout, format!("{NEWER_RESPONSE}\n").as_bytes());
}
