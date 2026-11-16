use std::net::{IpAddr, Ipv4Addr, SocketAddr};
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicU16, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls::ClientConfig;
use rustls_pki_types::ServerName;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
#[cfg(test)]
use tokio::net::TcpListener;
use tokio::net::{TcpSocket, TcpStream, UdpSocket};
use tokio::time::{timeout, Instant};
use tokio_rustls::TlsConnector;

const L1_TIMEOUT: Duration = Duration::from_secs(3);
const L2_TIMEOUT: Duration = Duration::from_secs(6);
const DEFAULT_DNS_TARGETS: [SocketAddr; 2] = [
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), 53),
    SocketAddr::new(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)), 53),
];

/// Probe tier used to verify cellular reachability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum ProbeTier {
    /// Ultra-lightweight cellular-bound UDP DNS query (~90 B) with TCP:443 fallback.
    L1Dns,
    /// Full cellular-bound HTTP/HTTPS `HEAD` validation (~1.5–3 KB) for ground truth.
    L2HttpHead,
}

/// Result of a single cellular probe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProbeOutcome {
    pub tier: ProbeTier,
    pub status_code: u16,
    pub latency_ms: u64,
    pub bytes_estimated: u64,
    pub target: String,
}

/// Hook allowing Android to bind raw sockets onto the cellular `Network` (`Network.bindSocket`).
pub trait SocketBinder: Send + Sync {
    fn bind_cellular_fd(&self, fd: i32) -> bool;
}

/// Async cellular-bound L1/L2 probe engine powered by `tokio` and `tokio-rustls`.
#[derive(Clone)]
pub struct CellularPinger {
    tls_connector: TlsConnector,
    binder: Arc<dyn SocketBinder>,
    next_tx_id: Arc<AtomicU16>,
    rr_counter: Arc<AtomicUsize>,
    rtt_ewma_ms: Arc<Mutex<[Option<u64>; 2]>>,
}

impl std::fmt::Debug for CellularPinger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CellularPinger").finish_non_exhaustive()
    }
}

impl CellularPinger {
    /// Builds a `CellularPinger` using `rustls` + `webpki-roots` and a socket binder hook.
    pub fn new(binder: Arc<dyn SocketBinder>) -> Result<Self, String> {
        let mut root_store = rustls::RootCertStore::empty();
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let tls_config = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|err| format!("failed to configure TLS versions: {err}"))?
            .with_root_certificates(root_store)
            .with_no_client_auth();

