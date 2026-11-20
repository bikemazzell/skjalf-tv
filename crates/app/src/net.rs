//! HTTP adapter: [`HttpFetcher`] backed by `ureq` (rustls).

use std::io::{self, Read};
use std::sync::Arc;
use std::time::Duration;

use skjalf_core::model::DEFAULT_USER_AGENT;
use skjalf_core::xtream::{FetchError, FetchLimits, HttpFetcher};

/// Thin blocking fetcher; `ignore_ssl` skips certificate verification
/// (common for IPTV provider hosts with self-signed certs).
#[derive(Debug, Clone)]
pub struct UreqFetcher {
    ignore_ssl: bool,
    max_response_bytes: usize,
    request_timeout: Duration,
    user_agent: Arc<str>,
}

impl UreqFetcher {
    /// A fetcher that names the app ([`DEFAULT_USER_AGENT`]).
    pub fn new(ignore_ssl: bool) -> Self {
        Self {
            ignore_ssl,
            max_response_bytes: FetchLimits::DEFAULT.max_bytes,
            request_timeout: FetchLimits::DEFAULT.timeout,
            user_agent: DEFAULT_USER_AGENT.into(),
        }
    }

    pub fn with_user_agent(self, user_agent: &str) -> Self {
        Self {
            user_agent: user_agent.into(),
            ..self
        }
    }

    fn agent(&self) -> ureq::Agent {
        // Connect bound short, overall bound generous: a server that accepts
        // and then stalls must not hang setup or EPG workers forever.
        let mut builder = ureq::AgentBuilder::new()
            .user_agent(&self.user_agent)
            .timeout_connect(Duration::from_secs(10))
            .timeout(self.request_timeout);
        if self.ignore_ssl {
            let config = rustls::ClientConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .expect("ring supports all safe protocol versions")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAnyServerCert))
            .with_no_client_auth();
            builder = builder.tls_config(Arc::new(config));
        }
        builder.build()
    }
}

impl HttpFetcher for UreqFetcher {
    fn get(&self, url: &str) -> Result<String, FetchError> {
        self.fetch(url)
    }

    fn get_with(&self, url: &str, limits: FetchLimits) -> Result<String, FetchError> {
        Self {
            max_response_bytes: limits.max_bytes,
            request_timeout: limits.timeout,
            ..self.clone()
        }
        .fetch(url)
    }
}

