use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use kemuri_core::{ProbeKind, SampleOutcome};
use serde::{Deserialize, Serialize};

use crate::{Probe, ProbeExecutionError, ProbeRound, ResolvedCheck, RoundContext, SampleResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpConnectionMode {
    #[default]
    Pooled,
    PerRound,
    Fresh,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpProbeConfig {
    pub url: String,
    pub method: Option<String>,
    pub headers: HashMap<String, String>,
    pub expected_status: Option<u16>,
    pub expected_status_range: Option<(u16, u16)>,
    pub follow_redirects: bool,
    pub max_redirects: u32,
    pub tls_validate: bool,
    pub connection_mode: HttpConnectionMode,
    pub user_agent: Option<String>,
}

impl Default for HttpProbeConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            method: None,
            headers: HashMap::new(),
            expected_status: None,
            expected_status_range: None,
            follow_redirects: true,
            max_redirects: 10,
            tls_validate: true,
            connection_mode: HttpConnectionMode::default(),
            user_agent: None,
        }
    }
}

pub struct HttpProbe {
    client: Arc<reqwest::Client>,
    config: HttpProbeConfig,
}

impl HttpProbe {
    pub fn new(config: HttpProbeConfig) -> Result<Self, ProbeExecutionError> {
        let client = Self::build_client(&config)?;
        Ok(Self {
            client: Arc::new(client),
            config,
        })
    }

    fn build_client(config: &HttpProbeConfig) -> Result<reqwest::Client, ProbeExecutionError> {
        let mut builder = reqwest::Client::builder()
            .redirect(if config.follow_redirects {
                reqwest::redirect::Policy::limited(config.max_redirects as usize)
            } else {
                reqwest::redirect::Policy::none()
            })
            .danger_accept_invalid_certs(!config.tls_validate)
            .tcp_nodelay(true);

        if let Some(ref ua) = config.user_agent {
            builder = builder.user_agent(ua);
        }

        builder
            .build()
            .map_err(|e| ProbeExecutionError::Internal(e.to_string()))
    }

    fn make_client_for_round(&self) -> Result<reqwest::Client, ProbeExecutionError> {
        Self::build_client(&self.config)
    }

    fn is_expected_status(&self, status: u16) -> bool {
        if let Some((lo, hi)) = self.config.expected_status_range {
            return (lo..=hi).contains(&status);
        }
        if let Some(expected) = self.config.expected_status {
            return status == expected;
        }
        (200..400).contains(&status)
    }

    fn classify_request_error(err: &reqwest::Error) -> SampleOutcome {
        if err.is_timeout() {
            return SampleOutcome::Timeout;
        }
        if err.is_connect() {
            let msg = err.to_string().to_lowercase();
            if msg.contains("dns")
                || msg.contains("resolve")
                || msg.contains("name")
                || msg.contains("nodename")
            {
                return SampleOutcome::DnsError;
            }
            if msg.contains("refused") {
                return SampleOutcome::ConnectionRefused;
            }
            if msg.contains("reset") || msg.contains("broken pipe") {
                return SampleOutcome::ConnectionReset;
            }
            if msg.contains("network") || msg.contains("unreachable") {
                return SampleOutcome::NetworkUnreachable;
            }
            return SampleOutcome::ConnectionRefused;
        }
        if err.is_redirect() {
            return SampleOutcome::UnexpectedResponse;
        }
        let msg = err.to_string().to_lowercase();
        if msg.contains("tls")
            || msg.contains("certificate")
            || msg.contains("ssl")
            || msg.contains("handshake")
        {
            return SampleOutcome::TlsError;
        }
        if msg.contains("protocol") || msg.contains("parse") {
            return SampleOutcome::ProtocolError;
        }
        SampleOutcome::InternalError
    }

    async fn execute_single(&self, client: &reqwest::Client, timeout: Duration) -> SampleResult {
        let method = match self.config.method.as_deref() {
            Some("HEAD") => reqwest::Method::HEAD,
            Some("POST") => reqwest::Method::POST,
            Some("PUT") => reqwest::Method::PUT,
            Some("DELETE") => reqwest::Method::DELETE,
            _ => reqwest::Method::GET,
        };

        let mut request_builder = client.request(method, &self.config.url).timeout(timeout);

        for (key, value) in &self.config.headers {
            request_builder = request_builder.header(key.as_str(), value.as_str());
        }

        let start = std::time::Instant::now();
        match request_builder.send().await {
            Ok(response) => {
                let elapsed = start.elapsed();
                let status = response.status().as_u16();
                if self.is_expected_status(status) {
                    SampleResult {
                        outcome: SampleOutcome::Success,
                        latency: Some(elapsed),
                        detail: Some(format!("status={}", status)),
                        metadata: None,
                    }
                } else {
                    SampleResult {
                        outcome: SampleOutcome::UnexpectedResponse,
                        latency: Some(elapsed),
                        detail: Some(format!("status={}", status)),
                        metadata: None,
                    }
                }
            }
            Err(err) => {
                let elapsed = start.elapsed();
                let outcome = Self::classify_request_error(&err);
                SampleResult {
                    outcome,
                    latency: Some(elapsed),
                    detail: Some(err.to_string()),
                    metadata: None,
                }
            }
        }
    }
}

#[async_trait]
impl Probe for HttpProbe {
    fn kind(&self) -> ProbeKind {
        ProbeKind::Http
    }

    async fn execute_round(
        &self,
        _context: RoundContext,
        check: ResolvedCheck,
    ) -> Result<ProbeRound, ProbeExecutionError> {
        let client = match self.config.connection_mode {
            HttpConnectionMode::Pooled => self.client.clone(),
            HttpConnectionMode::PerRound => Arc::new(self.make_client_for_round()?),
            HttpConnectionMode::Fresh => Arc::new(self.make_client_for_round()?),
        };

        let result = self.execute_single(&client, check.timeout).await;

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
    fn default_config_connection_mode() {
        let config = HttpProbeConfig::default();
        assert_eq!(config.connection_mode, HttpConnectionMode::Pooled);
        assert!(config.follow_redirects);
        assert!(config.tls_validate);
    }

    #[test]
    fn expected_status_check_single() {
        let config = HttpProbeConfig {
            expected_status: Some(200),
            ..Default::default()
        };
        let probe = HttpProbe::new(config).unwrap();
        assert!(probe.is_expected_status(200));
        assert!(!probe.is_expected_status(404));
    }

    #[test]
    fn expected_status_check_range() {
        let config = HttpProbeConfig {
            expected_status_range: Some((200, 399)),
            ..Default::default()
        };
        let probe = HttpProbe::new(config).unwrap();
        assert!(probe.is_expected_status(200));
        assert!(probe.is_expected_status(301));
        assert!(probe.is_expected_status(399));
        assert!(!probe.is_expected_status(400));
        assert!(!probe.is_expected_status(500));
    }

    #[test]
    fn expected_status_default_2xx_3xx() {
        let config = HttpProbeConfig::default();
        let probe = HttpProbe::new(config).unwrap();
        assert!(probe.is_expected_status(200));
        assert!(probe.is_expected_status(204));
        assert!(probe.is_expected_status(301));
        assert!(!probe.is_expected_status(400));
        assert!(!probe.is_expected_status(500));
    }
}