        Ok(Self {
            tls_connector: TlsConnector::from(Arc::new(tls_config)),
            binder,
            next_tx_id: Arc::new(AtomicU16::new(0x1337)),
            rr_counter: Arc::new(AtomicUsize::new(0)),
            rtt_ewma_ms: Arc::new(Mutex::new([None, None])),
        })
    }

    /// Executes an L1 fast probe (`L1Dns` with cellular TCP:443 fallback) or an L2 `HEAD` probe.
    pub async fn probe(&self, tier: ProbeTier, url: &str) -> Result<ProbeOutcome, String> {
        match tier {
            ProbeTier::L1Dns => self.probe_l1().await,
            ProbeTier::L2HttpHead => self.probe_l2_head(url).await,
        }
    }

    /// Sends an ultra-cheap L1 UDP DNS query to the fastest healthy target (`1.1.1.1` / `8.8.8.8`),
    /// falling back to a cellular-bound TCP:443 handshake if UDP/53 is filtered.
    pub async fn probe_l1(&self) -> Result<ProbeOutcome, String> {
        let idx = self.pick_target_index();
        let target = DEFAULT_DNS_TARGETS[idx];

        match self
            .udp_dns_ping(target, "connectivitycheck.gstatic.com")
            .await
        {
            Ok((rtt_ms, bytes)) => {
                self.record_target_rtt(idx, rtt_ms);
                Ok(ProbeOutcome {
                    tier: ProbeTier::L1Dns,
                    status_code: 0,
                    latency_ms: rtt_ms,
                    bytes_estimated: bytes,
                    target: format!("dns://{target}"),
                })
            }
            Err(dns_err) => {
                // Rotate to second DNS server before falling back to TCP connect.
                let alt_idx = (idx + 1) % DEFAULT_DNS_TARGETS.len();
                let alt_target = DEFAULT_DNS_TARGETS[alt_idx];
                if let Ok((rtt_ms, bytes)) = self
                    .udp_dns_ping(alt_target, "connectivitycheck.gstatic.com")
                    .await
                {
                    self.record_target_rtt(alt_idx, rtt_ms);
                    return Ok(ProbeOutcome {
                        tier: ProbeTier::L1Dns,
                        status_code: 0,
                        latency_ms: rtt_ms,
                        bytes_estimated: bytes,
                        target: format!("dns://{alt_target}"),
                    });
                }

                // If carrier APN blocks external port 53 UDP, test raw TCP:443 SYN-ACK.
                let tcp_target = SocketAddr::new(target.ip(), 443);
                let started = Instant::now();
                let _stream = timeout(L1_TIMEOUT, self.connect_cellular_tcp(tcp_target))
                    .await
                    .map_err(|_| format!("L1 DNS ({dns_err}) and TCP:443 timed out after 3s"))?
                    .map_err(|tcp_err| format!("L1 failed (DNS: {dns_err}; TCP: {tcp_err})"))?;

                let rtt_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
                self.record_target_rtt(idx, rtt_ms);
                Ok(ProbeOutcome {
                    tier: ProbeTier::L1Dns,
                    status_code: 0,
                    latency_ms: rtt_ms,
                    bytes_estimated: 300,
                    target: format!("tcp://{tcp_target}"),
                })
            }
        }
    }

    /// Sends one fresh cellular-bound HTTP/HTTPS `HEAD` probe to `url` with a 6s timeout.
    pub async fn probe_l2_head(&self, url: &str) -> Result<ProbeOutcome, String> {
        let parsed = ParsedHttpUrl::parse(url)?;
        let started = Instant::now();

        let outcome = timeout(L2_TIMEOUT, self.execute_l2_head(&parsed, started))
            .await
            .map_err(|_| {
                format!(
                    "HEAD probe timed out after {}s ({})",
                    L2_TIMEOUT.as_secs(),
                    parsed.host
                )
            })??;

        Ok(outcome)
    }

    async fn execute_l2_head(
        &self,
        parsed: &ParsedHttpUrl,
        started: Instant,
    ) -> Result<ProbeOutcome, String> {
        let addr = self.resolve_host(&parsed.host, parsed.port).await?;
        let tcp = self.connect_cellular_tcp(addr).await?;

        let req = format!(
            "HEAD {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: Stabilizer-NaNo/0.2\r\nAccept: */*\r\nConnection: close\r\n\r\n",
            parsed.path, parsed.host_header
        );

        let (status_code, resp_bytes) = if parsed.is_tls {
            let server_name = ServerName::try_from(parsed.host.clone())
                .map_err(|_| format!("invalid TLS server name: {}", parsed.host))?;
            let mut tls_stream = self
                .tls_connector
                .connect(server_name, tcp)
                .await
                .map_err(|err| format!("TLS handshake failed: {err}"))?;
            tls_stream
                .write_all(req.as_bytes())
                .await
                .map_err(|err| format!("TLS write failed: {err}"))?;
            tls_stream
                .flush()
                .await
                .map_err(|err| format!("TLS flush failed: {err}"))?;
            read_http_status_line(tls_stream).await?
        } else {
            let mut plain = tcp;
            plain
                .write_all(req.as_bytes())
                .await
                .map_err(|err| format!("HTTP write failed: {err}"))?;
            plain
                .flush()
                .await
                .map_err(|err| format!("HTTP flush failed: {err}"))?;
            read_http_status_line(plain).await?
        };

        let latency_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let tls_overhead = if parsed.is_tls { 1_800 } else { 120 };
        let bytes_estimated = req.len() as u64 + resp_bytes + tls_overhead;

        Ok(ProbeOutcome {
            tier: ProbeTier::L2HttpHead,
            status_code,
            latency_ms,
            bytes_estimated,
            target: parsed.host.clone(),
        })
    }

    async fn resolve_host(&self, host: &str, port: u16) -> Result<SocketAddr, String> {
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(SocketAddr::new(ip, port));
        }

        // Resolve via cellular-bound UDP DNS so host lookups never leak onto Wi-Fi
        // and our own DNS traffic stays inside our app UID for TrafficStats subtraction.
        let dns_idx = self.pick_target_index();
        let dns_server = DEFAULT_DNS_TARGETS[dns_idx];
        match self.udp_dns_resolve_a(dns_server, host).await {
            Ok(ip) => return Ok(SocketAddr::new(IpAddr::V4(ip), port)),
            Err(err) if err.contains("cellular network unavailable") => return Err(err),
            Err(_) => {}
        }
        let alt_server = DEFAULT_DNS_TARGETS[(dns_idx + 1) % DEFAULT_DNS_TARGETS.len()];
        match self.udp_dns_resolve_a(alt_server, host).await {
            Ok(ip) => return Ok(SocketAddr::new(IpAddr::V4(ip), port)),
            Err(err) if err.contains("cellular network unavailable") => return Err(err),
            Err(_) => {}
        }

        // Fallback to OS resolver (for localhost / IPv6-only / local test hosts).
        let mut addrs = tokio::net::lookup_host((host, port))
            .await
            .map_err(|err| format!("DNS resolution failed for {host}: {err}"))?;
        addrs
            .next()
            .ok_or_else(|| format!("DNS returned no addresses for {host}"))
    }

    async fn connect_cellular_tcp(&self, addr: SocketAddr) -> Result<TcpStream, String> {
        let socket = match addr {
            SocketAddr::V4(_) => TcpSocket::new_v4(),
            SocketAddr::V6(_) => TcpSocket::new_v6(),
        }
        .map_err(|err| format!("failed to allocate TCP socket: {err}"))?;

        #[cfg(unix)]
        {
            if !self.binder.bind_cellular_fd(socket.as_raw_fd()) {
                return Err("cellular network unavailable for socket bind".to_owned());
            }
        }

        socket
            .connect(addr)
            .await
            .map_err(|err| format!("TCP connect to {addr} failed: {err}"))
    }

    fn bind_cellular_udp(&self, target: SocketAddr) -> Result<UdpSocket, String> {
        let bind_addr: SocketAddr = match target {
            SocketAddr::V4(_) => SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
            SocketAddr::V6(_) => "[::]:0".parse().map_err(|e| format!("{e}"))?,
        };
        let std_sock = std::net::UdpSocket::bind(bind_addr)
            .map_err(|err| format!("failed to bind UDP socket: {err}"))?;
        std_sock
            .set_nonblocking(true)
            .map_err(|err| format!("failed to set UDP nonblocking: {err}"))?;

        #[cfg(unix)]
        {
            if !self.binder.bind_cellular_fd(std_sock.as_raw_fd()) {
                return Err("cellular network unavailable for socket bind".to_owned());
            }
        }

        UdpSocket::from_std(std_sock).map_err(|err| format!("failed to wrap UDP socket: {err}"))
    }

    async fn udp_dns_ping(&self, server: SocketAddr, qname: &str) -> Result<(u64, u64), String> {
        let tx_id = self.next_tx_id.fetch_add(1, Ordering::Relaxed);
        let packet = build_dns_a_query(tx_id, qname)?;
        let sock = self.bind_cellular_udp(server)?;
        let started = Instant::now();

        sock.connect(server)
            .await
            .map_err(|err| format!("UDP connect to {server} failed: {err}"))?;
        sock.send(&packet)
            .await
            .map_err(|err| format!("UDP DNS send failed: {err}"))?;

        let mut buf = [0u8; 512];
        let n = timeout(L1_TIMEOUT, sock.recv(&mut buf))
            .await
            .map_err(|_| format!("DNS probe timeout ({server})"))?
            .map_err(|err| format!("UDP DNS recv failed: {err}"))?;

        if n < 12 {
            return Err("truncated DNS response".to_owned());
        }
        let resp_id = u16::from_be_bytes([buf[0], buf[1]]);
        if resp_id != tx_id {
            return Err("mismatched DNS transaction ID".to_owned());
        }
        let is_response = (buf[2] & 0x80) != 0;
        if !is_response {
            return Err("invalid DNS response flags".to_owned());
        }

        let rtt_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        Ok((rtt_ms, (packet.len() + n) as u64))
    }

    async fn udp_dns_resolve_a(&self, server: SocketAddr, host: &str) -> Result<Ipv4Addr, String> {
        let tx_id = self.next_tx_id.fetch_add(1, Ordering::Relaxed);
        let packet = build_dns_a_query(tx_id, host)?;
        let sock = self.bind_cellular_udp(server)?;
        sock.connect(server)
            .await
            .map_err(|err| format!("UDP connect failed: {err}"))?;
        sock.send(&packet)
            .await
            .map_err(|err| format!("UDP send failed: {err}"))?;

        let mut buf = [0u8; 512];
        let n = timeout(L1_TIMEOUT, sock.recv(&mut buf))
            .await
            .map_err(|_| "DNS resolve timeout".to_owned())?
            .map_err(|err| format!("UDP recv failed: {err}"))?;

        parse_dns_first_a_record(&buf[..n], tx_id)
    }

    fn pick_target_index(&self) -> usize {
        let count = self.rr_counter.fetch_add(1, Ordering::Relaxed);
        // Periodically probe the secondary target (1 in 5) so RTT stays fresh.
        if count % 5 == 4 {
            return 1;
        }
        let guard = match self.rtt_ewma_ms.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        match (guard[0], guard[1]) {
            (Some(a), Some(b)) if b + 15 < a => 1,
            _ => 0,
        }
    }

    fn record_target_rtt(&self, idx: usize, rtt_ms: u64) {
        let mut guard = match self.rtt_ewma_ms.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        guard[idx] = Some(match guard[idx] {
            Some(prev) => (prev * 3 + rtt_ms) / 4,
            None => rtt_ms,
        });
    }
}

