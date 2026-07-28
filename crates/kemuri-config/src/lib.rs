use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;

use kemuri_core::{
    CheckId, CheckRevisionId, ConfigGeneration, NotifierId, ProbeKind, ProfileId, RuleId, TargetId,
    parse_duration,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("validation error at {path}: {message}")]
    Validation { path: String, message: String },
    #[error("invalid duration: {0}")]
    Duration(#[from] kemuri_core::DurationParseError),
    #[error("invalid percentage: {0}")]
    Percentage(#[from] kemuri_core::PercentageParseError),
}

impl ConfigError {
    fn validation(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Validation {
            path: path.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigWarning {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KemuriConfig {
    pub version: u32,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub scheduler: SchedulerConfig,
    #[serde(default)]
    pub profiles: Vec<ProbeProfileConfig>,
    #[serde(default)]
    pub notifiers: Vec<NotifierConfig>,
    #[serde(default)]
    pub rules: Vec<AlertRuleConfig>,
    #[serde(default)]
    pub targets: Vec<TargetConfig>,
}

impl KemuriConfig {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = serde_yaml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    pub fn parse(yaml: &str) -> Result<Self, ConfigError> {
        let config: Self = serde_yaml::from_str(yaml)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        self.validate_with_warnings().map(|_| ())
    }

    pub fn validate_with_warnings(&self) -> Result<Vec<ConfigWarning>, ConfigError> {
        let mut warnings = Vec::new();

        if self.version != 1 {
            return Err(ConfigError::validation(
                "version",
                format!("unsupported config version: {}", self.version),
            ));
        }

        let profile_ids: HashMap<ProfileId, ProbeKind> = {
            let mut map = HashMap::new();
            let mut seen = HashSet::new();
            for profile in &self.profiles {
                let id = profile.id().clone();
                if !seen.insert(id.clone()) {
                    return Err(ConfigError::validation(
                        format!("profiles.{}", id),
                        "duplicate profile id",
                    ));
                }
                profile.validate()?;
                map.insert(id, profile.kind());
            }
            map
        };

        {
            let mut seen = HashSet::new();
            for notifier in &self.notifiers {
                let id = notifier.id().clone();
                if !seen.insert(id.clone()) {
                    return Err(ConfigError::validation(
                        format!("notifiers.{}", id),
                        "duplicate notifier id",
                    ));
                }
                notifier.collect_warnings(&mut warnings);
            }
        }

        {
            let notifier_ids: HashSet<_> = self.notifiers.iter().map(|n| n.id().clone()).collect();
            let mut seen = HashSet::new();
            for rule in &self.rules {
                let id = rule.id.clone();
                if !seen.insert(id.clone()) {
                    return Err(ConfigError::validation(
                        format!("rules.{}", id),
                        "duplicate rule id",
                    ));
                }
                if !profile_ids.contains_key(&rule.profile) {
                    return Err(ConfigError::validation(
                        format!("rules.{}.profile", id),
                        format!("references unknown profile: {}", rule.profile),
                    ));
                }
                if !notifier_ids.contains(&rule.notifier) {
                    return Err(ConfigError::validation(
                        format!("rules.{}.notifier", id),
                        format!("references unknown notifier: {}", rule.notifier),
                    ));
                }
                parse_duration(&rule.window).map_err(|e| {
                    ConfigError::validation(format!("rules.{}.window", id), e.to_string())
                })?;
            }
        }

        {
            let mut seen = HashSet::new();
            for target in &self.targets {
                let id = target.id.clone();
                if !seen.insert(id.clone()) {
                    return Err(ConfigError::validation(
                        format!("targets.{}", id),
                        "duplicate target id",
                    ));
                }
                target.validate(&profile_ids, &mut warnings)?;
            }
        }

        Ok(warnings)
    }

    pub fn generation_hash(&self) -> ConfigGeneration {
        use sha2::{Digest, Sha256};
        let yaml = serde_yaml::to_string(self).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(yaml.as_bytes());
        let hash = hasher.finalize();
        ConfigGeneration::new(hex::encode(hash))
    }

    pub fn resolve(&self) -> Result<ResolvedConfig, ConfigError> {
        self.validate()?;
        let profile_map: HashMap<ProfileId, &ProbeProfileConfig> =
            self.profiles.iter().map(|p| (p.id().clone(), p)).collect();

        let mut checks = Vec::new();
        for target in &self.targets {
            for check_cfg in &target.checks {
                let profile = profile_map.get(&check_cfg.profile).ok_or_else(|| {
                    ConfigError::validation(
                        format!("targets.{}.checks.{}.profile", target.id, check_cfg.id),
                        format!("references unknown profile: {}", check_cfg.profile),
                    )
                })?;

                let resolved = profile.resolve_check(target, check_cfg)?;
                checks.push(resolved);
            }
        }

        Ok(ResolvedConfig {
            generation: self.generation_hash(),
            checks,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    pub generation: ConfigGeneration,
    pub checks: Vec<ResolvedCheckDef>,
}

#[derive(Debug, Clone)]
pub struct ResolvedCheckDef {
    pub check_id: CheckId,
    pub target_id: TargetId,
    pub target_address: String,
    pub profile_id: ProfileId,
    pub probe_kind: ProbeKind,
    pub interval: std::time::Duration,
    pub timeout: std::time::Duration,
    pub revision_id: CheckRevisionId,
    pub probe_params: ResolvedProbeParams,
}

#[derive(Debug, Clone)]
pub enum ResolvedProbeParams {
    Icmp(ResolvedIcmpParams),
    Http(ResolvedHttpParams),
    Tcp(ResolvedTcpParams),
    Dns(ResolvedDnsParams),
}

#[derive(Debug, Clone)]
pub struct ResolvedIcmpParams {
    pub count: u32,
}

#[derive(Debug, Clone)]
pub struct ResolvedHttpParams {
    pub url: String,
    pub method: Option<String>,
    pub headers: HashMap<String, String>,
    pub expected_status: Option<u16>,
    pub body: Option<SecretRef>,
}

#[derive(Debug, Clone)]
pub struct ResolvedTcpParams {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone)]
pub struct ResolvedDnsParams {
    pub domain: String,
    pub record_type: Option<String>,
    pub resolver: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default)]
    pub cors: bool,
    #[serde(default)]
    pub public_url: Option<String>,
    #[serde(default = "default_shutdown_timeout")]
    pub shutdown_timeout: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            port: default_port(),
            cors: false,
            public_url: None,
            shutdown_timeout: default_shutdown_timeout(),
        }
    }
}

fn default_bind() -> String {
    "127.0.0.1".to_owned()
}

fn default_port() -> u16 {
    8080
}

fn default_shutdown_timeout() -> String {
    "30s".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default)]
    pub format: LogFormat,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            format: LogFormat::default(),
        }
    }
}

