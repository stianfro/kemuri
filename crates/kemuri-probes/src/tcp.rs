use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

use async_trait::async_trait;
use kemuri_core::{ProbeKind, SampleOutcome};
use rustls::pki_types::pem::PemObject;
use serde::{Deserialize, Serialize};

use crate::{
    AddressFamily, Probe, ProbeExecutionError, ProbeRound, ResolvedCheck, RoundContext,
    SampleResult,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TcpProbeConfig {
    pub address_family: AddressFamily,
    pub source_address: Option<String>,
    pub tls: Option<TcpTlsConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TcpTlsConfig {
    pub enabled: bool,
    pub server_name: Option<String>,
    pub tls_validate: Option<bool>,
    pub root_certificates: Option<Vec<String>>,
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

        let socket = if is_ipv6 {
            tokio::net::TcpSocket::new_v6()
        } else {
            tokio::net::TcpSocket::new_v4()
        };
        let connect_result = match socket {
            Ok(socket) => {
                if let Some(source) = &self.config.source_address {
                    let source_ip = source.parse::<IpAddr>().map_err(|error| {
                        ProbeExecutionError::Network(format!(
                            "invalid TCP source address {source}: {error}"
                        ))
                    });
                    match source_ip {
                        Ok(source_ip) if source_ip.is_ipv6() == is_ipv6 => {
                            if let Err(error) = socket.bind(std::net::SocketAddr::new(source_ip, 0))
                            {
                                return SampleResult {
                                    outcome: Self::classify_connect_error(&error),
                                    latency: None,
                                    detail: Some(error.to_string()),
                                    metadata: None,
                                };
                            }
                        }
                        Ok(_) => {
                            return SampleResult {
                                outcome: SampleOutcome::InternalError,
                                latency: None,
                                detail: Some(
                                    "TCP source address family does not match destination"
                                        .to_owned(),
                                ),
                                metadata: None,
                            };
                        }
                        Err(error) => {
                            return SampleResult {
                                outcome: SampleOutcome::InternalError,
                                latency: None,
                                detail: Some(error.to_string()),
                                metadata: None,
                            };
                        }
                    }
                }
                tokio::time::timeout(timeout, socket.connect(addr)).await
            }
            Err(error) => Ok(Err(error)),
        };

        let elapsed = start.elapsed();

        match connect_result {
            Ok(Ok(stream)) => {
                let mut metadata = HashMap::new();
                metadata.insert("resolved_ip".to_owned(), resolved_ip.to_string());
                metadata.insert(
                    "ip_family".to_owned(),
                    if is_ipv6 { "ipv6" } else { "ipv4" }.to_owned(),
                );
                metadata.insert("port".to_owned(), port.to_string());

                if self.config.tls.as_ref().is_some_and(|tls| tls.enabled) {
                    match self.tls_handshake(stream, host, timeout).await {
                        Ok(()) => {
                            metadata.insert("tls".to_owned(), "true".to_owned());
                            SampleResult {
                                outcome: SampleOutcome::Success,
                                latency: Some(start.elapsed()),
                                detail: Some(format!("TLS handshake completed with {}", addr)),
                                metadata: Some(metadata),
                            }
                        }
                        Err(error) => SampleResult {
                            outcome: SampleOutcome::TlsError,
                            latency: None,
                            detail: Some(error.to_string()),
                            metadata: Some(metadata),
                        },
                    }
                } else {
                    SampleResult {
                        outcome: SampleOutcome::Success,
                        latency: Some(elapsed),
                        detail: Some(format!("connected to {}", addr)),
                        metadata: Some(metadata),
                    }
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
                    latency: None,
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
                    latency: None,
                    detail: Some("tcp connect timeout".to_owned()),
                    metadata: Some(metadata),
                }
            }
        }
    }

    async fn tls_handshake(
        &self,
        stream: tokio::net::TcpStream,
        host: &str,
        timeout: Duration,
    ) -> Result<(), ProbeExecutionError> {
        let tls = self
            .config
            .tls
            .as_ref()
            .ok_or_else(|| ProbeExecutionError::Internal("missing TLS settings".to_owned()))?;
        if tls.tls_validate == Some(false) {
            return Err(ProbeExecutionError::Tls(
                "TCP TLS certificate validation cannot be disabled".to_owned(),
            ));
        }
        let mut roots = rustls::RootCertStore::empty();
        roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        for path in tls.root_certificates.as_deref().unwrap_or_default() {
            let certificates = rustls::pki_types::CertificateDer::pem_file_iter(path)
                .map_err(|error| ProbeExecutionError::Tls(format!("{path}: {error}")))?;
            for certificate in certificates {
                roots
                    .add(
                        certificate.map_err(|error| {
                            ProbeExecutionError::Tls(format!("{path}: {error}"))
                        })?,
                    )
                    .map_err(|error| ProbeExecutionError::Tls(error.to_string()))?;
            }
        }
        let config = rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let server_name = tls.server_name.as_deref().unwrap_or(host).to_owned();
        let server_name = rustls::pki_types::ServerName::try_from(server_name)
            .map_err(|error| ProbeExecutionError::Tls(error.to_string()))?;
        tokio::time::timeout(
            timeout,
            tokio_rustls::TlsConnector::from(std::sync::Arc::new(config))
                .connect(server_name, stream),
        )
        .await
        .map_err(|_| ProbeExecutionError::Timeout(format!("{timeout:?}")))?
        .map_err(|error| ProbeExecutionError::Tls(error.to_string()))?;
        Ok(())
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
        let crate::ProbeSettings::Tcp(settings) = &check.settings else {
            return Err(ProbeExecutionError::Internal(
                "TCP probe received settings for another probe type".to_owned(),
            ));
        };
        let host = &settings.host;
        let port = settings.port;

        let effective = Self::new(TcpProbeConfig {
            address_family: match settings.address_family.as_str() {
                "ipv4" => AddressFamily::Ipv4,
                "ipv6" => AddressFamily::Ipv6,
                _ => AddressFamily::Auto,
            },
            source_address: settings.source_address.clone(),
            tls: settings.tls.as_ref().map(|tls| TcpTlsConfig {
                enabled: tls.enabled,
                server_name: tls.server_name.clone(),
                tls_validate: tls.tls_validate,
                root_certificates: tls.root_certificates.clone(),
            }),
        });
        let result = effective.execute_single(&host, port, check.timeout).await;

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