impl UreqFetcher {
    fn fetch(&self, url: &str) -> Result<String, FetchError> {
        let resp = self.agent().get(url).call().map_err(|e| match e {
            ureq::Error::Status(status, _) => FetchError(format!("HTTP status {status}.")),
            ureq::Error::Transport(transport) if transport_timed_out(&transport) => {
                FetchError("Network request timed out.".into())
            }
            ureq::Error::Transport(_) => FetchError("Network connection failed.".into()),
        })?;
        let mut bytes = Vec::new();
        resp.into_reader()
            .take(self.max_response_bytes.saturating_add(1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| {
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) {
                    FetchError("Server response timed out.".into())
                } else {
                    FetchError("Could not read the server response.".into())
                }
            })?;
        if bytes.len() > self.max_response_bytes {
            let limit = if self.max_response_bytes.is_multiple_of(1024 * 1024) {
                format!("{} MiB", self.max_response_bytes / (1024 * 1024))
            } else {
                format!("{} bytes", self.max_response_bytes)
            };
            return Err(FetchError(format!(
                "Server response exceeds the {limit} limit."
            )));
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

fn transport_timed_out(transport: &ureq::Transport) -> bool {
    let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(transport);
    while let Some(error) = cause {
        if error
            .downcast_ref::<io::Error>()
            .is_some_and(|io_error| io_error.kind() == io::ErrorKind::TimedOut)
        {
            return true;
        }
        cause = error.source();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Serve one request and return the request text the server received.
    fn capture_request(fetcher: UreqFetcher) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/x", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut request = vec![0u8; 4096];
            let read = socket.read(&mut request).unwrap();
            let _ = socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
            String::from_utf8_lossy(&request[..read]).into_owned()
        });
        assert_eq!(fetcher.get(&url).unwrap(), "ok");
        worker.join().unwrap()
    }

    fn user_agent_header(request: &str) -> Option<&str> {
        request.lines().find_map(|line| {
            line.strip_prefix("user-agent: ")
                .or(line.strip_prefix("User-Agent: "))
        })
    }

    #[test]
    fn requests_name_the_app_by_default() {
        let request = capture_request(UreqFetcher::new(false));
        assert_eq!(
            user_agent_header(&request),
            Some(skjalf_core::model::DEFAULT_USER_AGENT),
            "{request}"
        );
    }

    #[test]
    fn requests_send_the_configured_user_agent() {
        let request = capture_request(UreqFetcher::new(false).with_user_agent("MyPlayer/1.0"));
        assert_eq!(
            user_agent_header(&request),
            Some("MyPlayer/1.0"),
            "{request}"
        );
    }

    fn serve_once(
        headers: &str,
        body: Vec<u8>,
        header_delay: Duration,
        body_delay: Duration,
    ) -> (String, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://{}/list?password=SECRETWORD",
            listener.local_addr().unwrap()
        );
        let headers = headers.to_owned();
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let _ = socket.read(&mut [0u8; 2048]);
            std::thread::sleep(header_delay);
            let _ = socket.write_all(headers.as_bytes());
            std::thread::sleep(body_delay);
            let _ = socket.write_all(&body);
        });
        (url, worker)
    }

    #[test]
    fn response_larger_than_ureq_into_string_limit_succeeds() {
        let body = vec![b'a'; 11 * 1024 * 1024];
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let (url, worker) = serve_once(&headers, body, Duration::ZERO, Duration::ZERO);
        let result = UreqFetcher::new(false).get(&url).unwrap();
        assert_eq!(result.len(), 11 * 1024 * 1024);
        worker.join().unwrap();
    }

    #[test]
    fn chunked_response_without_content_length_is_read() {
        let body = b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n".to_vec();
        let (url, worker) = serve_once(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
            body,
            Duration::ZERO,
            Duration::ZERO,
        );
        assert_eq!(UreqFetcher::new(false).get(&url).unwrap(), "hello world");
        worker.join().unwrap();
    }

    #[test]
    fn invalid_utf8_keeps_lossy_text_behavior() {
        let (url, worker) = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\n",
            vec![b'a', 0xff, b'b'],
            Duration::ZERO,
            Duration::ZERO,
        );
        assert_eq!(UreqFetcher::new(false).get(&url).unwrap(), "a\u{fffd}b");
        worker.join().unwrap();
    }

    #[test]
    fn custom_limits_apply_to_get_with_and_leave_get_unchanged() {
        let body = vec![b'a'; 2048];
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let (url, worker) = serve_once(&headers, body.clone(), Duration::ZERO, Duration::ZERO);
        let limits = FetchLimits {
            max_bytes: 1024,
            timeout: Duration::from_secs(5),
        };
        let error = UreqFetcher::new(false).get_with(&url, limits).unwrap_err();
        assert!(error.0.contains("1024 bytes"), "{}", error.0);
        assert!(!error.0.contains("SECRETWORD"));
        worker.join().unwrap();
        let (url, worker) = serve_once(&headers, body, Duration::ZERO, Duration::ZERO);
        assert_eq!(UreqFetcher::new(false).get(&url).unwrap().len(), 2048);
        worker.join().unwrap();
    }

    #[test]
    fn decoded_body_over_cap_fails_without_url_or_body() {
        let body = format!("{:X}\r\n{}\r\n0\r\n\r\n", 512, "S".repeat(512)).into_bytes();
        let (url, worker) = serve_once(
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
            body,
            Duration::ZERO,
            Duration::ZERO,
        );
        let mut fetcher = UreqFetcher::new(false);
        fetcher.max_response_bytes = 128;
        let error = fetcher.get(&url).unwrap_err();
        assert!(error.0.contains("limit"), "{}", error.0);
        assert!(error.0.contains("128 bytes"), "{}", error.0);
        assert!(!error.0.contains("SECRETWORD"));
        assert!(!error.0.contains("SSSS"));
        worker.join().unwrap();
    }

    #[test]
    fn gzip_inflated_body_is_limited_after_decompression() {
        // gzip of 512 'S' bytes: 26 wire bytes, 512 decoded bytes.
        let body = vec![
            31, 139, 8, 0, 0, 0, 0, 0, 2, 3, 11, 14, 30, 5, 35, 25, 0, 0, 8, 76, 148, 166, 0, 2, 0,
            0,
        ];
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let (url, worker) = serve_once(&headers, body, Duration::ZERO, Duration::ZERO);
        let mut fetcher = UreqFetcher::new(false);
        fetcher.max_response_bytes = 128;
        let error = fetcher.get(&url).unwrap_err();
        assert!(error.0.contains("limit"), "{}", error.0);
        worker.join().unwrap();
    }

    #[test]
    fn header_timeout_has_distinct_safe_error() {
        let (url, worker) = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n",
            b"late".to_vec(),
            Duration::from_millis(250),
            Duration::ZERO,
        );
        let mut fetcher = UreqFetcher::new(false);
        fetcher.request_timeout = Duration::from_millis(80);
        let error = fetcher.get(&url).unwrap_err();
        assert!(error.0.contains("timed out"), "{}", error.0);
        assert!(!error.0.contains("SECRETWORD"));
        worker.join().unwrap();
    }

    #[test]
    fn response_read_timeout_has_distinct_safe_error() {
        let (url, worker) = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n",
            b"late".to_vec(),
            Duration::ZERO,
            Duration::from_millis(250),
        );
        let mut fetcher = UreqFetcher::new(false);
        fetcher.request_timeout = Duration::from_millis(80);
        let error = fetcher.get(&url).unwrap_err();
        assert!(error.0.contains("timed out"), "{}", error.0);
        assert!(!error.0.contains("SECRETWORD"));
        worker.join().unwrap();
    }

    #[test]
    fn truncated_response_has_safe_read_error() {
        let (url, worker) = serve_once(
            "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n",
            b"short".to_vec(),
            Duration::ZERO,
            Duration::ZERO,
        );
        let error = UreqFetcher::new(false).get(&url).unwrap_err();
        assert_eq!(error.0, "Could not read the server response.");
        worker.join().unwrap();
    }

    #[test]
    fn error_messages_redact_credentials() {
        // Port 9 (discard) on localhost: refused instantly, no network.
        let fetcher = UreqFetcher::new(false);
        let err = fetcher
            .get("http://127.0.0.1:9/player_api.php?username=alice&password=SECRETWORD")
            .unwrap_err();
        assert!(!err.0.contains("SECRETWORD"), "got: {}", err.0);
        assert!(!err.0.contains("alice"), "got: {}", err.0);
    }

    #[test]
    fn path_credentials_do_not_appear_in_network_error() {
        let error = UreqFetcher::new(false)
            .get("http://127.0.0.1:9/playlist/alice/REVIEW_SECRET/list.m3u")
            .unwrap_err();
        assert!(!error.0.contains("alice"));
        assert!(!error.0.contains("REVIEW_SECRET"));
        assert!(!error.0.contains("playlist"));
    }
}

/// Verifier that accepts any server certificate (only used when
/// `ignore_ssl` is on).
#[derive(Debug)]
struct AcceptAnyServerCert;

impl rustls::client::danger::ServerCertVerifier for AcceptAnyServerCert {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::ED25519,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
        ]
    }
}
