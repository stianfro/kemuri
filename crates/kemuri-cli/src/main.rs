use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "kemuri", version, about = "Latency monitoring system")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Serve {
        #[arg(short, long)]
        config: PathBuf,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    Database {
        #[command(subcommand)]
        command: DatabaseCommands,
    },
    Doctor {
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        test_notifiers: bool,
    },
    Notify {
        #[command(subcommand)]
        command: NotifyCommands,
    },
    Check {
        #[arg(short, long)]
        config: PathBuf,
        target_check: String,
    },
    Version,
}

#[derive(Subcommand)]
enum ConfigCommands {
    Validate {
        #[arg(short, long)]
        config: PathBuf,
    },
}

#[derive(Subcommand)]
enum DatabaseCommands {
    Backup {
        #[arg(short, long)]
        config: Option<PathBuf>,
        #[arg(short, long)]
        output: String,
    },
}

#[derive(Subcommand)]
enum NotifyCommands {
    Test {
        #[arg(short, long)]
        config: PathBuf,
        notifier_id: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Serve { config } => {
            let config_path_clone = config.clone();
            let config = kemuri_config::KemuriConfig::load(&config)?;
            let build_info = kemuri_core::BuildInfo {
                version: env!("CARGO_PKG_VERSION").to_owned(),
                git_hash: option_env!("GIT_HASH").unwrap_or("unknown").to_owned(),
                build_timestamp_ms: option_env!("BUILD_TIMESTAMP")
                    .and_then(|value| value.parse::<i64>().ok())
                    .unwrap_or_default()
                    * 1_000,
                target: option_env!("BUILD_TARGET").unwrap_or("unknown").to_owned(),
            };
            init_tracing(&config);
            kemuri_server::serve(config, build_info, config_path_clone).await?;
        }
        Commands::Config { command } => match command {
            ConfigCommands::Validate { config } => {
                let yaml = std::fs::read_to_string(&config)?;
                let raw: kemuri_config::KemuriConfig = serde_yaml::from_str(&yaml)?;
                let warnings = raw.validate_with_warnings()?;
                if warnings.is_empty() {
                    println!("Configuration is valid.");
                } else {
                    println!("Configuration is valid with warnings:");
                    for w in &warnings {
                        println!("  [WARN] {}: {}", w.path, w.message);
                    }
                }
            }
        },
        Commands::Database { command } => match command {
            DatabaseCommands::Backup { config, output } => {
                run_database_backup(config.as_deref(), &output).await?;
            }
        },
        Commands::Doctor {
            config,
            test_notifiers,
        } => {
            run_doctor(config.as_deref(), test_notifiers).await?;
        }
        Commands::Notify { command } => match command {
            NotifyCommands::Test {
                config,
                notifier_id,
            } => {
                run_notify_test(&config, &notifier_id).await?;
            }
        },
        Commands::Version => {
            println!("kemuri {}", env!("CARGO_PKG_VERSION"));
            println!("git: {}", option_env!("GIT_HASH").unwrap_or("unknown"));
            println!(
                "built: {}",
                option_env!("BUILD_TIMESTAMP").unwrap_or("unknown")
            );
            println!(
                "target: {}",
                option_env!("BUILD_TARGET").unwrap_or("unknown")
            );
        }
        Commands::Check {
            config,
            target_check,
        } => {
            run_check(&config, &target_check).await?;
        }
    }

    Ok(())
}

async fn run_database_backup(config_path: Option<&std::path::Path>, output: &str) -> Result<()> {
    let db_path = if let Some(path) = config_path {
        let config = kemuri_config::KemuriConfig::load(path)?;
        config.storage.path.clone()
    } else {
        "kemuri.db".to_owned()
    };

    let options =
        sqlx::sqlite::SqliteConnectOptions::from_str(&format!("sqlite://{}?mode=ro", db_path))?
            .foreign_keys(true);

    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;

    if output == "-" {
        let temp =
            std::env::temp_dir().join(format!("kemuri-backup-{}.sqlite", std::process::id()));
        sqlx::query(&format!(
            "VACUUM INTO '{}'",
            temp.display().to_string().replace('\'', "''")
        ))
        .execute(&pool)
        .await?;
        let image = std::fs::read(&temp)?;
        std::fs::remove_file(&temp).ok();
        use std::io::Write;
        std::io::stdout().lock().write_all(&image)?;
    } else {
        sqlx::query(&format!("VACUUM INTO '{}'", output.replace('\'', "''")))
            .execute(&pool)
            .await?;
        println!("Backup written to {}", output);
    }

    pool.close().await;
    Ok(())
}

