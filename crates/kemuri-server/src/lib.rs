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
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse};
use axum::routing::get;
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
use tokio::sync::mpsc;

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
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ReloadStatus {
    pub generation: String,
    pub result: String,
    pub error: Option<String>,
    pub timestamp: String,
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

    kemuri_storage::reconcile(&pool, &config)
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

    if resolved
        .checks
        .iter()
        .any(|c| c.probe_kind == kemuri_core::ProbeKind::Http)
    {
        let http_config = HttpProbeConfig::default();
        if let Ok(http_probe) = HttpProbe::new(http_config) {
            registry.register(Arc::new(http_probe));
        }
    }

    if resolved
        .checks
        .iter()
        .any(|c| c.probe_kind == kemuri_core::ProbeKind::Icmp)
    {
        let icmp_cap = check_icmp_capability();
        if icmp_cap.is_available() {
            let icmp_config = IcmpProbeConfig::default();
            registry.register(Arc::new(IcmpProbe::new(icmp_config)));
        } else {
            tracing::warn!(
                "ICMP checks configured but ICMP capability not available. \
                 Ensure the process has permission to create ICMP sockets \
                 (e.g., add to the ping group or set CAP_NET_RAW)"
            );
        }
    }

    if resolved
        .checks
        .iter()
        .any(|c| c.probe_kind == kemuri_core::ProbeKind::Tcp)
    {
        let tcp_config = TcpProbeConfig::default();
        registry.register(Arc::new(TcpProbe::new(tcp_config)));
    }

    if resolved
        .checks
        .iter()
        .any(|c| c.probe_kind == kemuri_core::ProbeKind::Dns)
    {
        let dns_config = DnsProbeConfig::default();
        registry.register(Arc::new(DnsProbe::new(dns_config)));
    }

    let registry = Arc::new(registry);

    let (job_tx, job_rx) = mpsc::channel::<worker::RoundJob>(256);
    let (result_tx, result_rx) = mpsc::channel::<writer::RoundResult>(256);
    let (alert_tx, alert_rx) = mpsc::channel::<alerts::AlertNotification>(256);
    let (event_tx, _) = tokio::sync::broadcast::channel::<SystemEvent>(256);

    let clock = Arc::new(kemuri_core::RealClock);

    let shared_config = Arc::new(std::sync::RwLock::new(Arc::new(config.clone())));

    let mut scheduler_inst = Scheduler::new(
        config.scheduler.clone(),
        resolved.checks.clone(),
        job_tx,
        clock.clone(),
    );

    let running_rounds = scheduler_inst.running_rounds();

    let worker_pool = WorkerPool::new(registry.clone(), 4);
    let _worker_handles = worker_pool.start(job_rx, result_tx, running_rounds);

    let writer_storage = Arc::new(storage);
    let writer = StorageWriter::new(writer_storage.clone(), observer_internal_id)
        .with_alert_channel(alert_tx)
        .with_event_channel(event_tx.clone());
    let writer_handle = tokio::spawn(async move {
        writer.run(result_rx).await;
    });

    let alert_evaluator = AlertEvaluator::new(
        Arc::new(writer_storage.pool().clone()),
        shared_config.clone(),
        clock.clone(),
        observer_internal_id,
    )
    .with_event_channel(event_tx.clone());
    let _alert_handle = tokio::spawn(async move {
        alert_evaluator.run(alert_rx).await;
    });

    let mut notifier_map: HashMap<NotifierId, Arc<dyn Notifier>> = HashMap::new();
    for notifier_cfg in &config.notifiers {
        match notifier_cfg {
            kemuri_config::NotifierConfig::Webhook(params) => {
                if let Ok(notifier) = WebhookNotifier::from_config(params) {
                    let id = params.id.clone();
                    notifier_map.insert(id, Arc::new(notifier));
                }
            }
            kemuri_config::NotifierConfig::Smtp(params) => {
                if let Ok(notifier) = SmtpNotifier::from_config(params) {
                    let id = params.id.clone();
                    notifier_map.insert(id, Arc::new(notifier));
                }
            }
        }
    }
    let shared_notifiers = Arc::new(std::sync::RwLock::new(notifier_map));

    let (worker_shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);

    let notification_worker = NotificationWorker::new(
        Arc::new(writer_storage.pool().clone()),
        shared_notifiers.clone(),
        clock.clone(),
        config.server.public_url.clone(),
    );
    let notification_shutdown = worker_shutdown_tx.subscribe();
    let _notification_handle = tokio::spawn(async move {
        notification_worker.run(notification_shutdown).await;
    });

    let no_data_evaluator = AlertEvaluator::new(
        Arc::new(writer_storage.pool().clone()),
        shared_config.clone(),
        clock.clone(),
        observer_internal_id,
    )
    .with_event_channel(event_tx.clone());
    let mut no_data_shutdown = worker_shutdown_tx.subscribe();
    let _no_data_handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    no_data_evaluator.run_no_data_check().await;
                }
                _ = no_data_shutdown.recv() => {
                    return;
                }
            }
        }
    });

    let rollup_pool = pool.clone();
    let rollup_shutdown = worker_shutdown_tx.subscribe();
    let rollup_worker = rollup_worker::RollupWorker::new(Arc::new(rollup_pool));
    let _rollup_handle = tokio::spawn(async move {
        rollup_worker.run(rollup_shutdown).await;
    });

    let retention_pool = pool.clone();
    let retention_shutdown = worker_shutdown_tx.subscribe();
    let retention_config = config.storage.retention.clone();
    let retention_worker =
        retention_worker::RetentionWorker::new(Arc::new(retention_pool), retention_config);
    let _retention_handle = tokio::spawn(async move {
        retention_worker.run(retention_shutdown).await;
    });

    scheduler_inst.start();

    metrics::counter!("kemuri_build_info",
        "version" => build_info.version.clone(),
        "git_hash" => build_info.git_hash.clone())
    .increment(1);
    metrics::gauge!("kemuri_process_start_time_seconds").set(chrono::Utc::now().timestamp() as f64);

    let last_reload = Arc::new(std::sync::RwLock::new(None::<ReloadStatus>));

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

    let (shutdown_tx, shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);
    let (reload_tx, mut reload_rx) = tokio::sync::mpsc::channel::<()>(1);

    let mut sigterm_rx = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|e| ServerError::Serve(e.to_string()))?;
    let mut sighup_rx = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::hangup())
        .map_err(|e| ServerError::Serve(e.to_string()))?;

    let shutdown_tx_sigint = shutdown_tx.clone();
    let shutdown_tx_sigterm = shutdown_tx.clone();
    tokio::spawn(async move {
        tokio::signal::ctrl_c().await.ok();
        let _ = shutdown_tx_sigint.send(());
    });

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

    let shutdown_timeout = kemuri_core::parse_duration(&config.server.shutdown_timeout)
        .unwrap_or(std::time::Duration::from_secs(30));

    let mut shutdown_rx1 = shutdown_rx.resubscribe();
    let server = axum::serve(listener, app);

    let result = tokio::select! {
        result = server.with_graceful_shutdown(async move {
            let _ = shutdown_rx1.recv().await;
            tracing::info!("shutdown signal received, draining connections");
        }) => {
            result
        }
        _ = reload_rx.recv() => {
            perform_reload(
                &shared_config,
                &shared_notifiers,
                &mut scheduler_inst,
                &writer_storage,
                &event_tx,
                &last_reload,
                &pool,
                observer_internal_id,
                &config_path_arc,
            ).await;
            Ok(())
        }
    };

    match result {
        Ok(()) => {}
        Err(e) => return Err(ServerError::Serve(e.to_string())),
    }

    tracing::info!("stopping scheduler and workers");
    scheduler_inst.stop();

    let _ = worker_shutdown_tx.send(());

    let deadline = tokio::time::sleep(shutdown_timeout);
    tokio::pin!(deadline);

    tokio::select! {
        _ = writer_handle => {}
        _ = &mut deadline => {
            tracing::warn!("graceful shutdown deadline exceeded");
        }
    }

    tracing::info!("kemuri server stopped");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn perform_reload(
    shared_config: &Arc<std::sync::RwLock<Arc<KemuriConfig>>>,
    shared_notifiers: &Arc<std::sync::RwLock<HashMap<NotifierId, Arc<dyn Notifier>>>>,
    scheduler: &mut Scheduler,
    storage: &Arc<StorageManager>,
    event_tx: &tokio::sync::broadcast::Sender<SystemEvent>,
    last_reload: &Arc<std::sync::RwLock<Option<ReloadStatus>>>,
    pool: &sqlx::SqlitePool,
    _observer_internal_id: i64,
    config_path: &std::path::Path,
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
                timestamp: chrono::Utc::now().to_rfc3339(),
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
                timestamp: chrono::Utc::now().to_rfc3339(),
            };
            *last_reload.write().unwrap() = Some(status);
            return;
        }
    };

    let new_generation = new_resolved.generation.to_string();

    if let Err(e) = kemuri_storage::reconcile(pool, &new_config).await {
        tracing::error!(error = %e, "config reload failed: database reconciliation error");
        metrics::counter!("kemuri_config_reload_total", "result" => "failure").increment(1);
        let status = ReloadStatus {
            generation: new_generation,
            result: "failure".to_owned(),
            error: Some(e.to_string()),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };
        *last_reload.write().unwrap() = Some(status);
        return;
    }

    scheduler.reconcile(new_resolved.checks.clone()).await;

    {
        let mut notifier_map: HashMap<NotifierId, Arc<dyn Notifier>> = HashMap::new();
        for notifier_cfg in &new_config.notifiers {
            match notifier_cfg {
                kemuri_config::NotifierConfig::Webhook(params) => {
                    if let Ok(notifier) = WebhookNotifier::from_config(params) {
                        let id = params.id.clone();
                        notifier_map.insert(id, Arc::new(notifier));
                    }
                }
                kemuri_config::NotifierConfig::Smtp(params) => {
                    if let Ok(notifier) = SmtpNotifier::from_config(params) {
                        let id = params.id.clone();
                        notifier_map.insert(id, Arc::new(notifier));
                    }
                }
            }
        }
        *shared_notifiers.write().unwrap() = notifier_map;
    }

    {
        *shared_config.write().unwrap() = Arc::new(new_config);
    }

    let _ = storage
        .write_config_event(kemuri_storage::InsertConfigEvent {
            generation_hash: new_generation.clone(),
            event_type: "reload".to_owned(),
            summary: Some("configuration reloaded via SIGHUP".to_owned()),
        })
        .await;

    let _ = event_tx.send(SystemEvent::config_reloaded(&new_generation, "success"));

    metrics::counter!("kemuri_config_reload_total", "result" => "success").increment(1);

    let status = ReloadStatus {
        generation: new_generation,
        result: "success".to_owned(),
        error: None,
        timestamp: chrono::Utc::now().to_rfc3339(),
    };
    *last_reload.write().unwrap() = Some(status);

    tracing::info!("configuration reload completed successfully");
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

    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics_handler))
        .nest("/api/v1", api_routes)
        .fallback(ui_fallback)
        .with_state(state)
}

