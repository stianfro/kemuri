use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;

use kemuri_core::{
    CheckId, CheckRevisionId, ConfigGeneration, NotifierId, ProbeKind, ProfileId, RuleId, TargetId,
    parse_duration,
};
use rustls::pki_types::pem::PemObject;
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
                let window = parse_duration(&rule.window).map_err(|e| {
                    ConfigError::validation(format!("rules.{}.window", id), e.to_string())
                })?;
                if window.is_zero() {
                    return Err(ConfigError::validation(
                        format!("rules.{}.window", id),
                        "must be greater than zero",
                    ));
                }
                if ![
                    "measurement_loss_ratio",
                    "health_failure_ratio",
                    "healthy_sample_ratio",
                    "consecutive_total_loss_rounds",
                    "consecutive_unhealthy_rounds",
                    "p50_latency",
                    "p95_latency",
                    "p99_latency",
                    "no_data",
                ]
                .contains(&rule.metric.as_str())
                {
                    return Err(ConfigError::validation(
                        format!("rules.{}.metric", id),
                        "unsupported alert metric",
                    ));
                }
                if !["gt", "gte", "lt", "lte"].contains(&rule.operator.as_str()) {
                    return Err(ConfigError::validation(
                        format!("rules.{}.operator", id),
                        "unsupported alert operator",
                    ));
                }
                if let Some(operator) = rule.clear_operator.as_deref()
                    && !["gt", "gte", "lt", "lte"].contains(&operator)
                {
                    return Err(ConfigError::validation(
                        format!("rules.{}.clear_operator", id),
                        "unsupported alert operator",
                    ));
                }
                validate_alert_threshold(
                    &rule.threshold,
                    &rule.metric,
                    &format!("rules.{}.threshold", id),
                )?;
                if let Some(threshold) = rule.clear_threshold.as_deref() {
                    validate_alert_threshold(
                        threshold,
                        &rule.metric,
                        &format!("rules.{}.clear_threshold", id),
                    )?;
                }
                for (field, value) in [
                    ("duration", rule.duration.as_deref()),
                    ("repeat_every", rule.repeat_every.as_deref()),
                    ("no_data_period", rule.no_data_period.as_deref()),
                ] {
                    if let Some(value) = value {
                        let parsed = parse_duration(value).map_err(|e| {
                            ConfigError::validation(
                                format!("rules.{}.{}", id, field),
                                e.to_string(),
                            )
                        })?;
                        if parsed.is_zero() {
                            return Err(ConfigError::validation(
                                format!("rules.{}.{}", id, field),
                                "must be greater than zero",
                            ));
                        }
                    }
                }
                if rule.minimum_rounds == Some(0) {
                    return Err(ConfigError::validation(
                        format!("rules.{}.minimum_rounds", id),
                        "must be greater than zero",
                    ));
                }
                if rule.minimum_latency_samples == Some(0) {
                    return Err(ConfigError::validation(
                        format!("rules.{}.minimum_latency_samples", id),
                        "must be greater than zero",
                    ));
                }
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

        self.scheduler.validate()?;
        self.storage.validate()?;
        parse_duration(&self.server.shutdown_timeout)
            .map_err(|e| ConfigError::validation("server.shutdown_timeout", e.to_string()))?;

        Ok(warnings)
    }

    pub fn generation_hash(&self) -> ConfigGeneration {
        use sha2::{Digest, Sha256};
        let mut canonical = self.clone();
        canonical
            .profiles
            .sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        canonical
            .notifiers
            .sort_by(|left, right| left.id().as_str().cmp(right.id().as_str()));
        canonical
            .rules
            .sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
        canonical
            .targets
            .sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
        for target in &mut canonical.targets {
            target
                .checks
                .sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));
        }
        let json = serde_json::to_vec(&canonical).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(json);
        let mut secret_values = Vec::new();
        for notifier in &canonical.notifiers {
            match notifier {
                NotifierConfig::Webhook(params) => {
                    if let Ok(value) = params.url.resolve() {
                        secret_values.push((format!("notifier:{}:url", params.id), value));
                    }
                    if let Some(headers) = &params.headers {
                        for (name, secret) in headers {
                            if let Ok(value) = secret.resolve() {
                                secret_values.push((
                                    format!(
                                        "notifier:{}:header:{}",
                                        params.id,
                                        name.to_lowercase()
                                    ),
                                    value,
                                ));
                            }
                        }
                    }
                }
                NotifierConfig::Smtp(params) => {
                    if let Some(secret) = &params.password
                        && let Ok(value) = secret.resolve()
                    {
                        secret_values.push((format!("notifier:{}:password", params.id), value));
                    }
                }
            }
        }
        for target in &canonical.targets {
            for check in &target.checks {
                if let Some(secret) = &check.body
                    && let Ok(value) = secret.resolve()
                {
                    secret_values.push((
                        format!("target:{}:check:{}:body", target.id, check.id),
                        value,
                    ));
                }
            }
        }
        secret_values.sort_by(|left, right| left.0.cmp(&right.0));
        for (key, value) in secret_values {
            hasher.update(key.as_bytes());
            hasher.update([0]);
            hasher.update(value.as_bytes());
            hasher.update([0]);
        }
        let hash = hasher.finalize();
        ConfigGeneration::new(hex::encode(hash))
    }

    pub fn resolve(&self) -> Result<ResolvedConfig, ConfigError> {
        self.validate()?;
        let profile_map: HashMap<ProfileId, &ProbeProfileConfig> =
            self.profiles.iter().map(|p| (p.id().clone(), p)).collect();

        let mut checks = Vec::new();
        for target in &self.targets {
            if !target.enabled {
                continue;
            }
            for check_cfg in &target.checks {
                if !check_cfg.enabled {
                    continue;
                }
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

#[derive(Debug, Clone, Serialize)]
pub enum ResolvedProbeParams {
    Icmp(ResolvedIcmpParams),
    Http(ResolvedHttpParams),
    Tcp(ResolvedTcpParams),
    Dns(ResolvedDnsParams),
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedIcmpParams {
    pub count: u32,
    pub address_family: String,
    pub payload_size: usize,
    pub source_address: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedHttpParams {
    pub url: String,
    pub method: Option<String>,
    pub headers: HashMap<String, String>,
    pub expected_status: Option<u16>,
    pub expected_status_range: Option<(u16, u16)>,
    pub body: Option<String>,
    pub follow_redirects: bool,
    pub max_redirect_count: u32,
    pub connection_mode: String,
    pub measure_until: String,
    pub user_agent: Option<String>,
    pub tls_validate: bool,
    pub root_certificates: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedTcpParams {
    pub host: String,
    pub port: u16,
    pub address_family: String,
    pub source_address: Option<String>,
    pub tls: Option<TcpTlsConfig>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedDnsParams {
    pub domain: String,
    pub record_type: Option<String>,
    pub resolver: Option<String>,
    pub protocol: String,
    pub expected_rcode: String,
    pub require_answer: bool,
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
    #[serde(default)]
    pub disk_pressure: DiskPressureConfig,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            path: default_db_path(),
            retention: RetentionConfig::default(),
            disk_pressure: DiskPressureConfig::default(),
        }
    }
}

impl StorageConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        self.retention.validate()?;
        let warning = parse_percentage_value(
            &self.disk_pressure.warning_free,
            "storage.disk_pressure.warning_free",
        )?;
        let critical = parse_percentage_value(
            &self.disk_pressure.critical_free,
            "storage.disk_pressure.critical_free",
        )?;
        if critical >= warning {
            return Err(ConfigError::validation(
                "storage.disk_pressure",
                "critical_free must be less than warning_free",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiskPressureConfig {
    #[serde(default = "default_warning_free")]
    pub warning_free: String,
    #[serde(default = "default_critical_free")]
    pub critical_free: String,
}

impl Default for DiskPressureConfig {
    fn default() -> Self {
        Self {
            warning_free: default_warning_free(),
            critical_free: default_critical_free(),
        }
    }
}

fn default_warning_free() -> String {
    "10%".to_owned()
}
fn default_critical_free() -> String {
    "5%".to_owned()
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
    fn validate(&self) -> Result<(), ConfigError> {
        for (name, value) in [
            ("raw_rounds", &self.raw_rounds),
            ("rollup_5m", &self.rollup_5m),
            ("rollup_1h", &self.rollup_1h),
            ("alert_events", &self.alert_events),
            ("notification_records", &self.notification_records),
        ] {
            if value != "forever" {
                let duration = parse_duration(value).map_err(|e| {
                    ConfigError::validation(format!("storage.retention.{name}"), e.to_string())
                })?;
                if duration.is_zero() {
                    return Err(ConfigError::validation(
                        format!("storage.retention.{name}"),
                        "retention must be greater than zero",
                    ));
                }
            }
        }
        Ok(())
    }

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
    #[serde(default)]
    pub startup_mode: StartupMode,
    #[serde(default = "default_jitter")]
    pub default_jitter: String,
    #[serde(default)]
    pub max_concurrent_by_probe: ProbeConcurrencyLimits,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self {
            tick_interval: default_tick_interval(),
            max_concurrent: default_max_concurrent(),
            startup_mode: StartupMode::default(),
            default_jitter: default_jitter(),
            max_concurrent_by_probe: ProbeConcurrencyLimits::default(),
        }
    }
}

impl SchedulerConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        let tick = parse_duration(&self.tick_interval)
            .map_err(|e| ConfigError::validation("scheduler.tick_interval", e.to_string()))?;
        if tick.is_zero() {
            return Err(ConfigError::validation(
                "scheduler.tick_interval",
                "must be greater than zero",
            ));
        }
        if self.max_concurrent == 0 {
            return Err(ConfigError::validation(
                "scheduler.max_concurrent",
                "must be greater than zero",
            ));
        }
        parse_percentage_value(&self.default_jitter, "scheduler.default_jitter")?;
        for (kind, limit) in [
            ("icmp", self.max_concurrent_by_probe.icmp),
            ("http", self.max_concurrent_by_probe.http),
            ("tcp", self.max_concurrent_by_probe.tcp),
            ("dns", self.max_concurrent_by_probe.dns),
        ] {
            if limit == Some(0) {
                return Err(ConfigError::validation(
                    format!("scheduler.max_concurrent_by_probe.{kind}"),
                    "must be greater than zero",
                ));
            }
        }
        Ok(())
    }
}

fn parse_percentage_value(value: &str, path: &str) -> Result<f64, ConfigError> {
    let number = value
        .strip_suffix('%')
        .ok_or_else(|| ConfigError::validation(path, "must be a percentage such as 10%"))?
        .parse::<f64>()
        .map_err(|_| ConfigError::validation(path, "must be a valid percentage"))?;
    if !(0.0..=100.0).contains(&number) {
        return Err(ConfigError::validation(path, "must be between 0% and 100%"));
    }
    Ok(number / 100.0)
}

fn validate_alert_threshold(value: &str, metric: &str, path: &str) -> Result<(), ConfigError> {
    if matches!(
        metric,
        "measurement_loss_ratio" | "health_failure_ratio" | "healthy_sample_ratio"
    ) {
        parse_percentage_value(value, path)?;
        return Ok(());
    }
    if matches!(metric, "p50_latency" | "p95_latency" | "p99_latency") {
        let duration = parse_duration(value)
            .map_err(|error| ConfigError::validation(path, error.to_string()))?;
        if duration.is_zero() {
            return Err(ConfigError::validation(path, "must be greater than zero"));
        }
        return Ok(());
    }
    let number = value
        .parse::<f64>()
        .map_err(|_| ConfigError::validation(path, "must be a finite number"))?;
    if !number.is_finite() || number < 0.0 {
        return Err(ConfigError::validation(
            path,
            "must be a finite number greater than or equal to zero",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StartupMode {
    #[default]
    ImmediateThenAligned,
    Aligned,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeConcurrencyLimits {
    pub icmp: Option<u32>,
    pub http: Option<u32>,
    pub tcp: Option<u32>,
    pub dns: Option<u32>,
}

fn default_jitter() -> String {
    "10%".to_owned()
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
        let interval = parse_duration(interval_str)
            .map_err(|e| ConfigError::validation(format!("{}.interval", path), e.to_string()))?;
        if interval.is_zero() {
            return Err(ConfigError::validation(
                format!("{}.interval", path),
                "must be greater than zero",
            ));
        }
        let timeout = parse_duration(timeout_str)
            .map_err(|e| ConfigError::validation(format!("{}.timeout", path), e.to_string()))?;
        if timeout.is_zero() {
            return Err(ConfigError::validation(
                format!("{}.timeout", path),
                "must be greater than zero",
            ));
        }
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
        if interval.is_zero() {
            return Err(ConfigError::validation(
                format!("{}.interval", check_path),
                "must be greater than zero",
            ));
        }
        if timeout.is_zero() {
            return Err(ConfigError::validation(
                format!("{}.timeout", check_path),
                "must be greater than zero",
            ));
        }
        let probe_params = match self {
            Self::Icmp(p) => {
                let count = check.count.unwrap_or(p.count);
                if count == 0 {
                    return Err(ConfigError::validation(
                        format!("{}.count", check_path),
                        "must be greater than zero",
                    ));
                }
                ResolvedProbeParams::Icmp(ResolvedIcmpParams {
                    count,
                    address_family: check
                        .address_family
                        .as_ref()
                        .or(p.address_family.as_ref())
                        .cloned()
                        .unwrap_or_else(|| "auto".to_owned()),
                    payload_size: check.payload_size.or(p.payload_size).unwrap_or(56),
                    source_address: check
                        .source_address
                        .as_ref()
                        .or(p.source_address.as_ref())
                        .cloned(),
                })
            }
            Self::Http(p) => {
                let url = check.url.as_deref().unwrap_or(&p.url).to_owned();
                let parsed_url = url::Url::parse(&url).map_err(|error| {
                    ConfigError::validation(format!("{}.url", check_path), error.to_string())
                })?;
                if !matches!(parsed_url.scheme(), "http" | "https") {
                    return Err(ConfigError::validation(
                        format!("{}.url", check_path),
                        "URL scheme must be http or https",
                    ));
                }
                let method = check.method.as_ref().or(p.method.as_ref()).cloned();
                if let Some(method) = &method
                    && method.parse::<http::Method>().is_err()
                {
                    return Err(ConfigError::validation(
                        format!("{}.method", check_path),
                        "invalid HTTP method",
                    ));
                }
                let headers = match (&check.headers, &p.headers) {
                    (Some(override_h), Some(base_h)) => {
                        let mut merged = base_h.clone();
                        merged.extend(override_h.clone());
                        merged
                    }
                    (Some(h), None) | (None, Some(h)) => h.clone(),
                    (None, None) => HashMap::new(),
                };
                let expectation = check
                    .expected_status
                    .as_ref()
                    .or(p.expected_status.as_ref());
                let (expected_status, expected_status_range) = expectation
                    .map(HttpStatusExpectation::resolve)
                    .transpose()?
                    .unwrap_or((None, None));
                let body = check.body.as_ref().or(p.body.as_ref()).cloned();
                let body = body.map(|secret| secret.resolve()).transpose()?;
                let root_certificates = check
                    .root_certificates
                    .as_ref()
                    .or(p.root_certificates.as_ref())
                    .cloned()
                    .unwrap_or_default();
                ResolvedProbeParams::Http(ResolvedHttpParams {
                    url,
                    method,
                    headers,
                    expected_status,
                    expected_status_range,
                    body,
                    follow_redirects: check
                        .follow_redirects
                        .or(p.follow_redirects)
                        .unwrap_or(true),
                    max_redirect_count: check
                        .max_redirect_count
                        .or(p.max_redirect_count)
                        .unwrap_or(10),
                    connection_mode: check
                        .connection_mode
                        .as_ref()
                        .or(p.connection_mode.as_ref())
                        .cloned()
                        .unwrap_or_else(|| "pooled".to_owned()),
                    measure_until: check
                        .measure_until
                        .as_ref()
                        .or(p.measure_until.as_ref())
                        .cloned()
                        .unwrap_or_else(|| "headers".to_owned()),
                    user_agent: check.user_agent.as_ref().or(p.user_agent.as_ref()).cloned(),
                    tls_validate: check.tls_validate.or(p.tls_validate).unwrap_or(true),
                    root_certificates,
                })
            }
            Self::Tcp(p) => {
                let host = check.host.as_deref().unwrap_or(&p.host).to_owned();
                let port = check.port.unwrap_or(p.port);
                if port == 0 {
                    return Err(ConfigError::validation(
                        format!("{}.port", check_path),
                        "must be between 1 and 65535",
                    ));
                }
                ResolvedProbeParams::Tcp(ResolvedTcpParams {
                    host,
                    port,
                    address_family: check
                        .address_family
                        .as_ref()
                        .or(p.address_family.as_ref())
                        .cloned()
                        .unwrap_or_else(|| "auto".to_owned()),
                    source_address: check
                        .source_address
                        .as_ref()
                        .or(p.source_address.as_ref())
                        .cloned(),
                    tls: check.tls.as_ref().or(p.tls.as_ref()).cloned(),
                })
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
                    protocol: check
                        .protocol
                        .as_ref()
                        .or(p.protocol.as_ref())
                        .cloned()
                        .unwrap_or_else(|| "udp".to_owned()),
                    expected_rcode: check
                        .expected_rcode
                        .as_ref()
                        .or(p.expected_rcode.as_ref())
                        .cloned()
                        .unwrap_or_else(|| "noerror".to_owned()),
                    require_answer: check.require_answer.or(p.require_answer).unwrap_or(false),
                })
            }
        };
        validate_resolved_probe_params(&probe_params, &check_path)?;
        let revision_input = serde_json::json!({
            "check_id": check.id.as_str(),
            "profile_id": self.id().as_str(),
            "interval_ns": interval.as_nanos(),
            "timeout_ns": timeout.as_nanos(),
            "probe": probe_params,
        })
        .to_string();
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

fn validate_resolved_probe_params(
    params: &ResolvedProbeParams,
    path: &str,
) -> Result<(), ConfigError> {
    let validate_family = |family: &str, source: Option<&str>| -> Result<(), ConfigError> {
        if !["auto", "ipv4", "ipv6"].contains(&family) {
            return Err(ConfigError::validation(
                format!("{path}.address_family"),
                "must be auto, ipv4, or ipv6",
            ));
        }
        if let Some(source) = source {
            let address = source.parse::<std::net::IpAddr>().map_err(|_| {
                ConfigError::validation(format!("{path}.source_address"), "must be an IP address")
            })?;
            if (family == "ipv4" && !address.is_ipv4()) || (family == "ipv6" && !address.is_ipv6())
            {
                return Err(ConfigError::validation(
                    format!("{path}.source_address"),
                    "source address is incompatible with address_family",
                ));
            }
        }
        Ok(())
    };
    let validate_certificates = |certificates: &[String]| -> Result<(), ConfigError> {
        for certificate in certificates {
            let certificates = rustls::pki_types::CertificateDer::pem_file_iter(certificate)
                .map_err(|error| {
                    ConfigError::validation(
                        format!("{path}.root_certificates"),
                        format!("cannot read {certificate}: {error}"),
                    )
                })?;
            let mut found = false;
            for parsed in certificates {
                parsed.map_err(|error| {
                    ConfigError::validation(
                        format!("{path}.root_certificates"),
                        format!("invalid PEM certificate {certificate}: {error}"),
                    )
                })?;
                found = true;
            }
            if !found {
                return Err(ConfigError::validation(
                    format!("{path}.root_certificates"),
                    format!("{certificate} contains no PEM certificates"),
                ));
            }
        }
        Ok(())
    };
    match params {
        ResolvedProbeParams::Icmp(params) => {
            validate_family(&params.address_family, params.source_address.as_deref())?;
            if params.payload_size > 65_507 {
                return Err(ConfigError::validation(
                    format!("{path}.payload_size"),
                    "must not exceed 65507 bytes",
                ));
            }
        }
        ResolvedProbeParams::Http(params) => {
            if !["pooled", "per_round", "fresh"].contains(&params.connection_mode.as_str()) {
                return Err(ConfigError::validation(
                    format!("{path}.connection_mode"),
                    "must be pooled, per_round, or fresh",
                ));
            }
            if !["headers", "body"].contains(&params.measure_until.as_str()) {
                return Err(ConfigError::validation(
                    format!("{path}.measure_until"),
                    "must be headers or body",
                ));
            }
            validate_certificates(&params.root_certificates)?;
        }
        ResolvedProbeParams::Tcp(params) => {
            validate_family(&params.address_family, params.source_address.as_deref())?;
            if let Some(tls) = &params.tls
                && let Some(certificates) = &tls.root_certificates
            {
                validate_certificates(certificates)?;
            }
        }
        ResolvedProbeParams::Dns(params) => {
            if !["udp", "tcp"].contains(&params.protocol.as_str()) {
                return Err(ConfigError::validation(
                    format!("{path}.protocol"),
                    "must be udp or tcp",
                ));
            }
            if ![
                "noerror", "formerr", "servfail", "nxdomain", "notimp", "refused",
            ]
            .contains(&params.expected_rcode.as_str())
            {
                return Err(ConfigError::validation(
                    format!("{path}.expected_rcode"),
                    "unsupported DNS response code",
                ));
            }
            if let Some(server) = &params.resolver
                && server.parse::<std::net::IpAddr>().is_err()
                && server.parse::<std::net::SocketAddr>().is_err()
            {
                return Err(ConfigError::validation(
                    format!("{path}.server"),
                    "must be an IP address with an optional port",
                ));
            }
        }
    }
    Ok(())
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
    pub address_family: Option<String>,
    pub payload_size: Option<usize>,
    pub source_address: Option<String>,
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
    pub expected_status: Option<HttpStatusExpectation>,
    pub body: Option<SecretRef>,
    pub follow_redirects: Option<bool>,
    pub max_redirect_count: Option<u32>,
    pub connection_mode: Option<String>,
    pub measure_until: Option<String>,
    pub user_agent: Option<String>,
    pub tls_validate: Option<bool>,
    pub root_certificates: Option<Vec<String>>,
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
    pub address_family: Option<String>,
    pub source_address: Option<String>,
    pub tls: Option<TcpTlsConfig>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TcpTlsConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub server_name: Option<String>,
    pub tls_validate: Option<bool>,
    pub root_certificates: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DnsProfileParams {
    pub id: ProfileId,
    #[serde(default = "default_interval")]
    pub interval: String,
    #[serde(default = "default_timeout")]
    pub timeout: String,
    #[serde(rename = "name", alias = "domain")]
    pub domain: String,
    pub record_type: Option<String>,
    #[serde(rename = "server", alias = "resolver")]
    pub resolver: Option<String>,
    pub protocol: Option<String>,
    pub expected_rcode: Option<String>,
    pub require_answer: Option<bool>,
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
    pub minimum_latency_samples: Option<u32>,
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
    pub expected_status: Option<HttpStatusExpectation>,
    #[serde(default)]
    pub body: Option<SecretRef>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default, rename = "name", alias = "domain")]
    pub domain: Option<String>,
    #[serde(default)]
    pub record_type: Option<String>,
    #[serde(default, rename = "server", alias = "resolver")]
    pub resolver: Option<String>,
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub address_family: Option<String>,
    #[serde(default)]
    pub payload_size: Option<usize>,
    #[serde(default)]
    pub source_address: Option<String>,
    #[serde(default)]
    pub follow_redirects: Option<bool>,
    #[serde(default)]
    pub max_redirect_count: Option<u32>,
    #[serde(default)]
    pub connection_mode: Option<String>,
    #[serde(default)]
    pub measure_until: Option<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub tls_validate: Option<bool>,
    #[serde(default)]
    pub root_certificates: Option<Vec<String>>,
    #[serde(default)]
    pub tls: Option<TcpTlsConfig>,
    #[serde(default)]
    pub protocol: Option<String>,
    #[serde(default)]
    pub expected_rcode: Option<String>,
    #[serde(default)]
    pub require_answer: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HttpStatusExpectation {
    Status(u16),
    Range(String),
}

type ResolvedHttpStatus = (Option<u16>, Option<(u16, u16)>);

impl HttpStatusExpectation {
    fn resolve(&self) -> Result<ResolvedHttpStatus, ConfigError> {
        match self {
            Self::Status(status) if (100..=599).contains(status) => Ok((Some(*status), None)),
            Self::Status(_) => Err(ConfigError::validation(
                "expected_status",
                "HTTP status must be between 100 and 599",
            )),
            Self::Range(value) => {
                let (start, end) = value.split_once('-').ok_or_else(|| {
                    ConfigError::validation("expected_status", "status range must use START-END")
                })?;
                let start = start.parse::<u16>().map_err(|_| {
                    ConfigError::validation("expected_status", "invalid status range")
                })?;
                let end = end.parse::<u16>().map_err(|_| {
                    ConfigError::validation("expected_status", "invalid status range")
                })?;
                if !(100..=599).contains(&start) || !(100..=599).contains(&end) || start > end {
                    return Err(ConfigError::validation(
                        "expected_status",
                        "status range must be ordered and between 100 and 599",
                    ));
                }
                Ok((None, Some((start, end))))
            }
        }
    }
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

impl SecretRef {
    pub fn resolve(&self) -> Result<String, ConfigError> {
        match self {
            Self::Literal(value) => Ok(value.clone()),
            Self::FromEnv { from_env } => std::env::var(from_env).map_err(|_| {
                ConfigError::validation(
                    "secret",
                    format!("environment variable is not set: {from_env}"),
                )
            }),
            Self::FromFile { from_file } => std::fs::read_to_string(from_file)
                .map(|value| value.trim_end_matches(['\r', '\n']).to_owned())
                .map_err(ConfigError::Io),
        }
    }
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
    fn alert_validation_matches_runtime_metrics_and_operators() {
        let template = r#"
version: 1
profiles:
  - kind: icmp
    id: p1
notifiers:
  - kind: webhook
    id: n1
    url: http://example.com
rules:
  - id: r1
    profile: p1
    metric: METRIC
    operator: OPERATOR
    threshold: "1"
    window: WINDOW
    notifier: n1
"#;
        for metric in [
            "consecutive_total_loss_rounds",
            "consecutive_unhealthy_rounds",
            "no_data",
        ] {
            let config = KemuriConfig::parse(
                &template
                    .replace("METRIC", metric)
                    .replace("OPERATOR", "gte")
                    .replace("WINDOW", "1m"),
            );
            assert!(config.is_ok(), "{metric} must be accepted");
        }
        for (metric, operator, window) in [
            ("consecutive_unhealthy", "gte", "1m"),
            ("consecutive_unhealthy_rounds", "eq", "1m"),
            ("consecutive_unhealthy_rounds", "gte", "0s"),
        ] {
            let config = KemuriConfig::parse(
                &template
                    .replace("METRIC", metric)
                    .replace("OPERATOR", operator)
                    .replace("WINDOW", window),
            );
            assert!(
                config.is_err(),
                "{metric}/{operator}/{window} must be rejected"
            );
        }
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

    #[test]
    fn resolve_excludes_disabled_targets_and_checks() {
        let yaml = r#"
version: 1
profiles:
  - kind: icmp
    id: ping
targets:
  - id: disabled-target
    address: 127.0.0.1
    enabled: false
    checks:
      - id: ping
        profile: ping
  - id: enabled-target
    address: 127.0.0.1
    checks:
      - id: disabled-check
        profile: ping
        enabled: false
      - id: enabled-check
        profile: ping
"#;
        let config = KemuriConfig::parse(yaml).unwrap();
        let resolved = config.resolve().unwrap();
        assert_eq!(resolved.checks.len(), 1);
        assert_eq!(resolved.checks[0].check_id.as_str(), "enabled-check");
    }

    #[test]
    fn reject_zero_scheduler_values() {
        for yaml in [
            "version: 1\nscheduler:\n  tick_interval: 0s\n",
            "version: 1\nscheduler:\n  max_concurrent: 0\n",
        ] {
            assert!(KemuriConfig::parse(yaml).is_err());
        }
    }

    #[test]
    fn resolve_all_probe_options() {
        let yaml = r#"
version: 1
profiles:
  - kind: icmp
    id: ping
    count: 2
    address_family: ipv4
    payload_size: 32
    source_address: 127.0.0.1
  - kind: http
    id: web
    url: https://example.test/
    method: POST
    body: payload
    expected_status: "200-399"
    follow_redirects: false
    max_redirect_count: 3
    connection_mode: per_round
    measure_until: body
    user_agent: kemuri-test
    tls_validate: false
  - kind: tcp
    id: socket
    host: localhost
    port: 443
    address_family: ipv4
    source_address: 127.0.0.1
    tls:
      enabled: true
      server_name: localhost
  - kind: dns
    id: resolver
    name: example.test
    server: 127.0.0.1:53
    record_type: AAAA
    protocol: tcp
    expected_rcode: nxdomain
    require_answer: true
targets:
  - id: local
    address: 127.0.0.1
    checks:
      - id: ping
        profile: ping
      - id: web
        profile: web
      - id: socket
        profile: socket
      - id: resolver
        profile: resolver
"#;
        let config = KemuriConfig::parse(yaml).unwrap();
        let resolved = config.resolve().unwrap();
        assert_eq!(resolved.checks.len(), 4);
        let http = resolved
            .checks
            .iter()
            .find(|check| check.check_id.as_str() == "web")
            .unwrap();
        let ResolvedProbeParams::Http(http) = &http.probe_params else {
            panic!("expected HTTP parameters");
        };
        assert_eq!(http.expected_status_range, Some((200, 399)));
        assert_eq!(http.measure_until, "body");
    }

    #[test]
    fn dns_check_accepts_name_and_server_overrides() {
        let yaml = r#"
version: 1
profiles:
  - kind: dns
    id: resolver
    name: profile.example
    server: 192.0.2.1
targets:
  - id: local
    address: 127.0.0.1
    checks:
      - id: resolver
        profile: resolver
        name: check.example
        server: 192.0.2.2:5353
"#;
        let config = KemuriConfig::parse(yaml).unwrap();
        let resolved = config.resolve().unwrap();
        let ResolvedProbeParams::Dns(dns) = &resolved.checks[0].probe_params else {
            panic!("expected DNS parameters");
        };
        assert_eq!(dns.domain, "check.example");
        assert_eq!(dns.resolver.as_deref(), Some("192.0.2.2:5353"));
    }

    #[test]
    fn reject_invalid_root_certificate_before_startup() {
        let path =
            std::env::temp_dir().join(format!("kemuri-invalid-certificate-{}", std::process::id()));
        std::fs::write(&path, "not a PEM certificate").unwrap();
        let yaml = format!(
            r#"
version: 1
profiles:
  - kind: http
    id: web
    url: https://example.test
    root_certificates:
      - {}
targets:
  - id: local
    address: 127.0.0.1
    checks:
      - id: web
        profile: web
"#,
            path.display()
        );
        let result = KemuriConfig::parse(&yaml).and_then(|config| config.resolve().map(|_| ()));
        std::fs::remove_file(path).unwrap();
        assert!(result.is_err());
    }
}
