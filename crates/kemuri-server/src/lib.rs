mod alerts;
mod api;
mod events;
mod notification;
mod notifiers;
mod probe_registry;
mod retention_worker;
mod rollup_worker;
mod scheduler;
mod supervisor;
mod worker;
mod writer;

use std::collections::HashMap;
use std::future::IntoFuture;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use axum::Json;
use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use futures::stream::Stream;
use kemuri_config::KemuriConfig;
use kemuri_core::BuildInfo;
use kemuri_core::NotifierId;
use kemuri_probes::{
    DnsProbe, DnsProbeConfig, HttpProbe, HttpProbeConfig, IcmpProbe, IcmpProbeConfig,
    SyntheticProbe, TcpProbe, TcpProbeConfig, check_icmp_capability,
};
use kemuri_storage::StorageManager;
use metrics_exporter_prometheus::PrometheusBuilder;
use metrics_exporter_prometheus::PrometheusHandle;
use rust_embed::RustEmbed;
use tokio::sync::mpsc;
use utoipa::OpenApi;

pub use alerts::AlertEvaluator;
pub use api::{
    AlertEventResponse, AlertEventsResponse, AlertStateResponse, AlertsListResponse, ApiError,
    CheckDetail, CheckSummary, GroupResponse, RoundSummary, SeriesPoint, SeriesResponse,
    TargetDetail, TargetSummary,
};
pub use events::SystemEvent;
pub use notification::NotificationWorker;
pub use notifiers::{NotificationPayload, Notifier, SmtpNotifier, WebhookNotifier};
pub use probe_registry::ProbeRegistry;
pub use scheduler::Scheduler;
pub use supervisor::Supervisor;
pub use worker::{RoundJob, WorkerPool};
pub use writer::{RoundResult, StorageWriter};