/// No-op socket binder for unit tests and non-Android hosts.
pub struct NoopSocketBinder;

impl SocketBinder for NoopSocketBinder {
    fn bind_cellular_fd(&self, _fd: i32) -> bool {
        true
    }
}

/// Starts a one-shot local HTTP test responder only in tests.
#[cfg(test)]
async fn spawn_test_http_server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind local test port");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        if let Ok((mut sock, _)) = listener.accept().await {
            let _ = sock
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
        }
    });
    addr
}

#[derive(Debug, PartialEq, Eq)]
struct ParsedHttpUrl {
    is_tls: bool,
    host: String,
    port: u16,
    host_header: String,
    path: String,
}

impl ParsedHttpUrl {
    fn parse(raw: &str) -> Result<Self, String> {
        let trimmed = raw.trim();
        let (is_tls, rest) = if let Some(r) = trimmed.strip_prefix("https://") {
            (true, r)
        } else if let Some(r) = trimmed.strip_prefix("http://") {
            (false, r)
        } else {
            return Err("URL must begin with http:// or https://".to_owned());
        };

        let (authority, path) = match rest.find('/') {
            Some(pos) => (&rest[..pos], rest[pos..].to_owned()),
            None => (rest, "/".to_owned()),
        };

        if authority.is_empty() {
            return Err("URL host is empty".to_owned());
        }

        let default_port = if is_tls { 443 } else { 80 };
        let (host, port) = if let Some(colon) = authority.rfind(':') {
            if let Ok(parsed_port) = authority[colon + 1..].parse::<u16>() {
                (authority[..colon].to_owned(), parsed_port)
            } else {
                (authority.to_owned(), default_port)
            }
        } else {
            (authority.to_owned(), default_port)
        };

        Ok(Self {
            is_tls,
            host,
            port,
            host_header: authority.to_owned(),
            path,
        })
    }
}

