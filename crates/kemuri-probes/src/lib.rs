use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use kemuri_core::{CheckId, ObserverId, ProbeKind, ProfileId, SampleOutcome, TargetId};
use serde::{Deserialize, Serialize};

pub mod dns;
pub mod http;
pub mod icmp;
pub mod synthetic;
pub mod tcp;

pub use dns::{DnsProbe, DnsProbeConfig, DnsProtocol, DnsResponseCode};
pub use http::{HttpConnectionMode, HttpProbe, HttpProbeConfig};
pub use icmp::{
    AddressFamily, IcmpCapability, IcmpProbe, IcmpProbeConfig, SocketMethod, check_icmp_capability,
};
pub use synthetic::{SyntheticProbe, SyntheticProbeConfig};
pub use tcp::{TcpProbe, TcpProbeConfig};

#[derive(Debug, Clone, thiserror::Error)]
pub enum ProbeExecutionError {
    #[error("timeout after {0}")]
    Timeout(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("dns error: {0}")]
    Dns(String),
    #[error("tls error: {0}")]
    Tls(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("permission denied: {0}")]
    Permission(String),
    #[error("cancelled")]
    Cancelled,
    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(Debug, Clone)]
pub struct RoundContext {
    pub observer_id: ObserverId,
    pub scheduled_at: Duration,
    pub deadline: Duration,
}

#[derive(Debug, Clone)]
pub struct ResolvedCheck {
    pub check_id: CheckId,
    pub target_id: TargetId,
    pub profile_id: ProfileId,
    pub address: String,
    pub probe_kind: ProbeKind,
    pub timeout: Duration,
    pub sample_count: u32,
    pub params: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SampleResult {
    pub outcome: SampleOutcome,
    pub latency: Option<Duration>,
    pub detail: Option<String>,
    pub metadata: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeRound {
    pub check_id: CheckId,
    pub results: Vec<SampleResult>,
}

#[async_trait]
pub trait Probe: Send + Sync {
    fn kind(&self) -> ProbeKind;
    async fn execute_round(
        &self,
        context: RoundContext,
        check: ResolvedCheck,
    ) -> Result<ProbeRound, ProbeExecutionError>;
}