#[derive(Clone)]
pub struct AppState {
    pub build_info: Arc<BuildInfo>,
    pub started_at: Instant,
    pub config: Arc<std::sync::RwLock<Arc<KemuriConfig>>>,
    pub prometheus_handle: PrometheusHandle,
    pub pool: sqlx::SqlitePool,
    pub observer_internal_id: i64,
    pub event_tx: tokio::sync::broadcast::Sender<SystemEvent>,
    pub config_path: Arc<PathBuf>,
    pub last_reload: Arc<std::sync::RwLock<Option<ReloadStatus>>>,
    pub reload_tx: mpsc::Sender<()>,
    pub disk_ready: Arc<AtomicBool>,
    pub shutdown_tx: tokio::sync::broadcast::Sender<()>,
    pub runtime_ready: Arc<AtomicBool>,
    pub probe_ready: Arc<AtomicBool>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ReloadStatus {
    pub generation: String,
    pub result: String,
    pub error: Option<String>,
    pub timestamp_ms: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    #[error("metrics: {0}")]
    Metrics(String),
    #[error("bind: {0}")]
    Bind(String),
    #[error("serve: {0}")]
    Serve(String),
    #[error("storage: {0}")]
    Storage(String),
}

pub async fn serve(
    config: KemuriConfig,
    build_info: BuildInfo,
    config_path: PathBuf,
) -> Result<(), ServerError> {
    let handle = PrometheusBuilder::new()
        .install_recorder()
        .map_err(|e| ServerError::Metrics(e.to_string()))?;

    let storage = StorageManager::open(&config.storage.path)
        .await
        .map_err(|e| ServerError::Storage(e.to_string()))?;

    let pool = storage.pool().clone();

    kemuri_storage::reconcile_with_event(&pool, &config, "startup")
        .await
        .map_err(|e| ServerError::Storage(e.to_string()))?;

    let observer_internal_id = ensure_observer(&pool).await?;

    let resolved = config
        .resolve()
        .map_err(|e| ServerError::Storage(e.to_string()))?;

    let mut registry = ProbeRegistry::new();
    registry.register(Arc::new(SyntheticProbe::success(
        std::time::Duration::from_millis(10),
    )));

    let http_config = HttpProbeConfig::default();
    if let Ok(http_probe) = HttpProbe::new(http_config) {
        registry.register(Arc::new(http_probe));
    }

    let icmp_cap = check_icmp_capability();
    let probe_ready = Arc::new(AtomicBool::new(
        icmp_cap.is_available()
            || !resolved
                .checks
                .iter()
                .any(|check| check.probe_kind == kemuri_core::ProbeKind::Icmp),
    ));
    if icmp_cap.is_available() {
        let icmp_config = IcmpProbeConfig::default();
        registry.register(Arc::new(IcmpProbe::new(icmp_config)));
    } else if resolved
        .checks
        .iter()
        .any(|c| c.probe_kind == kemuri_core::ProbeKind::Icmp)
    {
        tracing::warn!(
            "ICMP checks configured but ICMP capability not available. \
                 Ensure the process has permission to create ICMP sockets \
                 (e.g., add to the ping group or set CAP_NET_RAW)"
        );
    }

    let tcp_config = TcpProbeConfig::default();
    registry.register(Arc::new(TcpProbe::new(tcp_config)));

    let dns_config = DnsProbeConfig::default();
    registry.register(Arc::new(DnsProbe::new(dns_config)));

    let registry = Arc::new(registry);

    let (job_tx, job_rx) = mpsc::channel::<worker::RoundJob>(256);
    let (result_tx, result_rx) = mpsc::channel::<writer::RoundResult>(256);
    let (alert_tx, alert_rx) = mpsc::channel::<alerts::AlertNotification>(256);
    let (event_tx, _) = tokio::sync::broadcast::channel::<SystemEvent>(256);
    let (fatal_tx, mut fatal_rx) = mpsc::channel::<&'static str>(16);

    let clock = Arc::new(kemuri_core::RealClock);

    let shared_config = Arc::new(std::sync::RwLock::new(Arc::new(config.clone())));

    let mut scheduler_inst = Scheduler::new(
        config.scheduler.clone(),
        resolved.checks.clone(),
        job_tx,
        result_tx.clone(),
        clock.clone(),
    )
    .with_fatal_channel(fatal_tx.clone());

    let running_rounds = scheduler_inst.running_rounds();

    let worker_pool = WorkerPool::new(registry.clone(), config.scheduler.max_concurrent as usize)
        .with_fatal_channel(fatal_tx.clone());
    let mut worker_handles = worker_pool.start(job_rx, result_tx, running_rounds);

    let writer_storage = Arc::new(storage);
    let writer = StorageWriter::new(writer_storage.clone(), observer_internal_id)
        .with_alert_channel(alert_tx)
        .with_event_channel(event_tx.clone());
    let fatal_writer = fatal_tx.clone();
    let mut writer_handle = tokio::spawn(async move {
        writer.run(result_rx).await;
        let _ = fatal_writer.send("storage_writer").await;
    });

    let alert_evaluator = AlertEvaluator::new(
        Arc::new(writer_storage.pool().clone()),
        shared_config.clone(),
        clock.clone(),
        observer_internal_id,
    )
    .with_event_channel(event_tx.clone());
    let fatal_alert = fatal_tx.clone();
    let _alert_handle = tokio::spawn(async move {
        alert_evaluator.run(alert_rx).await;
        let _ = fatal_alert.send("alert_evaluator").await;
    });

    let notifier_map =
        build_notifiers(&config).map_err(|error| ServerError::Storage(error.to_string()))?;
    let shared_notifiers = Arc::new(std::sync::RwLock::new(notifier_map));

    let (worker_shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);

    let notification_worker = NotificationWorker::new(
        Arc::new(writer_storage.pool().clone()),
        shared_notifiers.clone(),
        clock.clone(),
        config.server.public_url.clone(),
    );
    let notification_shutdown = worker_shutdown_tx.subscribe();
    let fatal_notification = fatal_tx.clone();
    let _notification_handle = tokio::spawn(async move {
        notification_worker.run(notification_shutdown).await;
        let _ = fatal_notification.send("notification_worker").await;
    });

    let no_data_evaluator = AlertEvaluator::new(
        Arc::new(writer_storage.pool().clone()),
        shared_config.clone(),
        clock.clone(),
        observer_internal_id,
    )
    .with_event_channel(event_tx.clone());
    let mut no_data_shutdown = worker_shutdown_tx.subscribe();
    let fatal_no_data = fatal_tx.clone();
    let _no_data_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    no_data_evaluator.run_no_data_check().await;
                }
                _ = no_data_shutdown.recv() => {
                    let _ = fatal_no_data.send("no_data_evaluator").await;
                    return;
                }
            }
        }
    });

    let rollup_pool = pool.clone();
    let rollup_shutdown = worker_shutdown_tx.subscribe();
    let rollup_worker = rollup_worker::RollupWorker::new(Arc::new(rollup_pool));
    let fatal_rollup = fatal_tx.clone();
    let _rollup_handle = tokio::spawn(async move {
        rollup_worker.run(rollup_shutdown).await;
        let _ = fatal_rollup.send("rollup_worker").await;
    });

    let retention_pool = pool.clone();
    let retention_shutdown = worker_shutdown_tx.subscribe();
    let retention_config = config.storage.retention.clone();
    let retention_worker =
        retention_worker::RetentionWorker::new(Arc::new(retention_pool), retention_config);
    let fatal_retention = fatal_tx.clone();
    let _retention_handle = tokio::spawn(async move {
        retention_worker.run(retention_shutdown).await;
        let _ = fatal_retention.send("retention_worker").await;
    });

    scheduler_inst.start();
    let disk_ready = Arc::new(AtomicBool::new(true));
    let mut disk_shutdown = worker_shutdown_tx.subscribe();
    let disk_config = shared_config.clone();
    let disk_scheduler = scheduler_inst.command_sender();
    let disk_ready_worker = disk_ready.clone();
    let _disk_handle = tokio::spawn(async move {
        let mut paused = false;
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    let config = disk_config.read().unwrap().clone();
                    let database = std::path::Path::new(&config.storage.path);
                    let directory = database.parent().unwrap_or_else(|| std::path::Path::new("."));
                    if let (Ok(available), Ok(total)) =
                        (fs2::available_space(directory), fs2::total_space(directory))
                    {
                        let free_ratio = if total == 0 {
                            0.0
                        } else {
                            available as f64 / total as f64
                        };
                        metrics::gauge!("kemuri_disk_free_bytes").set(available as f64);
                        metrics::gauge!("kemuri_disk_free_ratio").set(free_ratio);
                        let critical = percentage_ratio(
                            &config.storage.disk_pressure.critical_free,
                            0.05,
                        );
                        let warning = percentage_ratio(
                            &config.storage.disk_pressure.warning_free,
                            0.10,
                        );
                        let should_pause = if paused {
                            free_ratio <= warning
                        } else {
                            free_ratio <= critical
                        };
                        if should_pause != paused {
                            paused = should_pause;
                            disk_ready_worker.store(!paused, Ordering::Release);
                            if let Some(sender) = &disk_scheduler {
                                let _ = sender.send(scheduler::SchedulerCommand::Pause(paused)).await;
                            }
                            if paused {
                                tracing::error!(
                                    free_ratio,
                                    "scheduling paused because disk free space is critical"
                                );
                            } else {
                                tracing::info!(
                                    free_ratio,
                                    "scheduling resumed after disk free space recovered"
                                );
                            }
                        }
                    } else {
                        disk_ready_worker.store(false, Ordering::Release);
                    }
                }
                _ = disk_shutdown.recv() => return,
            }
        }
    });

    metrics::counter!("kemuri_build_info",
        "version" => build_info.version.clone(),
        "git_hash" => build_info.git_hash.clone())
    .increment(1);
    metrics::gauge!("kemuri_process_start_time_seconds").set(chrono::Utc::now().timestamp() as f64);

    let last_reload = Arc::new(std::sync::RwLock::new(None::<ReloadStatus>));
    let (reload_tx, mut reload_rx) = tokio::sync::mpsc::channel::<()>(1);
    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(4);
    let runtime_ready = Arc::new(AtomicBool::new(true));

    let state = AppState {
        build_info: Arc::new(build_info),
        started_at: Instant::now(),
        config: shared_config.clone(),
        prometheus_handle: handle,
        pool: pool.clone(),
        observer_internal_id,
        event_tx: event_tx.clone(),
        config_path: Arc::new(config_path),
        last_reload: last_reload.clone(),
        reload_tx: reload_tx.clone(),
        disk_ready,
        shutdown_tx: shutdown_tx.clone(),
        runtime_ready: runtime_ready.clone(),
        probe_ready: probe_ready.clone(),
    };

    let config_path_arc = state.config_path.clone();
    let app = create_router(state);

    let addr: std::net::SocketAddr = format!("{}:{}", config.server.bind, config.server.port)
        .parse()
        .map_err(|e: std::net::AddrParseError| ServerError::Bind(e.to_string()))?;

    tracing::info!("listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| ServerError::Bind(e.to_string()))?;

    let shutdown_tx_sigint = shutdown_tx.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        let _ = shutdown_tx_sigint.send(());
    });

    #[cfg(unix)]
    {
        let mut sigterm_rx =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .map_err(|e| ServerError::Serve(e.to_string()))?;
        let mut sighup_rx = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
            .map_err(|e| ServerError::Serve(e.to_string()))?;
        let shutdown_tx_sigterm = shutdown_tx.clone();
        tokio::spawn(async move {
            let _ = sigterm_rx.recv().await;
            let _ = shutdown_tx_sigterm.send(());
        });
        tokio::spawn(async move {
            loop {
                let _ = sighup_rx.recv().await;
                let _ = reload_tx.send(()).await;
            }
        });
    }

    let shutdown_timeout = kemuri_core::parse_duration(&config.server.shutdown_timeout)
        .unwrap_or(std::time::Duration::from_secs(30));

    let mut shutdown_rx1 = shutdown_tx.subscribe();
    let mut shutdown_rx_main = shutdown_tx.subscribe();
    let server = axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx1.recv().await;
            tracing::info!("shutdown signal received, draining connections");
        })
        .into_future();
    tokio::pin!(server);

    let result = loop {
        tokio::select! {
            result = &mut server => break result,
            _ = shutdown_rx_main.recv() => {
                runtime_ready.store(false, Ordering::Release);
                break Ok(());
            },
            Some(component) = fatal_rx.recv() => {
                runtime_ready.store(false, Ordering::Release);
                tracing::error!(component, "required runtime task exited unexpectedly");
                break Err(std::io::Error::other(format!(
                    "required runtime task exited: {component}"
                )));
            }
            Some(()) = reload_rx.recv() => {
            perform_reload(
                &shared_config,
                &shared_notifiers,
                &mut scheduler_inst,
                &event_tx,
                &last_reload,
                &pool,
                observer_internal_id,
                &config_path_arc,
                &probe_ready,
            ).await;
            }
        }
    };

    let serve_error = result.err().map(|error| error.to_string());

    tracing::info!("stopping scheduler and workers");
    scheduler_inst.stop().await;

    let _ = worker_shutdown_tx.send(());

    let graceful = async {
        let server_result = (&mut server).await;
        for worker in &mut worker_handles {
            let _ = worker.await;
        }
        let _ = (&mut writer_handle).await;
        server_result
    };
    match tokio::time::timeout(shutdown_timeout, graceful).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            tracing::warn!(%error, "HTTP server failed during shutdown");
        }
        Err(_) => {
            tracing::warn!("graceful shutdown deadline exceeded");
            for worker in &worker_handles {
                worker.abort();
            }
            writer_handle.abort();
        }
    }

    tracing::info!("kemuri server stopped");
    if let Some(error) = serve_error {
        Err(ServerError::Serve(error))
    } else {
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
async fn perform_reload(
    shared_config: &Arc<std::sync::RwLock<Arc<KemuriConfig>>>,
    shared_notifiers: &Arc<std::sync::RwLock<HashMap<NotifierId, Arc<dyn Notifier>>>>,
    scheduler: &mut Scheduler,
    event_tx: &tokio::sync::broadcast::Sender<SystemEvent>,
    last_reload: &Arc<std::sync::RwLock<Option<ReloadStatus>>>,
    pool: &sqlx::SqlitePool,
    _observer_internal_id: i64,
    config_path: &std::path::Path,
    probe_ready: &Arc<AtomicBool>,
) {
    tracing::info!("SIGHUP received, attempting configuration reload");

    let new_config = match KemuriConfig::load(config_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!(error = %e, "config reload failed: could not read or parse config");
            metrics::counter!("kemuri_config_reload_total", "result" => "failure").increment(1);
            let status = ReloadStatus {
                generation: String::new(),
                result: "failure".to_owned(),
                error: Some(e.to_string()),
                timestamp_ms: chrono::Utc::now().timestamp_millis(),
            };
            *last_reload.write().unwrap() = Some(status);
            return;
        }
    };

    let new_resolved = match new_config.resolve() {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "config reload failed: resolution error");
            metrics::counter!("kemuri_config_reload_total", "result" => "failure").increment(1);
            let status = ReloadStatus {
                generation: String::new(),
                result: "failure".to_owned(),
                error: Some(e.to_string()),
                timestamp_ms: chrono::Utc::now().timestamp_millis(),
            };
            *last_reload.write().unwrap() = Some(status);
            return;
        }
    };

    let new_generation = new_resolved.generation.to_string();
    let notifier_map = match build_notifiers(&new_config) {
        Ok(notifiers) => notifiers,
        Err(error) => {
            tracing::error!(%error, "config reload failed: notifier initialization error");
            metrics::counter!("kemuri_config_reload_total", "result" => "failure").increment(1);
            *last_reload.write().unwrap() = Some(ReloadStatus {
                generation: new_generation,
                result: "failure".to_owned(),
                error: Some("notifier initialization failed".to_owned()),
                timestamp_ms: chrono::Utc::now().timestamp_millis(),
            });
            return;
        }
    };

    if let Err(e) = kemuri_storage::reconcile_with_event(pool, &new_config, "reload").await {
        tracing::error!(error = %e, "config reload failed: database reconciliation error");
        metrics::counter!("kemuri_config_reload_total", "result" => "failure").increment(1);
        let status = ReloadStatus {
            generation: new_generation,
            result: "failure".to_owned(),
            error: Some(e.to_string()),
            timestamp_ms: chrono::Utc::now().timestamp_millis(),
        };
        *last_reload.write().unwrap() = Some(status);
        return;
    }
    if let Err(error) = resolve_removed_alerts(pool, &new_config).await {
        tracing::error!(%error, "config reload failed: alert reconciliation error");
        return;
    }

    scheduler.reconcile(new_resolved.checks.clone()).await;
    let icmp_available = check_icmp_capability().is_available();
    probe_ready.store(
        icmp_available
            || !new_resolved
                .checks
                .iter()
                .any(|check| check.probe_kind == kemuri_core::ProbeKind::Icmp),
        Ordering::Release,
    );

    {
        *shared_notifiers.write().unwrap() = notifier_map;
    }

    {
        *shared_config.write().unwrap() = Arc::new(new_config);
    }

    let _ = event_tx.send(SystemEvent::config_reloaded(&new_generation, "success"));

    metrics::counter!("kemuri_config_reload_total", "result" => "success").increment(1);

    let status = ReloadStatus {
        generation: new_generation,
        result: "success".to_owned(),
        error: None,
        timestamp_ms: chrono::Utc::now().timestamp_millis(),
    };
    *last_reload.write().unwrap() = Some(status);

    tracing::info!("configuration reload completed successfully");
}