use std::str::FromStr;

async fn run_doctor(config_path: Option<&std::path::Path>, test_notifiers: bool) -> Result<()> {
    let mut any_fail = false;

    println!("Kemuri Doctor");
    println!("=============");

    let config = match config_path {
        Some(path) => match kemuri_config::KemuriConfig::load(path) {
            Ok(c) => {
                println!("[PASS] Configuration readability and validity");
                Some(c)
            }
            Err(e) => {
                println!("[FAIL] Configuration readability and validity: {}", e);
                any_fail = true;
                None
            }
        },
        None => {
            println!("[WARN] No configuration path specified");
            None
        }
    };

    if let Some(ref cfg) = config {
        let data_dir = std::path::Path::new(&cfg.storage.path)
            .parent()
            .unwrap_or(std::path::Path::new("."));
        if data_dir.exists() {
            let test_file = data_dir.join(".kemuri_doctor_test");
            match std::fs::write(&test_file, b"test") {
                Ok(()) => {
                    std::fs::remove_file(&test_file).ok();
                    match std::fs::read_dir(data_dir) {
                        Ok(_) => println!("[PASS] Data-directory permissions (read/write)"),
                        Err(e) => {
                            println!("[FAIL] Data-directory read access: {}", e);
                            any_fail = true;
                        }
                    }
                }
                Err(e) => {
                    println!("[FAIL] Data-directory write access: {}", e);
                    any_fail = true;
                }
            }
        } else {
            println!(
                "[WARN] Data directory does not exist: {}",
                data_dir.display()
            );
        }

        let db_path = &cfg.storage.path;
        let db_result = async {
            let options = sqlx::sqlite::SqliteConnectOptions::from_str(&format!(
                "sqlite://{}?mode=ro",
                db_path
            ))?
            .foreign_keys(true);
            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(options)
                .await?;
            let row: (String,) = sqlx::query_as("PRAGMA integrity_check")
                .fetch_one(&pool)
                .await?;
            pool.close().await;
            Ok::<String, anyhow::Error>(row.0)
        }
        .await;
        match db_result {
            Ok(result) if result == "ok" => println!("[PASS] Database integrity check"),
            Ok(result) => {
                println!("[WARN] Database integrity check: {}", result);
            }
            Err(e) => {
                println!("[FAIL] Database open/integrity: {}", e);
                any_fail = true;
            }
        }

        let icmp_cap = kemuri_probes::check_icmp_capability();
        if icmp_cap.is_available() {
            println!("[PASS] ICMP permission availability");
        } else {
            println!("[WARN] ICMP permission not available (ICMP checks will not work)");
        }

        let bind_addr = format!("{}:{}", cfg.server.bind, cfg.server.port);
        match std::net::TcpListener::bind(&bind_addr) {
            Ok(_) => println!("[PASS] Listening-address availability ({})", bind_addr),
            Err(e) => {
                println!(
                    "[FAIL] Listening-address availability ({}): {}",
                    bind_addr, e
                );
                any_fail = true;
            }
        }

        for notifier_cfg in &cfg.notifiers {
            match notifier_cfg {
                kemuri_config::NotifierConfig::Webhook(params) => {
                    if let kemuri_config::SecretRef::FromFile { from_file } = &params.url {
                        match std::fs::read_to_string(from_file) {
                            Ok(_) => println!("[PASS] Secret file readable: {}", from_file),
                            Err(e) => {
                                println!("[FAIL] Secret file not readable: {}: {}", from_file, e);
                                any_fail = true;
                            }
                        }
                    }
                }
                kemuri_config::NotifierConfig::Smtp(params) => {
                    if let Some(kemuri_config::SecretRef::FromFile { from_file }) = &params.password
                    {
                        match std::fs::read_to_string(from_file) {
                            Ok(_) => println!("[PASS] Secret file readable: {}", from_file),
                            Err(e) => {
                                println!("[FAIL] Secret file not readable: {}: {}", from_file, e);
                                any_fail = true;
                            }
                        }
                    }
                }
            }
        }

        if test_notifiers {
            for notifier_cfg in &cfg.notifiers {
                let notifier_id = notifier_cfg.id().to_string();
                let notifier: Result<Box<dyn kemuri_server::Notifier>, _> = match notifier_cfg {
                    kemuri_config::NotifierConfig::Webhook(params) => {
                        kemuri_server::WebhookNotifier::from_config(params)
                            .map(|value| Box::new(value) as Box<dyn kemuri_server::Notifier>)
                    }
                    kemuri_config::NotifierConfig::Smtp(params) => {
                        kemuri_server::SmtpNotifier::from_config(params)
                            .map(|value| Box::new(value) as Box<dyn kemuri_server::Notifier>)
                    }
                };
                match notifier {
                    Ok(notifier) => match notifier.send(make_test_notification(&notifier_id)).await
                    {
                        Ok(()) => println!("[PASS] Notifier connectivity: {}", notifier_id),
                        Err(e) => {
                            println!("[FAIL] Notifier connectivity: {}: {}", notifier_id, e);
                            any_fail = true;
                        }
                    },
                    Err(e) => {
                        println!("[FAIL] Notifier configuration: {}: {}", notifier_id, e);
                        any_fail = true;
                    }
                }
            }
        }
    }

    println!("=============");
    if any_fail {
        println!("Result: FAIL");
        std::process::exit(1);
    } else {
        println!("Result: PASS");
    }

    Ok(())
}