fn default_log_level() -> String {
    "info".to_owned()
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogFormat {
    #[default]
    Plain,
    Json,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageConfig {
    #[serde(default = "default_db_path")]
    pub path: String,
    #[serde(default)]
    pub retention: RetentionConfig,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            path: default_db_path(),
            retention: RetentionConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionConfig {
    #[serde(default = "default_raw_retention")]
    pub raw_rounds: String,
    #[serde(default = "default_rollup_5m_retention")]
    pub rollup_5m: String,
    #[serde(default = "default_rollup_1h_retention")]
    pub rollup_1h: String,
    #[serde(default = "default_alert_events_retention")]
    pub alert_events: String,
    #[serde(default = "default_notification_retention")]
    pub notification_records: String,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            raw_rounds: default_raw_retention(),
            rollup_5m: default_rollup_5m_retention(),
            rollup_1h: default_rollup_1h_retention(),
            alert_events: default_alert_events_retention(),
            notification_records: default_notification_retention(),
        }
    }
}

impl RetentionConfig {
    pub fn parse_raw_retention(&self) -> Option<std::time::Duration> {
        parse_retention(&self.raw_rounds)
    }

    pub fn parse_rollup_5m_retention(&self) -> Option<std::time::Duration> {
        parse_retention(&self.rollup_5m)
    }

    pub fn parse_rollup_1h_retention(&self) -> Option<std::time::Duration> {
        parse_retention(&self.rollup_1h)
    }

    pub fn parse_alert_events_retention(&self) -> Option<std::time::Duration> {
        parse_retention(&self.alert_events)
    }

    pub fn parse_notification_retention(&self) -> Option<std::time::Duration> {
        parse_retention(&self.notification_records)
    }
}

fn parse_retention(s: &str) -> Option<std::time::Duration> {
    if s == "forever" {
        return None;
    }
    parse_duration(s).ok()
}

fn default_raw_retention() -> String {
    "7d".to_owned()
}

fn default_rollup_5m_retention() -> String {
    "90d".to_owned()
}

fn default_rollup_1h_retention() -> String {
    "forever".to_owned()
}

fn default_alert_events_retention() -> String {
    "30d".to_owned()
}

fn default_notification_retention() -> String {
    "30d".to_owned()
}

fn default_db_path() -> String {
    "kemuri.db".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchedulerConfig {
    #[serde(default = "default_tick_interval")]
    pub tick_interval: String,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: u32,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            tick_interval: default_tick_interval(),
            max_concurrent: default_max_concurrent(),
        }
    }
}