fn build_notifiers(
    config: &KemuriConfig,
) -> Result<HashMap<NotifierId, Arc<dyn Notifier>>, String> {
    let mut notifiers: HashMap<NotifierId, Arc<dyn Notifier>> = HashMap::new();
    for notifier in &config.notifiers {
        match notifier {
            kemuri_config::NotifierConfig::Webhook(params) => {
                let value = WebhookNotifier::from_config(params)
                    .map_err(|error| format!("notifier {}: {error}", params.id))?;
                notifiers.insert(params.id.clone(), Arc::new(value));
            }
            kemuri_config::NotifierConfig::Smtp(params) => {
                let value = SmtpNotifier::from_config(params)
                    .map_err(|error| format!("notifier {}: {error}", params.id))?;
                notifiers.insert(params.id.clone(), Arc::new(value));
            }
        }
    }
    Ok(notifiers)
}

async fn resolve_removed_alerts(
    pool: &sqlx::SqlitePool,
    config: &KemuriConfig,
) -> Result<(), sqlx::Error> {
    let configured_rules: std::collections::HashSet<&str> =
        config.rules.iter().map(|rule| rule.id.as_str()).collect();
    let rows: Vec<(i64, String, i64, i64, String)> = sqlx::query_as(
        "SELECT a.internal_id, a.rule_id, a.check_internal_id,
                a.observer_internal_id, a.state
         FROM alert_states a
         LEFT JOIN checks c ON c.internal_id = a.check_internal_id
         WHERE a.state IN ('firing', 'pending_fire', 'pending_clear')
           AND (c.active = 0 OR c.internal_id IS NULL)",
    )
    .fetch_all(pool)
    .await?;
    let mut candidates = rows;
    let rule_rows: Vec<(i64, String, i64, i64, String)> = sqlx::query_as(
        "SELECT internal_id, rule_id, check_internal_id, observer_internal_id, state
         FROM alert_states
         WHERE state IN ('firing', 'pending_fire', 'pending_clear')",
    )
    .fetch_all(pool)
    .await?;
    for row in rule_rows {
        if !configured_rules.contains(row.1.as_str())
            && !candidates.iter().any(|candidate| candidate.0 == row.0)
        {
            candidates.push(row);
        }
    }
    let now = chrono::Utc::now().to_rfc3339();
    for (internal_id, rule_id, check_id, observer_id, state) in candidates {
        sqlx::query(
            "INSERT INTO alert_events
             (rule_id, check_internal_id, observer_internal_id, event_type,
              from_state, to_state, occurred_at, reason)
             VALUES (?, ?, ?, 'resolved', ?, 'normal', ?, 'config_removed')",
        )
        .bind(&rule_id)
        .bind(check_id)
        .bind(observer_id)
        .bind(&state)
        .bind(&now)
        .execute(pool)
        .await?;
        sqlx::query(
            "UPDATE alert_states SET state = 'normal', state_entered_at = ?,
             first_condition_true_at = NULL, last_evaluated_at = ?
             WHERE internal_id = ?",
        )
        .bind(&now)
        .bind(&now)
        .bind(internal_id)
        .execute(pool)
        .await?;
    }
    Ok(())
}

