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
    pub body: Option<String>,
    pub measure_until: String,
    pub root_certificates: Vec<String>,
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
            body: None,
            measure_until: "headers".to_owned(),
            root_certificates: Vec::new(),
        }
    }
}

pub struct HttpProbe {
    config: HttpProbeConfig,
}

impl HttpProbe {
    pub fn new(config: HttpProbeConfig) -> Result<Self, ProbeExecutionError> {
        Self::build_client(&config)?;
        Ok(Self { config })
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
        for path in &config.root_certificates {
            let pem = std::fs::read(path).map_err(|error| {
                ProbeExecutionError::Tls(format!("cannot read root certificate {path}: {error}"))
            })?;
            let certificate = reqwest::Certificate::from_pem(&pem).map_err(|error| {
                ProbeExecutionError::Tls(format!("invalid root certificate {path}: {error}"))
            })?;
            builder = builder.add_root_certificate(certificate);
        }

        builder
            .build()
            .map_err(|e| ProbeExecutionError::Internal(e.to_string()))
    }

    fn config_for_check(
        &self,
        check: &ResolvedCheck,
    ) -> Result<HttpProbeConfig, ProbeExecutionError> {
        let mut config = self.config.clone();
        if let Some(url) = check.params.get("url") {
            config.url = url.clone();
        }
        if config.url.is_empty() {
            return Err(ProbeExecutionError::Internal(
                "HTTP check has no configured URL".to_owned(),
            ));
        }
        if let Some(method) = check.params.get("method") {
            config.method = Some(method.clone());
        }
        if let Some(status) = check.params.get("expected_status") {
            config.expected_status = Some(status.parse().map_err(|_| {
                ProbeExecutionError::Internal("invalid expected HTTP status".to_owned())
            })?);
        }
        if let Some(range) = check.params.get("expected_status_range") {
            let (start, end) = range.split_once('-').ok_or_else(|| {
                ProbeExecutionError::Internal("invalid expected HTTP status range".to_owned())
            })?;
            config.expected_status_range = Some((
                start.parse().map_err(|_| {
                    ProbeExecutionError::Internal("invalid expected HTTP status range".to_owned())
                })?,
                end.parse().map_err(|_| {
                    ProbeExecutionError::Internal("invalid expected HTTP status range".to_owned())
                })?,
            ));
        }
        if let Some(headers) = check.params.get("headers") {
            config.headers = serde_json::from_str(headers)
                .map_err(|e| ProbeExecutionError::Internal(format!("invalid HTTP headers: {e}")))?;
        }
        config.body = check.params.get("body").cloned();
        config.follow_redirects = check
            .params
            .get("follow_redirects")
            .and_then(|value| value.parse().ok())
            .unwrap_or(config.follow_redirects);
        config.max_redirects = check
            .params
            .get("max_redirect_count")
            .and_then(|value| value.parse().ok())
            .unwrap_or(config.max_redirects);
        config.tls_validate = check
            .params
            .get("tls_validate")
            .and_then(|value| value.parse().ok())
            .unwrap_or(config.tls_validate);
        config.user_agent = check
            .params
            .get("user_agent")
            .cloned()
            .or(config.user_agent);
        config.measure_until = check
            .params
            .get("measure_until")
            .cloned()
            .unwrap_or(config.measure_until);
        if let Some(certificates) = check.params.get("root_certificates") {
            config.root_certificates = serde_json::from_str(certificates).map_err(|error| {
                ProbeExecutionError::Internal(format!("invalid root certificates: {error}"))
            })?;
        }
        config.connection_mode = match check.params.get("connection_mode").map(String::as_str) {
            Some("per_round") => HttpConnectionMode::PerRound,
            Some("fresh") => HttpConnectionMode::Fresh,
            _ => HttpConnectionMode::Pooled,
        };
        Ok(config)
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
        if let Some(body) = self.config.body.as_ref() {
            request_builder = request_builder.body(body.clone());
        }

        let start = std::time::Instant::now();
        match request_builder.send().await {
            Ok(response) => {
                let status = response.status().as_u16();
                if self.config.measure_until == "body"
                    && let Err(error) = response.bytes().await
                {
                    return SampleResult {
                        outcome: Self::classify_request_error(&error),
                        latency: None,
                        detail: Some(error.to_string()),
                        metadata: None,
                    };
                }
                let elapsed = start.elapsed();
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
                let outcome = Self::classify_request_error(&err);
                SampleResult {
                    outcome,
                    latency: None,
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
        let effective = self.config_for_check(&check)?;
        let client = Arc::new(Self::build_client(&effective)?);

        let probe = Self { config: effective };
        let result = probe.execute_single(&client, check.timeout).await;

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
