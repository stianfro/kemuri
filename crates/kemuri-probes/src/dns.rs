use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use async_trait::async_trait;
use hickory_resolver::TokioAsyncResolver;
use hickory_resolver::config::{NameServerConfig, Protocol, ResolverConfig, ResolverOpts};
use hickory_resolver::error::ResolveErrorKind;
use hickory_resolver::proto::rr::RecordType;
use kemuri_core::{ProbeKind, SampleOutcome};
use serde::{Deserialize, Serialize};

use crate::{Probe, ProbeExecutionError, ProbeRound, ResolvedCheck, RoundContext, SampleResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DnsProtocol {
    #[default]
    Udp,
    Tcp,
}

impl std::fmt::Display for DnsProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DnsProtocol::Udp => write!(f, "udp"),
            DnsProtocol::Tcp => write!(f, "tcp"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DnsResponseCode {
    #[default]
    NoError,
    FormErr,
    ServFail,
    NXDomain,
    NotImp,
    Refused,
}

impl DnsResponseCode {
    fn from_hickory(rcode: hickory_resolver::proto::op::ResponseCode) -> Self {
        use hickory_resolver::proto::op::ResponseCode as RC;
        match rcode {
            RC::NoError => DnsResponseCode::NoError,
            RC::FormErr => DnsResponseCode::FormErr,
            RC::ServFail => DnsResponseCode::ServFail,
            RC::NXDomain => DnsResponseCode::NXDomain,
            RC::NotImp => DnsResponseCode::NotImp,
            RC::Refused => DnsResponseCode::Refused,
            _ => DnsResponseCode::NoError,
        }
    }
}

impl std::fmt::Display for DnsResponseCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DnsResponseCode::NoError => write!(f, "noerror"),
            DnsResponseCode::FormErr => write!(f, "formerr"),
            DnsResponseCode::ServFail => write!(f, "servfail"),
            DnsResponseCode::NXDomain => write!(f, "nxdomain"),
            DnsResponseCode::NotImp => write!(f, "notimp"),
            DnsResponseCode::Refused => write!(f, "refused"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DnsProbeConfig {
    pub protocol: DnsProtocol,
    pub expected_rcode: DnsResponseCode,
    #[serde(default)]
    pub require_answer: bool,
}

pub struct DnsProbe {
    config: DnsProbeConfig,
}

impl DnsProbe {
    pub fn new(config: DnsProbeConfig) -> Self {
        Self { config }
    }

    fn parse_record_type(s: &str) -> RecordType {
        match s.to_uppercase().as_str() {
            "A" => RecordType::A,
            "AAAA" => RecordType::AAAA,
            "CNAME" => RecordType::CNAME,
            "MX" => RecordType::MX,
            "NS" => RecordType::NS,
            "SOA" => RecordType::SOA,
            "SRV" => RecordType::SRV,
            "TXT" => RecordType::TXT,
            "PTR" => RecordType::PTR,
            _ => RecordType::A,
        }
    }

    fn parse_server_addr(server: &str) -> Option<SocketAddr> {
        if let Ok(addr) = server.parse::<SocketAddr>() {
            return Some(addr);
        }
        if let Ok(ip) = server.parse::<std::net::IpAddr>() {
            return Some(SocketAddr::new(ip, 53));
        }
        None
    }

    async fn execute_single(
        &self,
        name: &str,
        record_type: RecordType,
        server: Option<&str>,
        timeout: Duration,
    ) -> SampleResult {
        let protocol = match self.config.protocol {
            DnsProtocol::Udp => Protocol::Udp,
            DnsProtocol::Tcp => Protocol::Tcp,
        };

        let resolver_config = if let Some(server_str) = server {
            if let Some(addr) = Self::parse_server_addr(server_str) {
                let mut config = ResolverConfig::new();
                config.add_name_server(NameServerConfig {
                    socket_addr: addr,
                    protocol,
                    tls_dns_name: None,
                    trust_negative_responses: false,
                    bind_addr: None,
                });
                config
            } else {
                return SampleResult {
                    outcome: SampleOutcome::DnsError,
                    latency: None,
                    detail: Some(format!("invalid dns server: {}", server_str)),
                    metadata: None,
                };
            }
        } else {
            ResolverConfig::default()
        };

        let mut opts = ResolverOpts::default();
        opts.timeout = timeout;
        opts.attempts = 1;
        opts.cache_size = 0;
        opts.rotate = false;

        let resolver = TokioAsyncResolver::tokio(resolver_config, opts);

        let name = match name.parse::<hickory_resolver::proto::rr::Name>() {
            Ok(n) => n,
            Err(e) => {
                return SampleResult {
                    outcome: SampleOutcome::DnsError,
                    latency: None,
                    detail: Some(format!("invalid query name: {}", e)),
                    metadata: None,
                };
            }
        };

        let start = std::time::Instant::now();
        let query_name_str = name.to_string();
        let lookup_result = resolver.lookup(name, record_type).await;
        let elapsed = start.elapsed();

        match lookup_result {
            Ok(lookup) => {
                let answer_count = lookup.records().len();
                let mut metadata = HashMap::new();
                metadata.insert("response_code".to_owned(), "noerror".to_owned());
                metadata.insert("answer_count".to_owned(), answer_count.to_string());
                metadata.insert("query_name".to_owned(), query_name_str.clone());
                metadata.insert("record_type".to_owned(), format!("{:?}", record_type));
                if let Some(srv) = server {
                    metadata.insert("server".to_owned(), srv.to_owned());
                }
                metadata.insert("protocol".to_owned(), self.config.protocol.to_string());

                let is_expected = self.config.expected_rcode == DnsResponseCode::NoError;
                let has_answer = answer_count > 0;

                if !is_expected {
                    SampleResult {
                        outcome: SampleOutcome::UnexpectedResponse,
                        latency: Some(elapsed),
                        detail: Some(format!(
                            "expected {:?}, got noerror",
                            self.config.expected_rcode
                        )),
                        metadata: Some(metadata),
                    }
                } else if self.config.require_answer && !has_answer {
                    SampleResult {
                        outcome: SampleOutcome::UnexpectedResponse,
                        latency: Some(elapsed),
                        detail: Some("expected answer but got none".to_owned()),
                        metadata: Some(metadata),
                    }
                } else {
                    SampleResult {
                        outcome: SampleOutcome::Success,
                        latency: Some(elapsed),
                        detail: Some(format!("answers={}", answer_count)),
                        metadata: Some(metadata),
                    }
                }
            }
            Err(e) => {
                let mut metadata = HashMap::new();
                metadata.insert("query_name".to_owned(), query_name_str.clone());
                metadata.insert("record_type".to_owned(), format!("{:?}", record_type));
                if let Some(srv) = server {
                    metadata.insert("server".to_owned(), srv.to_owned());
                }
                metadata.insert("protocol".to_owned(), self.config.protocol.to_string());

                match e.kind() {
                    ResolveErrorKind::NoRecordsFound { response_code, .. } => {
                        let rcode = DnsResponseCode::from_hickory(*response_code);
                        metadata.insert("response_code".to_owned(), rcode.to_string());
                        metadata.insert("answer_count".to_owned(), "0".to_owned());

                        if rcode == self.config.expected_rcode {
                            SampleResult {
                                outcome: SampleOutcome::Success,
                                latency: Some(elapsed),
                                detail: Some(format!("response_code={}", rcode)),
                                metadata: Some(metadata),
                            }
                        } else {
                            SampleResult {
                                outcome: SampleOutcome::UnexpectedResponse,
                                latency: Some(elapsed),
                                detail: Some(format!(
                                    "expected {:?}, got {}",
                                    self.config.expected_rcode, rcode
                                )),
                                metadata: Some(metadata),
                            }
                        }
                    }
                    ResolveErrorKind::Timeout => {
                        metadata.insert("response_code".to_owned(), "timeout".to_owned());
                        metadata.insert("answer_count".to_owned(), "0".to_owned());
                        SampleResult {
                            outcome: SampleOutcome::Timeout,
                            latency: Some(elapsed),
                            detail: Some("dns query timeout".to_owned()),
                            metadata: Some(metadata),
                        }
                    }
                    ResolveErrorKind::NoConnections => {
                        metadata.insert("response_code".to_owned(), "connection_error".to_owned());
                        metadata.insert("answer_count".to_owned(), "0".to_owned());
                        let msg = e.to_string().to_lowercase();
                        if msg.contains("refused") {
                            SampleResult {
                                outcome: SampleOutcome::ConnectionRefused,
                                latency: Some(elapsed),
                                detail: Some(e.to_string()),
                                metadata: Some(metadata),
                            }
                        } else {
                            SampleResult {
                                outcome: SampleOutcome::DnsError,
                                latency: Some(elapsed),
                                detail: Some(e.to_string()),
                                metadata: Some(metadata),
                            }
                        }
                    }
                    _ => {
                        metadata.insert("response_code".to_owned(), "error".to_owned());
                        metadata.insert("answer_count".to_owned(), "0".to_owned());
                        let msg = e.to_string().to_lowercase();
                        if msg.contains("refused") {
                            SampleResult {
                                outcome: SampleOutcome::ConnectionRefused,
                                latency: Some(elapsed),
                                detail: Some(e.to_string()),
                                metadata: Some(metadata),
                            }
                        } else if msg.contains("protocol") || msg.contains("parse") {
                            SampleResult {
                                outcome: SampleOutcome::ProtocolError,
                                latency: Some(elapsed),
                                detail: Some(e.to_string()),
                                metadata: Some(metadata),
                            }
                        } else {
                            SampleResult {
                                outcome: SampleOutcome::DnsError,
                                latency: Some(elapsed),
                                detail: Some(e.to_string()),
                                metadata: Some(metadata),
                            }
                        }
                    }
                }
            }
        }
    }
}

#[async_trait]
impl Probe for DnsProbe {
    fn kind(&self) -> ProbeKind {
        ProbeKind::Dns
    }

    async fn execute_round(
        &self,
        _context: RoundContext,
        check: ResolvedCheck,
    ) -> Result<ProbeRound, ProbeExecutionError> {
        let crate::ProbeSettings::Dns(settings) = &check.settings else {
            return Err(ProbeExecutionError::Internal(
                "DNS probe received settings for another probe type".to_owned(),
            ));
        };
        let name = &settings.domain;
        let record_type_str = settings.record_type.as_deref().unwrap_or("A");
        let record_type = Self::parse_record_type(record_type_str);
        let server = settings.resolver.as_deref();

        let effective = Self::new(DnsProbeConfig {
            protocol: match settings.protocol.as_str() {
                "tcp" => DnsProtocol::Tcp,
                _ => DnsProtocol::Udp,
            },
            expected_rcode: match settings.expected_rcode.as_str() {
                "formerr" => DnsResponseCode::FormErr,
                "servfail" => DnsResponseCode::ServFail,
                "nxdomain" => DnsResponseCode::NXDomain,
                "notimp" => DnsResponseCode::NotImp,
                "refused" => DnsResponseCode::Refused,
                _ => DnsResponseCode::NoError,
            },
            require_answer: settings.require_answer,
        });
        let result = effective
            .execute_single(name, record_type, server, check.timeout)
            .await;

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
        let config = DnsProbeConfig::default();
        assert_eq!(config.protocol, DnsProtocol::Udp);
        assert_eq!(config.expected_rcode, DnsResponseCode::NoError);
        assert!(!config.require_answer);
    }

    #[test]
    fn protocol_display() {
        assert_eq!(DnsProtocol::Udp.to_string(), "udp");
        assert_eq!(DnsProtocol::Tcp.to_string(), "tcp");
    }

    #[test]
    fn response_code_display() {
        assert_eq!(DnsResponseCode::NoError.to_string(), "noerror");
        assert_eq!(DnsResponseCode::NXDomain.to_string(), "nxdomain");
        assert_eq!(DnsResponseCode::ServFail.to_string(), "servfail");
    }

    #[test]
    fn parse_record_type_known() {
        assert_eq!(DnsProbe::parse_record_type("A"), RecordType::A);
    }

    #[test]
    fn parse_record_type_variants() {
        assert_eq!(DnsProbe::parse_record_type("A"), RecordType::A);
        assert_eq!(DnsProbe::parse_record_type("AAAA"), RecordType::AAAA);
        assert_eq!(DnsProbe::parse_record_type("MX"), RecordType::MX);
        assert_eq!(DnsProbe::parse_record_type("TXT"), RecordType::TXT);
        assert_eq!(DnsProbe::parse_record_type("CNAME"), RecordType::CNAME);
        assert_eq!(DnsProbe::parse_record_type("NS"), RecordType::NS);
        assert_eq!(DnsProbe::parse_record_type("SOA"), RecordType::SOA);
        assert_eq!(DnsProbe::parse_record_type("SRV"), RecordType::SRV);
        assert_eq!(DnsProbe::parse_record_type("PTR"), RecordType::PTR);
    }

    #[test]
    fn parse_record_type_unknown_defaults_to_a() {
        assert_eq!(DnsProbe::parse_record_type("UNKNOWN"), RecordType::A);
    }

    #[test]
    fn parse_server_addr_ip_only() {
        let addr = DnsProbe::parse_server_addr("1.1.1.1");
        assert!(addr.is_some());
        assert_eq!(addr.unwrap().port(), 53);
    }

    #[test]
    fn parse_server_addr_with_port() {
        let addr = DnsProbe::parse_server_addr("1.1.1.1:5353");
        assert!(addr.is_some());
        assert_eq!(addr.unwrap().port(), 5353);
    }

    #[test]
    fn parse_server_addr_invalid() {
        let addr = DnsProbe::parse_server_addr("not-an-address");
        assert!(addr.is_none());
    }
}