async fn ensure_observer(pool: &sqlx::SqlitePool) -> Result<i64, ServerError> {
    let existing: Option<(i64,)> =
        sqlx::query_as("SELECT internal_id FROM observers WHERE observer_id = 'local'")
            .fetch_optional(pool)
            .await
            .map_err(|e| ServerError::Storage(e.to_string()))?;

    if let Some((id,)) = existing {
        return Ok(id);
    }

    sqlx::query("INSERT INTO observers (observer_id) VALUES ('local')")
        .execute(pool)
        .await
        .map_err(|e| ServerError::Storage(e.to_string()))?;

    let (id,): (i64,) =
        sqlx::query_as("SELECT internal_id FROM observers WHERE observer_id = 'local'")
            .fetch_one(pool)
            .await
            .map_err(|e| ServerError::Storage(e.to_string()))?;

    Ok(id)
}

fn create_router(state: AppState) -> Router {
    let api_routes = Router::new()
        .route("/info", get(info))
        .route("/system/status", get(system_status))
        .route("/events", get(events_handler))
        .route("/groups", get(api::list_groups))
        .route("/groups/{*group_path}", get(api::get_group))
        .route("/targets", get(api::list_targets))
        .route("/targets/{target_id}", get(api::get_target))
        .route("/targets/{target_id}/checks", get(api::list_checks))
        .route(
            "/targets/{target_id}/checks/{check_id}",
            get(api::get_check),
        )
        .route(
            "/targets/{target_id}/checks/{check_id}/series",
            get(api::get_series),
        )
        .route(
            "/targets/{target_id}/checks/{check_id}/rounds",
            get(api::get_rounds),
        )
        .route("/alerts", get(api::list_alerts))
        .route("/alerts/{alert_id}", get(api::get_alert))
        .route("/alert-events", get(api::list_alert_events));
    let api_routes = api_routes
        .route("/config/reload", post(reload_config))
        .fallback(api_not_found);

    let mut router = Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics_handler))
        .nest("/api/v1", api_routes)
        .route("/api/openapi.json", get(openapi))
        .fallback(ui_fallback);
    if state.config.read().unwrap().server.cors {
        use axum::http::Method;
        use tower_http::cors::{Any, CorsLayer};
        router = router.layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods([Method::GET, Method::HEAD])
                .allow_headers(Any),
        );
    }
    router.with_state(state)
}