fn make_test_notification(notifier_id: &str) -> kemuri_server::NotificationPayload {
    use kemuri_core::{AlertEventKind, CheckId, ObserverId, ProbeKind, RuleId, TargetId};
    use std::collections::HashMap;

    let now = chrono::Utc::now();
    kemuri_server::NotificationPayload {
        event_id: "test".to_owned(),
        event_type: AlertEventKind::Firing,
        rule_id: RuleId::new("test-rule").unwrap(),
        target_id: TargetId::new("test-target").unwrap(),
        target_name: "Test Target".to_owned(),
        check_id: CheckId::new("test-check").unwrap(),
        observer_id: ObserverId::new("local").unwrap(),
        probe_type: ProbeKind::Http,
        current_value: 1.0,
        threshold: 0.5,
        state_start_time: now,
        event_time: now,
        kemuri_url: None,
        labels: HashMap::new(),
        summary: format!(
            "Test notification from kemuri doctor for notifier {}",
            notifier_id
        ),
    }
}

async fn run_notify_test(config_path: &std::path::Path, notifier_id: &str) -> Result<()> {
    let config = kemuri_config::KemuriConfig::load(config_path)?;
    let notifier_id_parsed = kemuri_core::NotifierId::new(notifier_id)?;

    let notifier_cfg = config
        .notifiers
        .iter()
        .find(|n| n.id() == &notifier_id_parsed)
        .ok_or_else(|| anyhow::anyhow!("notifier not found: {}", notifier_id))?;

    let notifier: Box<dyn kemuri_server::Notifier> = match notifier_cfg {
        kemuri_config::NotifierConfig::Webhook(params) => {
            Box::new(kemuri_server::WebhookNotifier::from_config(params)?)
        }
        kemuri_config::NotifierConfig::Smtp(params) => {
            Box::new(kemuri_server::SmtpNotifier::from_config(params)?)
        }
    };

    let payload = make_test_notification(notifier_id);

    match notifier.send(payload).await {
        Ok(()) => {
            println!("Test notification sent successfully to {}", notifier_id);
        }
        Err(e) => {
            eprintln!("Test notification failed for {}: {}", notifier_id, e);
            std::process::exit(1);
        }
    }

    Ok(())
}