async fn healthz() -> &'static str {
    "ok"
}

async fn readyz() -> &'static str {
    "ok"
}

async fn metrics_handler(State(state): State<AppState>) -> String {
    state.prometheus_handle.render()
}

async fn info(State(state): State<AppState>) -> Json<BuildInfo> {
    Json(state.build_info.as_ref().clone())
}

async fn system_status(State(state): State<AppState>) -> Json<serde_json::Value> {
    let uptime = state.started_at.elapsed().as_secs();
    let config = state.config.read().unwrap().clone();
    let db_size: i64 = match std::fs::metadata(&config.storage.path) {
        Ok(m) => m.len() as i64,
        Err(_) => 0,
    };
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
        "schema_version": schema_version.0,
        "config_generation": config_generation.0,
        "notification_outbox_pending": pending_outbox.0,
        "active_alerts": active_alerts.0,
        "last_config_reload": last_reload,
    }))
}

async fn events_handler(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let rx = state.event_tx.subscribe();
    let stream = futures::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    return Some((Ok(event.to_sse_event()), rx));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    return None;
                }
            }
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn ui_fallback() -> impl IntoResponse {
    Html(
        r#"<!DOCTYPE html><html lang="en"><head><meta charset="UTF-8"><meta name="viewport" content="width=device-width,initial-scale=1.0"><title>Kemuri</title></head><body><div id="root"></div><script type="module" src="/assets/index.js"></script></body></html>"#,
    )
}