async fn healthz() -> &'static str {
    "ok"
}

async fn readyz(State(state): State<AppState>) -> Response {
    if !state.runtime_ready.load(Ordering::Acquire) {
        return (StatusCode::SERVICE_UNAVAILABLE, "runtime task unavailable").into_response();
    }
    if !state.probe_ready.load(Ordering::Acquire) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "probe capability unavailable",
        )
            .into_response();
    }
    if !state.disk_ready.load(Ordering::Acquire) {
        return (StatusCode::SERVICE_UNAVAILABLE, "disk pressure").into_response();
    }
    match sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(&state.pool)
        .await
    {
        Ok(1) => (StatusCode::OK, "ok").into_response(),
        _ => (StatusCode::SERVICE_UNAVAILABLE, "not ready").into_response(),
    }
}

fn percentage_ratio(value: &str, fallback: f64) -> f64 {
    value
        .trim_end_matches('%')
        .parse::<f64>()
        .map(|value| value / 100.0)
        .unwrap_or(fallback)
}

async fn metrics_handler(State(state): State<AppState>) -> String {
    state.prometheus_handle.render()
}

#[utoipa::path(
    get,
    path = "/api/v1/info",
    responses((status = 200, description = "Build information"))
)]
async fn info(State(state): State<AppState>) -> Json<BuildInfo> {
    Json(state.build_info.as_ref().clone())
}