async fn run_check(config_path: &std::path::Path, target_check: &str) -> Result<()> {
    let config = kemuri_config::KemuriConfig::load(config_path)?;
    let resolved = config.resolve()?;

    let parts: Vec<&str> = target_check.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("invalid target/check format, expected <target_id>/<check_id>");
    }
    let target_id_str = parts[0];
    let check_id_str = parts[1];

    let target_id = kemuri_core::TargetId::new(target_id_str)?;
    let check_id = kemuri_core::CheckId::new(check_id_str)?;

    let check_def = resolved
        .checks
        .iter()
        .find(|c| c.target_id == target_id && c.check_id == check_id)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "check {}/{} not found in configuration",
                target_id_str,
                check_id_str
            )
        })?;

    let mut registry = kemuri_server::ProbeRegistry::new();

    match check_def.probe_kind {
        kemuri_core::ProbeKind::Http => {
            let http_config = kemuri_probes::HttpProbeConfig::default();
            if let Ok(probe) = kemuri_probes::HttpProbe::new(http_config) {
                registry.register(std::sync::Arc::new(probe));
            }
        }
        kemuri_core::ProbeKind::Icmp => {
            let icmp_cap = kemuri_probes::check_icmp_capability();
            if icmp_cap.is_available() {
                let icmp_config = kemuri_probes::IcmpProbeConfig::default();
                registry.register(std::sync::Arc::new(kemuri_probes::IcmpProbe::new(
                    icmp_config,
                )));
            } else {
                anyhow::bail!("ICMP capability not available for this process");
            }
        }
        kemuri_core::ProbeKind::Tcp => {
            let tcp_config = kemuri_probes::TcpProbeConfig::default();
            registry.register(std::sync::Arc::new(kemuri_probes::TcpProbe::new(
                tcp_config,
            )));
        }
        kemuri_core::ProbeKind::Dns => {
            let dns_config = kemuri_probes::DnsProbeConfig::default();
            registry.register(std::sync::Arc::new(kemuri_probes::DnsProbe::new(
                dns_config,
            )));
        }
    }

    let probe = registry.get(check_def.probe_kind).ok_or_else(|| {
        anyhow::anyhow!("no probe registered for kind {:?}", check_def.probe_kind)
    })?;

    let context = kemuri_probes::RoundContext {
        observer_id: kemuri_core::ObserverId::new("local")?,
        scheduled_at: std::time::Duration::ZERO,
        deadline: check_def.timeout,
    };

    let sample_count = match &check_def.probe_params {
        kemuri_config::ResolvedProbeParams::Icmp(p) => p.count,
        _ => 1,
    };

    let resolved_check = kemuri_probes::ResolvedCheck {
        check_id: check_def.check_id.clone(),
        target_id: check_def.target_id.clone(),
        profile_id: check_def.profile_id.clone(),
        address: check_def.target_address.clone(),
        probe_kind: check_def.probe_kind,
        timeout: check_def.timeout,
        sample_count,
        settings: check_def.probe_params.clone().into(),
    };

    let start = std::time::Instant::now();
    let result = match tokio::time::timeout(
        check_def.timeout,
        probe.execute_round(context, resolved_check),
    )
    .await
    {
        Ok(Ok(round)) => round,
        Ok(Err(e)) => {
            println!("Probe execution failed: {}", e);
            std::process::exit(1);
        }
        Err(_) => {
            println!("Probe execution timed out after {:?}", check_def.timeout);
            std::process::exit(1);
        }
    };
    let elapsed = start.elapsed();

    println!("Check: {}/{}", target_id_str, check_id_str);
    println!("Probe: {:?}", check_def.probe_kind);
    println!("Address: {}", check_def.target_address);
    println!("Timeout: {:?}", check_def.timeout);
    println!("Elapsed: {:?}", elapsed);
    println!("Samples: {}", result.results.len());
    println!();

    for (i, sample) in result.results.iter().enumerate() {
        let latency_str = sample
            .latency
            .map(|d| format!("{:.1}ms", d.as_secs_f64() * 1000.0))
            .unwrap_or_else(|| "-".to_owned());
        println!(
            "  sample {}: outcome={:?} latency={}",
            i, sample.outcome, latency_str
        );
        if let Some(ref detail) = sample.detail {
            println!("           detail: {}", detail);
        }
        if let Some(ref meta) = sample.metadata {
            for (k, v) in meta {
                println!("           {}: {}", k, v);
            }
        }
    }

    if result
        .results
        .iter()
        .any(|sample| sample.outcome != kemuri_core::SampleOutcome::Success)
    {
        anyhow::bail!("check is unhealthy");
    }

    Ok(())
}

fn init_tracing(config: &kemuri_config::KemuriConfig) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&config.logging.level));
    match config.logging.format {
        kemuri_config::LogFormat::Json => {
            tracing_subscriber::fmt()
                .json()
                .with_env_filter(filter)
                .init();
        }
        kemuri_config::LogFormat::Plain => {
            tracing_subscriber::fmt().with_env_filter(filter).init();
        }
    }
}