async fn read_http_status_line<S>(stream: S) -> Result<(u16, u64), String>
where
    S: tokio::io::AsyncRead + Unpin,
{
    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    let n = reader
        .read_line(&mut status_line)
        .await
        .map_err(|err| format!("failed reading HTTP status line: {err}"))?;
    if n == 0 {
        return Err("peer closed connection before HTTP status line".to_owned());
    }
    let code = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| format!("malformed HTTP status line: {}", status_line.trim()))?;
    Ok((code, n as u64))
}

fn build_dns_a_query(tx_id: u16, qname: &str) -> Result<Vec<u8>, String> {
    let mut pkt = Vec::with_capacity(64);
    pkt.extend_from_slice(&tx_id.to_be_bytes());
    pkt.extend_from_slice(&[0x01, 0x00]); // Standard query, Recursion Desired
    pkt.extend_from_slice(&[0x00, 0x01]); // QDCOUNT = 1
    pkt.extend_from_slice(&[0x00, 0x00]); // ANCOUNT = 0
    pkt.extend_from_slice(&[0x00, 0x00]); // NSCOUNT = 0
    pkt.extend_from_slice(&[0x00, 0x00]); // ARCOUNT = 0

    for label in qname.trim_end_matches('.').split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err(format!("invalid DNS label in {qname}"));
        }
        pkt.push(label.len() as u8);
        pkt.extend_from_slice(label.as_bytes());
    }
    pkt.push(0x00); // Root label
    pkt.extend_from_slice(&[0x00, 0x01]); // QTYPE = A
    pkt.extend_from_slice(&[0x00, 0x01]); // QCLASS = IN
    Ok(pkt)
}