#[utoipa::path(
    get,
    path = "/api/v1/system/status",
    responses((status = 200, description = "Runtime and dependency status"))
)]
async fn system_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let uptime = state.started_at.elapsed().as_secs();
    let config = state.config.read().unwrap().clone();
    let db_size: i64 = match std::fs::metadata(&config.storage.path) {
        Ok(m) => m.len() as i64,
        Err(_) => 0,
    };
    let data_directory = std::path::Path::new(&config.storage.path)
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let disk_free_bytes = fs2::available_space(data_directory).ok();
    let disk_total_bytes = fs2::total_space(data_directory).ok();
    let last_reload = state.last_reload.read().unwrap().clone();

    let pending_outbox: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM notification_outbox WHERE status = 'pending'")
            .fetch_one(&state.pool)
            .await
            .unwrap_or((0,));

    let schema_version: (String,) =
        sqlx::query_as("SELECT value FROM _sqlx_migrations ORDER BY version DESC LIMIT 1")
            .fetch_one(&state.pool)
            .await
            .unwrap_or(("unknown".to_owned(),));

    let active_alerts: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM alert_states WHERE state IN ('firing', 'pending_fire')",
    )
    .fetch_one(&state.pool)
    .await
    .unwrap_or((0,));

    let config_generation: (Option<String>,) = sqlx::query_as(
        "SELECT generation_hash FROM config_events ORDER BY occurred_at DESC LIMIT 1",
    )
    .fetch_one(&state.pool)
    .await
    .unwrap_or((None,));

    Json(serde_json::json!({
        "status": "running",
        "uptime_seconds": uptime,
        "database_path": config.storage.path,
        "database_size_bytes": db_size,
        "disk_free_bytes": disk_free_bytes,
        "disk_total_bytes": disk_total_bytes,
        "disk_ready": state.disk_ready.load(Ordering::Acquire),
        "runtime_ready": state.runtime_ready.load(Ordering::Acquire),
        "probe_ready": state.probe_ready.load(Ordering::Acquire),
        "schema_version": schema_version.0,
        "config_generation": config_generation.0,
        "notification_outbox_pending": pending_outbox.0,
        "active_alerts": active_alerts.0,
        "last_config_reload": last_reload,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/events",
    responses((status = 200, description = "Server-sent event stream"))
)]
async fn events_handler(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let rx = state.event_tx.subscribe();
    let shutdown_rx = state.shutdown_tx.subscribe();
    let stream =
        futures::stream::unfold((rx, shutdown_rx), |(mut rx, mut shutdown_rx)| async move {
            loop {
                tokio::select! {
                    event = rx.recv() => match event {
                        Ok(event) => {
                            return Some((Ok(event.to_sse_event()), (rx, shutdown_rx)));
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            continue;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            return None;
                        }
                    },
                    _ = shutdown_rx.recv() => {
                        return None;
                    }
                }
            }
        });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[utoipa::path(
    post,
    path = "/api/v1/config/reload",
    responses(
        (status = 202, description = "Reload accepted"),
        (status = 400, body = api::ApiError)
    )
)]
async fn reload_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<serde_json::Value>), api::ApiError> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !content_type.starts_with("application/json") {
        return Err(api::bad_request_public(
            "Content-Type must be application/json",
        ));
    }
    if let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        let config = state.config.read().unwrap().clone();
        let local_origin = format!("http://{}:{}", config.server.bind, config.server.port);
        if config.server.public_url.as_deref() != Some(origin) && origin != local_origin {
            return Err(api::bad_request_public(
                "cross-origin reload is not permitted",
            ));
        }
    }
    state
        .reload_tx
        .try_send(())
        .map_err(|_| api::bad_request_public("a configuration reload is already pending"))?;
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({"status": "reload_pending"})),
    ))
}

