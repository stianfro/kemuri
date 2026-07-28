use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

use async_trait::async_trait;
use kemuri_core::{ProbeKind, SampleOutcome};
use serde::{Deserialize, Serialize};

use crate::{
    AddressFamily, Probe, ProbeExecutionError, ProbeRound, ResolvedCheck, RoundContext,
    SampleResult,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TcpProbeConfig {
    pub address_family: AddressFamily,
    pub source_address: Option<String>,
}

pub struct TcpProbe {
    config: TcpProbeConfig,
}

impl TcpProbe {
    pub fn new(config: TcpProbeConfig) -> Self {
        Self { config }
    }

    async fn resolve_host(&self, host: &str) -> Result<(IpAddr, bool), ProbeExecutionError> {
        let lookup_addr = format!("{}:0", host);
        let addrs = tokio::net::lookup_host(&lookup_addr)
            .await
            .map_err(|e| ProbeExecutionError::Dns(e.to_string()))?;

        for socket_addr in addrs {
            let ip = socket_addr.ip();
            match self.config.address_family {
                AddressFamily::Ipv4 if !ip.is_ipv4() => continue,
                AddressFamily::Ipv6 if !ip.is_ipv6() => continue,
                _ => return Ok((ip, ip.is_ipv6())),
            }
        }

        Err(ProbeExecutionError::Dns(format!(
            "no matching address for {}",
            host
        )))
    }

    fn classify_connect_error(e: &std::io::Error) -> SampleOutcome {
        match e.kind() {
            std::io::ErrorKind::ConnectionRefused => SampleOutcome::ConnectionRefused,
            std::io::ErrorKind::ConnectionReset => SampleOutcome::ConnectionReset,
            std::io::ErrorKind::NetworkUnreachable => SampleOutcome::NetworkUnreachable,
            std::io::ErrorKind::PermissionDenied => SampleOutcome::PermissionDenied,
            std::io::ErrorKind::TimedOut => SampleOutcome::Timeout,
            _ => {
                let msg = e.to_string().to_lowercase();
                if msg.contains("refused") {
                    SampleOutcome::ConnectionRefused
                } else if msg.contains("reset") {
                    SampleOutcome::ConnectionReset
                } else if msg.contains("unreachable") {
                    SampleOutcome::NetworkUnreachable
                } else if msg.contains("permission") {
                    SampleOutcome::PermissionDenied
                } else if msg.contains("timed out") {
                    SampleOutcome::Timeout
                } else {
                    SampleOutcome::InternalError
                }
            }
        }
    }

    async fn execute_single(&self, host: &str, port: u16, timeout: Duration) -> SampleResult {
        let resolve_result = self.resolve_host(host).await;
        let (resolved_ip, is_ipv6) = match resolve_result {
            Ok(addr) => addr,
            Err(e) => {
                return SampleResult {
                    outcome: SampleOutcome::DnsError,
                    latency: None,
                    detail: Some(e.to_string()),
                    metadata: None,
                };
            }
        };

        let addr = std::net::SocketAddr::new(resolved_ip, port);
        let start = std::time::Instant::now();

        let connect_result =
            tokio::time::timeout(timeout, tokio::net::TcpStream::connect(addr)).await;

        let elapsed = start.elapsed();

        match connect_result {
            Ok(Ok(_stream)) => {
                let mut metadata = HashMap::new();
                metadata.insert("resolved_ip".to_owned(), resolved_ip.to_string());
                metadata.insert(
                    "ip_family".to_owned(),
                    if is_ipv6 { "ipv6" } else { "ipv4" }.to_owned(),
                );
                metadata.insert("port".to_owned(), port.to_string());

                SampleResult {
                    outcome: SampleOutcome::Success,
                    latency: Some(elapsed),
                    detail: Some(format!("connected to {}", addr)),
                    metadata: Some(metadata),
                }
            }
            Ok(Err(e)) => {
                let outcome = Self::classify_connect_error(&e);
                let mut metadata = HashMap::new();
                metadata.insert("resolved_ip".to_owned(), resolved_ip.to_string());
                metadata.insert(
                    "ip_family".to_owned(),
                    if is_ipv6 { "ipv6" } else { "ipv4" }.to_owned(),
                );
                metadata.insert("port".to_owned(), port.to_string());

                SampleResult {
                    outcome,
                    latency: Some(elapsed),
                    detail: Some(e.to_string()),
                    metadata: Some(metadata),
                }
            }
            Err(_) => {
                let mut metadata = HashMap::new();
                metadata.insert("resolved_ip".to_owned(), resolved_ip.to_string());
                metadata.insert(
                    "ip_family".to_owned(),
                    if is_ipv6 { "ipv6" } else { "ipv4" }.to_owned(),
                );
                metadata.insert("port".to_owned(), port.to_string());

                SampleResult {
                    outcome: SampleOutcome::Timeout,
                    latency: Some(elapsed),
                    detail: Some("tcp connect timeout".to_owned()),
                    metadata: Some(metadata),
                }
            }
        }
    }
}

#[async_trait]
impl Probe for TcpProbe {
    fn kind(&self) -> ProbeKind {
        ProbeKind::Tcp
    }

    async fn execute_round(
        &self,
        _context: RoundContext,
        check: ResolvedCheck,
    ) -> Result<ProbeRound, ProbeExecutionError> {
        let host = check
            .params
            .get("host")
            .cloned()
            .unwrap_or_else(|| check.address.clone());
        let port: u16 = check
            .params
            .get("port")
            .and_then(|p| p.parse().ok())
            .unwrap_or(80);

        let result = self.execute_single(&host, port, check.timeout).await;

        Ok(ProbeRound {
            check_id: check.check_id,
            results: vec![result],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let config = TcpProbeConfig::default();
        assert_eq!(config.address_family, AddressFamily::Auto);
        assert!(config.source_address.is_none());
    }

    #[test]
    fn classify_connection_refused() {
        let e = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused");
        assert_eq!(
            TcpProbe::classify_connect_error(&e),
            SampleOutcome::ConnectionRefused
        );
    }

    #[test]
    fn classify_connection_reset() {
        let e = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset");
        assert_eq!(
            TcpProbe::classify_connect_error(&e),
            SampleOutcome::ConnectionReset
        );
    }

    #[test]
    fn classify_network_unreachable() {
        let e = std::io::Error::new(std::io::ErrorKind::NetworkUnreachable, "unreachable");
        assert_eq!(
            TcpProbe::classify_connect_error(&e),
            SampleOutcome::NetworkUnreachable
        );
    }

    #[test]
    fn classify_permission_denied() {
        let e = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
        assert_eq!(
            TcpProbe::classify_connect_error(&e),
            SampleOutcome::PermissionDenied
        );
    }

    #[test]
    fn classify_timed_out() {
        let e = std::io::Error::new(std::io::ErrorKind::TimedOut, "timed out");
        assert_eq!(TcpProbe::classify_connect_error(&e), SampleOutcome::Timeout);
    }
}
