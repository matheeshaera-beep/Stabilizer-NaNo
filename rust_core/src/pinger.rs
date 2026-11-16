use std::time::Duration;

use reqwest::Client;
use tokio::time::Instant;

/// Result of a single lightweight HTTP `HEAD` keepalive probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PingOutcome {
    pub status_code: u16,
    pub latency_ms: u64,
}

/// Async HTTP `HEAD` client backed by `reqwest` and `rustls-tls`.
#[derive(Clone, Debug)]
pub struct HttpPinger {
    client: Client,
}

impl HttpPinger {
    /// Constructs an `HttpPinger` with conservative timeouts and no redirect following.
    pub fn new() -> Result<Self, String> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(4))
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .pool_idle_timeout(Duration::from_secs(30))
            .user_agent("Stabilizer-NaNo/0.1")
            .build()
            .map_err(|err| format!("failed to initialize HTTP client: {err}"))?;

        Ok(Self { client })
    }

    /// Sends one lightweight HTTP `HEAD` request to `url` and measures round-trip latency.
    pub async fn send_head_ping(&self, url: &str) -> Result<PingOutcome, String> {
        let started = Instant::now();
        let response = self
            .client
            .head(url)
            .send()
            .await
            .map_err(|err| format!("HEAD ping failed: {err}"))?;

        let latency_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let status_code = response.status().as_u16();
        Ok(PingOutcome {
            status_code,
            latency_ms,
        })
    }
}