async fn api_not_found() -> impl IntoResponse {
    api::not_found_public("route_not_found", "The requested API route does not exist")
}

#[derive(OpenApi)]
#[openapi(
    info(title = "Kemuri API", version = env!("CARGO_PKG_VERSION")),
    paths(
        info,
        system_status,
        events_handler,
        reload_config,
        api::list_groups,
        api::get_group,
        api::list_targets,
        api::get_target,
        api::list_checks,
        api::get_check,
        api::get_series,
        api::get_rounds,
        api::list_alerts,
        api::get_alert,
        api::list_alert_events
    ),
    components(schemas(
        api::ApiError,
        api::AlertStateResponse,
        api::AlertsListResponse,
        api::AlertEventResponse,
        api::AlertEventsResponse,
        api::GroupResponse,
        api::GroupsResponse,
        api::TargetSummary,
        api::TargetsResponse,
        api::CheckSummary,
        api::ChecksResponse,
        api::TargetDetail,
        api::CheckDetail,
        api::SeriesPoint,
        api::SeriesResponse,
        api::SeriesAlertEvent,
        api::SeriesRevisionMarker,
        api::SampleDetail,
        api::RoundSummary,
        api::RoundsResponse
    ))
)]
pub struct ApiDoc;

pub fn openapi_document() -> utoipa::openapi::OpenApi {
    ApiDoc::openapi()
}