fn default_tick_interval() -> String {
    "1s".to_owned()
}

fn default_max_concurrent() -> u32 {
    64
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProbeProfileConfig {
    Icmp(IcmpProfileParams),
    Http(HttpProfileParams),
    Tcp(TcpProfileParams),
    Dns(DnsProfileParams),
}

impl ProbeProfileConfig {
    pub fn id(&self) -> &ProfileId {
        match self {
            Self::Icmp(p) => &p.id,
            Self::Http(p) => &p.id,
            Self::Tcp(p) => &p.id,
            Self::Dns(p) => &p.id,
        }
    }

    pub fn kind(&self) -> ProbeKind {
        match self {
            Self::Icmp(_) => ProbeKind::Icmp,
            Self::Http(_) => ProbeKind::Http,
            Self::Tcp(_) => ProbeKind::Tcp,
            Self::Dns(_) => ProbeKind::Dns,
        }
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        let (interval_str, timeout_str, path) = match self {
            Self::Icmp(p) => (&p.interval, &p.timeout, format!("profiles.{}", p.id)),
            Self::Http(p) => (&p.interval, &p.timeout, format!("profiles.{}", p.id)),
            Self::Tcp(p) => (&p.interval, &p.timeout, format!("profiles.{}", p.id)),
            Self::Dns(p) => (&p.interval, &p.timeout, format!("profiles.{}", p.id)),
        };
        parse_duration(interval_str)
            .map_err(|e| ConfigError::validation(format!("{}.interval", path), e.to_string()))?;
        parse_duration(timeout_str)
            .map_err(|e| ConfigError::validation(format!("{}.timeout", path), e.to_string()))?;
        Ok(())
    }

    pub fn interval_str(&self) -> &str {
        match self {
            Self::Icmp(p) => &p.interval,
            Self::Http(p) => &p.interval,
            Self::Tcp(p) => &p.interval,
            Self::Dns(p) => &p.interval,
        }
    }

    pub fn timeout_str(&self) -> &str {
        match self {
            Self::Icmp(p) => &p.timeout,
            Self::Http(p) => &p.timeout,
            Self::Tcp(p) => &p.timeout,
            Self::Dns(p) => &p.timeout,
        }
    }

    fn resolve_check(
        &self,
        target: &TargetConfig,
        check: &CheckConfig,
    ) -> Result<ResolvedCheckDef, ConfigError> {
        let check_path = format!("targets.{}.checks.{}", target.id, check.id);
        if let Some(ref kind) = check.kind
            && *kind != self.kind()
        {
            return Err(ConfigError::validation(
                format!("{}.kind", check_path),
                format!(
                    "check kind {:?} does not match profile kind {:?}",
                    kind,
                    self.kind()
                ),
            ));
        }
        let interval = match &check.interval {
            Some(s) => parse_duration(s).map_err(|e| {
                ConfigError::validation(format!("{}.interval", check_path), e.to_string())
            })?,
            None => parse_duration(self.interval_str()).map_err(|e| {
                ConfigError::validation(format!("profiles.{}.interval", self.id()), e.to_string())
            })?,
        };
        let timeout = match &check.timeout {
            Some(s) => parse_duration(s).map_err(|e| {
                ConfigError::validation(format!("{}.timeout", check_path), e.to_string())
            })?,
            None => parse_duration(self.timeout_str()).map_err(|e| {
                ConfigError::validation(format!("profiles.{}.timeout", self.id()), e.to_string())
            })?,
        };
        let probe_params = match self {
            Self::Icmp(p) => {
                let count = check.count.unwrap_or(p.count);
                ResolvedProbeParams::Icmp(ResolvedIcmpParams { count })
            }
            Self::Http(p) => {
                let url = check.url.as_deref().unwrap_or(&p.url).to_owned();
                let method = check.method.as_ref().or(p.method.as_ref()).cloned();
                let headers = match (&check.headers, &p.headers) {
                    (Some(override_h), Some(base_h)) => {
                        let mut merged = base_h.clone();
                        merged.extend(override_h.clone());
                        merged
                    }
                    (Some(h), None) | (None, Some(h)) => h.clone(),
                    (None, None) => HashMap::new(),
                };
                let expected_status = check.expected_status.or(p.expected_status);
                let body = check.body.as_ref().or(p.body.as_ref()).cloned();
                ResolvedProbeParams::Http(ResolvedHttpParams {
                    url,
                    method,
                    headers,
                    expected_status,
                    body,
                })
            }
            Self::Tcp(p) => {
                let host = check.host.as_deref().unwrap_or(&p.host).to_owned();
                let port = check.port.unwrap_or(p.port);
                ResolvedProbeParams::Tcp(ResolvedTcpParams { host, port })
            }
            Self::Dns(p) => {
                let domain = check.domain.as_deref().unwrap_or(&p.domain).to_owned();
                let record_type = check
                    .record_type
                    .as_ref()
                    .or(p.record_type.as_ref())
                    .cloned();
                let resolver = check.resolver.as_ref().or(p.resolver.as_ref()).cloned();
                ResolvedProbeParams::Dns(ResolvedDnsParams {
                    domain,
                    record_type,
                    resolver,
                })
            }
        };
        let revision_input = format!(
            "{}:{}:{}:{}:{:?}",
            check.id,
            self.id(),
            interval.as_nanos(),
            timeout.as_nanos(),
            probe_params,
        );
        let revision_id = compute_revision_id(&revision_input);
        Ok(ResolvedCheckDef {
            check_id: check.id.clone(),
            target_id: target.id.clone(),
            target_address: target.address.clone(),
            profile_id: check.profile.clone(),
            probe_kind: self.kind(),
            interval,
            timeout,
            revision_id,
            probe_params,
        })
    }
}

fn compute_revision_id(input: &str) -> CheckRevisionId {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    let hash = hasher.finalize();
    let hex_str = hex::encode(&hash[..8]);
    CheckRevisionId::new(format!("rev-{}", hex_str))
        .unwrap_or_else(|_| CheckRevisionId::new("rev-unknown").unwrap())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IcmpProfileParams {
    pub id: ProfileId,
    #[serde(default = "default_interval")]
    pub interval: String,
    #[serde(default = "default_timeout")]
    pub timeout: String,
    #[serde(default = "default_count")]
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpProfileParams {
    pub id: ProfileId,
    #[serde(default = "default_interval")]
    pub interval: String,
    #[serde(default = "default_timeout")]
    pub timeout: String,
    pub url: String,
    pub method: Option<String>,
    pub headers: Option<HashMap<String, String>>,
    pub expected_status: Option<u16>,
    pub body: Option<SecretRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TcpProfileParams {
    pub id: ProfileId,
    #[serde(default = "default_interval")]
    pub interval: String,
    #[serde(default = "default_timeout")]
    pub timeout: String,
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DnsProfileParams {
    pub id: ProfileId,
    #[serde(default = "default_interval")]
    pub interval: String,
    #[serde(default = "default_timeout")]
    pub timeout: String,
    pub domain: String,
    pub record_type: Option<String>,
    pub resolver: Option<String>,
}

fn default_interval() -> String {
    "30s".to_owned()
}

fn default_timeout() -> String {
    "5s".to_owned()
}

fn default_count() -> u32 {
    3
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NotifierConfig {
    Webhook(WebhookNotifierParams),
    Smtp(SmtpNotifierParams),
}

impl NotifierConfig {
    pub fn id(&self) -> &NotifierId {
        match self {
            Self::Webhook(p) => &p.id,
            Self::Smtp(p) => &p.id,
        }
    }

    fn collect_warnings(&self, warnings: &mut Vec<ConfigWarning>) {
        match self {
            Self::Webhook(p) => {
                if let SecretRef::Literal(_) = &p.url {
                    warnings.push(ConfigWarning {
                        path: format!("notifiers.{}.url", p.id),
                        message: "literal secret value for URL; use from_env or from_file"
                            .to_owned(),
                    });
                }
                if let Some(ref headers) = p.headers {
                    for (key, value) in headers {
                        if let SecretRef::Literal(_) = value {
                            warnings.push(ConfigWarning {
                                path: format!("notifiers.{}.headers.{}", p.id, key),
                                message:
                                    "literal secret value in header; use from_env or from_file"
                                        .to_owned(),
                            });
                        }
                    }
                }
            }
            Self::Smtp(p) => {
                if let Some(SecretRef::Literal(_)) = &p.password {
                    warnings.push(ConfigWarning {
                        path: format!("notifiers.{}.password", p.id),
                        message: "literal secret value for password; use from_env or from_file"
                            .to_owned(),
                    });
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebhookNotifierParams {
    pub id: NotifierId,
    pub url: SecretRef,
    #[serde(default)]
    pub headers: Option<HashMap<String, SecretRef>>,
    #[serde(default = "default_webhook_timeout")]
    pub timeout: String,
}

fn default_webhook_timeout() -> String {
    "10s".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SmtpNotifierParams {
    pub id: NotifierId,
    pub host: String,
    pub port: u16,
    pub from: String,
    pub to: Vec<String>,
    pub username: Option<String>,
    pub password: Option<SecretRef>,
    #[serde(default = "default_tls_mode")]
    pub tls_mode: String,
    #[serde(default = "default_smtp_timeout")]
    pub timeout: String,
}

fn default_tls_mode() -> String {
    "required".to_owned()
}

fn default_smtp_timeout() -> String {
    "30s".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlertRuleConfig {
    pub id: RuleId,
    pub profile: ProfileId,
    #[serde(default = "default_metric")]
    pub metric: String,
    #[serde(default = "default_operator")]
    pub operator: String,
    pub threshold: String,
    pub window: String,
    pub notifier: NotifierId,
    #[serde(default)]
    pub duration: Option<String>,
    #[serde(default)]
    pub clear_threshold: Option<String>,
    #[serde(default)]
    pub clear_operator: Option<String>,
    #[serde(default)]
    pub repeat_every: Option<String>,
    #[serde(default)]
    pub minimum_rounds: Option<u32>,
    #[serde(default)]
    pub no_data_period: Option<String>,
}

fn default_metric() -> String {
    "measurement_loss_ratio".to_owned()
}

fn default_operator() -> String {
    "gte".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetConfig {
    pub id: TargetId,
    pub address: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub group_path: Option<String>,
    #[serde(default)]
    pub labels: Option<HashMap<String, String>>,
    #[serde(default)]
    pub checks: Vec<CheckConfig>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

impl TargetConfig {
    pub fn validate(
        &self,
        profile_ids: &HashMap<ProfileId, ProbeKind>,
        warnings: &mut Vec<ConfigWarning>,
    ) -> Result<(), ConfigError> {
        let mut seen = HashSet::new();
        for check in &self.checks {
            if !seen.insert(check.id.clone()) {
                return Err(ConfigError::validation(
                    format!("targets.{}.checks.{}", self.id, check.id),
                    "duplicate check id within target",
                ));
            }
            if !profile_ids.contains_key(&check.profile) {
                return Err(ConfigError::validation(
                    format!("targets.{}.checks.{}.profile", self.id, check.id),
                    format!("references unknown profile: {}", check.profile),
                ));
            }
            check.collect_warnings(format!("targets.{}.checks.{}", self.id, check.id), warnings);
        }
        Ok(())
    }
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckConfig {
    pub id: CheckId,
    pub profile: ProfileId,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub kind: Option<ProbeKind>,
    #[serde(default)]
    pub interval: Option<String>,
    #[serde(default)]
    pub timeout: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub expected_status: Option<u16>,
    #[serde(default)]
    pub body: Option<SecretRef>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub record_type: Option<String>,
    #[serde(default)]
    pub resolver: Option<String>,
    #[serde(default)]
    pub count: Option<u32>,
}

impl CheckConfig {
    fn collect_warnings(&self, path: String, warnings: &mut Vec<ConfigWarning>) {
        if let Some(SecretRef::Literal(_)) = &self.body {
            warnings.push(ConfigWarning {
                path: format!("{}.body", path),
                message: "literal secret value for body; use from_env or from_file".to_owned(),
            });
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SecretRef {
    Literal(String),
    FromEnv { from_env: String },
    FromFile { from_file: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigDiff {
    pub added_targets: Vec<TargetId>,
    pub removed_targets: Vec<TargetId>,
    pub modified_targets: Vec<TargetId>,
    pub added_checks: Vec<(TargetId, CheckId)>,
    pub removed_checks: Vec<(TargetId, CheckId)>,
    pub modified_checks: Vec<(TargetId, CheckId)>,
}

impl ConfigDiff {
    pub fn compute(old: &KemuriConfig, new: &KemuriConfig) -> Self {
        let old_targets: HashMap<_, _> = old.targets.iter().map(|t| (t.id.clone(), t)).collect();
        let new_targets: HashMap<_, _> = new.targets.iter().map(|t| (t.id.clone(), t)).collect();

        let mut added_targets = Vec::new();
        let mut removed_targets = Vec::new();
        let mut modified_targets = Vec::new();
        let mut added_checks = Vec::new();
        let mut removed_checks = Vec::new();
        let mut modified_checks = Vec::new();

        for id in new_targets.keys() {
            if !old_targets.contains_key(id) {
                added_targets.push(id.clone());
            }
        }
        for id in old_targets.keys() {
            if !new_targets.contains_key(id) {
                removed_targets.push(id.clone());
            }
        }

        for (id, new_target) in &new_targets {
            if let Some(old_target) = old_targets.get(id) {
                if old_target.address != new_target.address
                    || old_target.name != new_target.name
                    || old_target.group_path != new_target.group_path
                    || old_target.labels != new_target.labels
                    || old_target.enabled != new_target.enabled
                {
                    modified_targets.push(id.clone());
                }
                let old_checks: HashMap<_, _> = old_target
                    .checks
                    .iter()
                    .map(|c| (c.id.clone(), c))
                    .collect();
                let new_checks: HashMap<_, _> = new_target
                    .checks
                    .iter()
                    .map(|c| (c.id.clone(), c))
                    .collect();
                for check_id in new_checks.keys() {
                    if !old_checks.contains_key(check_id) {
                        added_checks.push((id.clone(), check_id.clone()));
                    }
                }
                for check_id in old_checks.keys() {
                    if !new_checks.contains_key(check_id) {
                        removed_checks.push((id.clone(), check_id.clone()));
                    }
                }
                for (check_id, new_check) in &new_checks {
                    if let Some(old_check) = old_checks.get(check_id)
                        && old_check != new_check
                    {
                        modified_checks.push((id.clone(), check_id.clone()));
                    }
                }
            }
        }

        ConfigDiff {
            added_targets,
            removed_targets,
            modified_targets,
            added_checks,
            removed_checks,
            modified_checks,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.added_targets.is_empty()
            && self.removed_targets.is_empty()
            && self.modified_targets.is_empty()
            && self.added_checks.is_empty()
            && self.removed_checks.is_empty()
            && self.modified_checks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minimal_valid_config() {
        let yaml = "version: 1\n";
        let config: KemuriConfig = serde_yaml::from_str(yaml).unwrap();
        config.validate().unwrap();
    }

    #[test]
    fn reject_invalid_version() {
        let yaml = "version: 2\n";
        let config: KemuriConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn reject_unknown_fields() {
        let yaml = "version: 1\nunknown_field: true\n";
        let result: Result<KemuriConfig, _> = serde_yaml::from_str(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn server_config_defaults() {
        let yaml = "version: 1\n";
        let config: KemuriConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.server.bind, "127.0.0.1");
        assert_eq!(config.server.port, 8080);
        assert!(!config.server.cors);
    }

    #[test]
    fn load_from_file() {
        let dir = std::env::temp_dir().join("kemuri_test_config");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.yaml");
        std::fs::write(
            &path,
            "version: 1\nserver:\n  bind: 0.0.0.0\n  port: 9090\n",
        )
        .unwrap();
        let config = KemuriConfig::load(&path).unwrap();
        assert_eq!(config.server.bind, "0.0.0.0");
        assert_eq!(config.server.port, 9090);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reject_duplicate_profile_id() {
        let yaml = r#"
version: 1
profiles:
  - kind: icmp
    id: dup
    interval: 30s
    timeout: 5s
  - kind: icmp
    id: dup
    interval: 60s
    timeout: 10s
"#;
        let config: KemuriConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn reject_duplicate_notifier_id() {
        let yaml = r#"
version: 1
notifiers:
  - kind: webhook
    id: dup
    url: http://example.com
  - kind: webhook
    id: dup
    url: http://example.org
"#;
        let config: KemuriConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn reject_duplicate_rule_id() {
        let yaml = r#"
version: 1
profiles:
  - kind: icmp
    id: p1
notifiers:
  - kind: webhook
    id: n1
    url: http://example.com
rules:
  - id: dup
    profile: p1
    threshold: "10%"
    window: 5m
    notifier: n1
  - id: dup
    profile: p1
    threshold: "20%"
    window: 10m
    notifier: n1
"#;
        let config: KemuriConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn reject_duplicate_target_id() {
        let yaml = r#"
version: 1
targets:
  - id: dup
    address: 1.1.1.1
  - id: dup
    address: 2.2.2.2
"#;
        let config: KemuriConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn reject_duplicate_check_id_within_target() {
        let yaml = r#"
version: 1
profiles:
  - kind: icmp
    id: p1
targets:
  - id: t1
    address: 1.1.1.1
    checks:
      - id: dup
        profile: p1
      - id: dup
        profile: p1
"#;
        let config: KemuriConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.validate().is_err());
    }

    #[test]
    fn reject_rule_unknown_profile() {
        let yaml = r#"
version: 1
notifiers:
  - kind: webhook
    id: n1
    url: http://example.com
rules:
  - id: r1
    profile: nonexistent
    threshold: "10%"
    window: 5m
    notifier: n1
"#;
        let config: KemuriConfig = serde_yaml::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("unknown profile"));
    }

    #[test]
    fn reject_rule_unknown_notifier() {
        let yaml = r#"
version: 1
profiles:
  - kind: icmp
    id: p1
rules:
  - id: r1
    profile: p1
    threshold: "10%"
    window: 5m
    notifier: nonexistent
"#;
        let config: KemuriConfig = serde_yaml::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("unknown notifier"));
    }

    #[test]
    fn reject_check_unknown_profile() {
        let yaml = r#"
version: 1
targets:
  - id: t1
    address: 1.1.1.1
    checks:
      - id: c1
        profile: nonexistent
"#;
        let config: KemuriConfig = serde_yaml::from_str(yaml).unwrap();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("unknown profile"));
    }

    #[test]
    fn reject_check_kind_mismatch() {
        let yaml = r#"
version: 1
profiles:
  - kind: icmp
    id: p1
targets:
  - id: t1
    address: 1.1.1.1
    checks:
      - id: c1
        profile: p1
        kind: http
"#;
        let config: KemuriConfig = serde_yaml::from_str(yaml).unwrap();
        let resolved = config.resolve();
        assert!(resolved.is_err());
        let err = resolved.unwrap_err();
        assert!(err.to_string().contains("does not match profile kind"));
    }

    #[test]
    fn warn_literal_secret_in_smtp_password() {
        let yaml = r#"
version: 1
notifiers:
  - kind: smtp
    id: n1
    host: smtp.example.com
    port: 587
    from: test@example.com
    to:
      - admin@example.com
    password: literal-secret
"#;
        let config: KemuriConfig = serde_yaml::from_str(yaml).unwrap();
        let warnings = config.validate_with_warnings().unwrap();
        assert!(!warnings.is_empty());
        assert!(warnings[0].message.contains("literal secret"));
    }

    #[test]
    fn warn_literal_secret_in_check_body() {
        let yaml = r#"
version: 1
profiles:
  - kind: http
    id: p1
    url: http://example.com
targets:
  - id: t1
    address: example.com
    checks:
      - id: c1
        profile: p1
        body: literal-secret
"#;
        let config: KemuriConfig = serde_yaml::from_str(yaml).unwrap();
        let warnings = config.validate_with_warnings().unwrap();
        assert!(
            warnings
                .iter()
                .any(|w| w.message.contains("literal secret"))
        );
    }

    #[test]
    fn no_warning_for_env_secret() {
        let yaml = r#"
version: 1
notifiers:
  - kind: smtp
    id: n1
    host: smtp.example.com
    port: 587
    from: test@example.com
    to:
      - admin@example.com
    password:
      from_env: SMTP_PASSWORD
"#;
        let config: KemuriConfig = serde_yaml::from_str(yaml).unwrap();
        let warnings = config.validate_with_warnings().unwrap();
        assert!(warnings.is_empty());
    }

    #[test]
    fn profile_resolution_http_override() {
        let yaml = r#"
version: 1
profiles:
  - kind: http
    id: http-default
    url: http://example.com
    interval: 30s
    timeout: 5s
targets:
  - id: t1
    address: example.com
    checks:
      - id: c1
        profile: http-default
        interval: 15s
        timeout: 3s
"#;
        let config: KemuriConfig = serde_yaml::from_str(yaml).unwrap();
        let resolved = config.resolve().unwrap();
        assert_eq!(resolved.checks.len(), 1);
        let check = &resolved.checks[0];
        assert_eq!(check.interval, std::time::Duration::from_secs(15));
        assert_eq!(check.timeout, std::time::Duration::from_secs(3));
    }

    #[test]
    fn profile_resolution_headers_merge() {
        let yaml = r#"
version: 1
profiles:
  - kind: http
    id: http-default
    url: http://example.com
    headers:
      Accept: application/json
      X-Base: value
targets:
  - id: t1
    address: example.com
    checks:
      - id: c1
        profile: http-default
        headers:
          X-Override: yes
          X-Base: overridden
"#;
        let config: KemuriConfig = serde_yaml::from_str(yaml).unwrap();
        let resolved = config.resolve().unwrap();
        let check = &resolved.checks[0];
        if let ResolvedProbeParams::Http(ref params) = check.probe_params {
            assert_eq!(params.headers.get("X-Base").unwrap(), "overridden");
            assert_eq!(params.headers.get("Accept").unwrap(), "application/json");
            assert_eq!(params.headers.get("X-Override").unwrap(), "yes");
        } else {
            panic!("expected HTTP params");
        }
    }

    #[test]
    fn config_diff_detects_changes() {
        let old_yaml = r#"
version: 1
profiles:
  - kind: icmp
    id: p1
targets:
  - id: t1
    address: 1.1.1.1
    checks:
      - id: c1
        profile: p1
  - id: t2
    address: 2.2.2.2
"#;
        let new_yaml = r#"
version: 1
profiles:
  - kind: icmp
    id: p1
targets:
  - id: t1
    address: 1.1.1.2
    checks:
      - id: c1
        profile: p1
        interval: 15s
  - id: t3
    address: 3.3.3.3
"#;
        let old: KemuriConfig = serde_yaml::from_str(old_yaml).unwrap();
        let new: KemuriConfig = serde_yaml::from_str(new_yaml).unwrap();
        let diff = ConfigDiff::compute(&old, &new);
        assert!(diff.added_targets.contains(&TargetId::new("t3").unwrap()));
        assert!(diff.removed_targets.contains(&TargetId::new("t2").unwrap()));
        assert!(
            diff.modified_targets
                .contains(&TargetId::new("t1").unwrap())
        );
        assert!(
            diff.modified_checks
                .contains(&(TargetId::new("t1").unwrap(), CheckId::new("c1").unwrap()))
        );
    }

    #[test]
    fn config_diff_empty() {
        let yaml = r#"
version: 1
targets:
  - id: t1
    address: 1.1.1.1
"#;
        let config: KemuriConfig = serde_yaml::from_str(yaml).unwrap();
        let diff = ConfigDiff::compute(&config, &config);
        assert!(diff.is_empty());
    }

    #[test]
    fn generation_hash_deterministic() {
        let yaml = "version: 1\n";
        let config: KemuriConfig = serde_yaml::from_str(yaml).unwrap();
        let h1 = config.generation_hash();
        let h2 = config.generation_hash();
        assert_eq!(h1, h2);
    }

    #[test]
    fn generation_hash_changes_with_config() {
        let c1: KemuriConfig = serde_yaml::from_str("version: 1\n").unwrap();
        let c2: KemuriConfig = serde_yaml::from_str("version: 1\nserver:\n  port: 9090\n").unwrap();
        assert_ne!(c1.generation_hash(), c2.generation_hash());
    }

    #[test]
    fn resolve_full_config() {
        let yaml = r#"
version: 1
profiles:
  - kind: http
    id: http-default
    url: http://example.com/health
    method: GET
    expected_status: 200
    interval: 30s
    timeout: 5s
  - kind: icmp
    id: icmp-default
    interval: 60s
    timeout: 3s
    count: 5
notifiers:
  - kind: webhook
    id: slack
    url: https://hooks.slack.com/test
targets:
  - id: web-1
    address: web1.example.com
    checks:
      - id: health
        profile: http-default
      - id: ping
        profile: icmp-default
"#;
        let config: KemuriConfig = serde_yaml::from_str(yaml).unwrap();
        let resolved = config.resolve().unwrap();
        assert_eq!(resolved.checks.len(), 2);
    }

    #[test]
    fn reject_invalid_duration_in_profile() {
        let yaml = r#"
version: 1
profiles:
  - kind: icmp
    id: p1
    interval: notaduration
    timeout: 5s
"#;
        let config: KemuriConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(config.validate().is_err());
    }
}