fn parse_dns_first_a_record(buf: &[u8], expected_tx_id: u16) -> Result<Ipv4Addr, String> {
    if buf.len() < 12 {
        return Err("short DNS response".to_owned());
    }
    if u16::from_be_bytes([buf[0], buf[1]]) != expected_tx_id {
        return Err("DNS transaction ID mismatch".to_owned());
    }
    let rcode = buf[3] & 0x0F;
    if rcode != 0 {
        return Err(format!("DNS RCODE {rcode}"));
    }
    let qdcount = u16::from_be_bytes([buf[4], buf[5]]);
    let ancount = u16::from_be_bytes([buf[6], buf[7]]);
    if ancount == 0 {
        return Err("DNS response had 0 answers".to_owned());
    }

    let mut pos = 12;
    for _ in 0..qdcount {
        pos = skip_dns_name(buf, pos)?;
        if pos + 4 > buf.len() {
            return Err("truncated DNS question".to_owned());
        }
        pos += 4;
    }

    for _ in 0..ancount {
        pos = skip_dns_name(buf, pos)?;
        if pos + 10 > buf.len() {
            return Err("truncated DNS answer header".to_owned());
        }
        let rtype = u16::from_be_bytes([buf[pos], buf[pos + 1]]);
        let rclass = u16::from_be_bytes([buf[pos + 2], buf[pos + 3]]);
        let rdlength = u16::from_be_bytes([buf[pos + 8], buf[pos + 9]]) as usize;
        pos += 10;
        if pos + rdlength > buf.len() {
            return Err("truncated DNS RDATA".to_owned());
        }
        if rtype == 1 && rclass == 1 && rdlength == 4 {
            return Ok(Ipv4Addr::new(
                buf[pos],
                buf[pos + 1],
                buf[pos + 2],
                buf[pos + 3],
            ));
        }
        pos += rdlength;
    }

    Err("no IPv4 A record in DNS response".to_owned())
}

fn skip_dns_name(buf: &[u8], mut pos: usize) -> Result<usize, String> {
    loop {
        if pos >= buf.len() {
            return Err("truncated DNS name".to_owned());
        }
        let len = buf[pos];
        if (len & 0xC0) == 0xC0 {
            if pos + 1 >= buf.len() {
                return Err("truncated DNS pointer".to_owned());
            }
            return Ok(pos + 2);
        }
        if len == 0 {
            return Ok(pos + 1);
        }
        pos += 1 + (len as usize);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_http_and_https_urls() {
        let https = ParsedHttpUrl::parse("https://connectivitycheck.gstatic.com/generate_204")
            .expect("valid https url");
        assert!(https.is_tls);
        assert_eq!(https.host, "connectivitycheck.gstatic.com");
        assert_eq!(https.port, 443);
        assert_eq!(https.path, "/generate_204");

        let custom = ParsedHttpUrl::parse("http://127.0.0.1:8080/ping").expect("valid http url");
        assert!(!custom.is_tls);
        assert_eq!(custom.host, "127.0.0.1");
        assert_eq!(custom.port, 8080);
        assert_eq!(custom.path, "/ping");
    }

    #[test]
    fn builds_and_parses_dns_a_packet() {
        let q = build_dns_a_query(0x4242, "example.com").expect("valid query");
        assert_eq!(&q[0..2], &[0x42, 0x42]);
        assert_eq!(q[12], 7); // "example"

        let mut resp = q.clone();
        resp[2] = 0x81; // QR=1
        resp[3] = 0x80; // NOERROR
        resp[7] = 1; // ANCOUNT=1
        resp.extend_from_slice(&[
            0xC0, 0x0C, // PTR to offset 12
            0x00, 0x01, // TYPE A
            0x00, 0x01, // CLASS IN
            0x00, 0x00, 0x00, 0x3C, // TTL 60
            0x00, 0x04, // RDLENGTH 4
            93, 184, 216, 34, // 93.184.216.34
        ]);
        let ip = parse_dns_first_a_record(&resp, 0x4242).expect("valid answer");
        assert_eq!(ip, Ipv4Addr::new(93, 184, 216, 34));
    }

    #[tokio::test]
    async fn l2_head_probe_works_against_local_http_server() {
        let addr = spawn_test_http_server().await;
        let pinger = CellularPinger::new(Arc::new(NoopSocketBinder)).expect("pinger init");
        let outcome = pinger
            .probe_l2_head(&format!("http://{addr}/generate_204"))
            .await
            .expect("local HEAD succeeds");
        assert_eq!(outcome.tier, ProbeTier::L2HttpHead);
        assert_eq!(outcome.status_code, 204);
    }
}