async fn openapi() -> Json<serde_json::Value> {
    Json(serde_json::to_value(openapi_document()).expect("OpenAPI document must serialize"))
}

#[derive(RustEmbed)]
#[folder = "../../web/dist"]
struct WebAssets;

async fn ui_fallback(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if !path.is_empty() {
        if let Some(asset) = WebAssets::get(path) {
            return asset_response(path, asset.data);
        }
        if path
            .rsplit('/')
            .next()
            .is_some_and(|part| part.contains('.'))
        {
            return StatusCode::NOT_FOUND.into_response();
        }
    }
    match WebAssets::get("index.html") {
        Some(asset) => asset_response("index.html", asset.data),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn asset_response(path: &str, data: std::borrow::Cow<'static, [u8]>) -> Response {
    let content_type = mime_guess::from_path(path).first_or_octet_stream();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type.as_ref())
        .body(Body::from(data.into_owned()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

#[cfg(test)]
mod openapi_tests {
    use std::collections::BTreeSet;

    use super::openapi_document;

    #[test]
    fn document_matches_api_router_paths() {
        let document = openapi_document();
        let actual: BTreeSet<&str> = document.paths.paths.keys().map(String::as_str).collect();
        let expected: BTreeSet<&str> = [
            "/api/v1/alert-events",
            "/api/v1/alerts",
            "/api/v1/alerts/{alert_id}",
            "/api/v1/config/reload",
            "/api/v1/events",
            "/api/v1/groups",
            "/api/v1/groups/{group_path}",
            "/api/v1/info",
            "/api/v1/system/status",
            "/api/v1/targets",
            "/api/v1/targets/{target_id}",
            "/api/v1/targets/{target_id}/checks",
            "/api/v1/targets/{target_id}/checks/{check_id}",
            "/api/v1/targets/{target_id}/checks/{check_id}/rounds",
            "/api/v1/targets/{target_id}/checks/{check_id}/series",
        ]
        .into_iter()
        .collect();
        assert_eq!(actual, expected);
        assert_eq!(document.info.version, env!("CARGO_PKG_VERSION"));
    }
}
