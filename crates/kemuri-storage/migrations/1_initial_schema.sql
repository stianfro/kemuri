CREATE TABLE targets (
    internal_id INTEGER PRIMARY KEY AUTOINCREMENT,
    target_id TEXT NOT NULL,
    name TEXT NOT NULL,
    group_path TEXT NOT NULL DEFAULT '',
    labels TEXT NOT NULL DEFAULT '{}',
    active INTEGER NOT NULL DEFAULT 1,
    first_seen_at TEXT NOT NULL DEFAULT (datetime('now')),
    last_seen_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(target_id)
);

CREATE TABLE checks (
    internal_id INTEGER PRIMARY KEY AUTOINCREMENT,
    target_internal_id INTEGER NOT NULL REFERENCES targets(internal_id),
    check_id TEXT NOT NULL,
    probe_type TEXT NOT NULL,
    active INTEGER NOT NULL DEFAULT 1,
    current_revision_id TEXT,
    first_seen_at TEXT NOT NULL DEFAULT (datetime('now')),
    last_seen_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(target_internal_id, check_id)
);

CREATE TABLE observers (
    internal_id INTEGER PRIMARY KEY AUTOINCREMENT,
    observer_id TEXT NOT NULL UNIQUE,
    status TEXT NOT NULL DEFAULT 'active',
    first_seen_at TEXT NOT NULL DEFAULT (datetime('now')),
    last_seen_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE check_assignments (
    check_internal_id INTEGER NOT NULL REFERENCES checks(internal_id),
    observer_internal_id INTEGER NOT NULL REFERENCES observers(internal_id),
    active INTEGER NOT NULL DEFAULT 1,
    assigned_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(check_internal_id, observer_internal_id)
);

CREATE TABLE check_revisions (
    internal_id INTEGER PRIMARY KEY AUTOINCREMENT,
    check_internal_id INTEGER NOT NULL REFERENCES checks(internal_id),
    revision_id TEXT NOT NULL,
    effective_at TEXT NOT NULL DEFAULT (datetime('now')),
    redacted_config TEXT NOT NULL,
    UNIQUE(check_internal_id, revision_id)
);

CREATE TABLE config_events (
    internal_id INTEGER PRIMARY KEY AUTOINCREMENT,
    generation_hash TEXT NOT NULL,
    event_type TEXT NOT NULL,
    summary TEXT,
    occurred_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE rounds (
    internal_id INTEGER PRIMARY KEY AUTOINCREMENT,
    check_internal_id INTEGER NOT NULL REFERENCES checks(internal_id),
    observer_internal_id INTEGER NOT NULL REFERENCES observers(internal_id),
    scheduled_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT,
    execution_status TEXT NOT NULL,
    stop_reason TEXT,
    configured_samples INTEGER NOT NULL,
    attempted_samples INTEGER NOT NULL DEFAULT 0,
    latency_bearing_samples INTEGER NOT NULL DEFAULT 0,
    healthy_samples INTEGER NOT NULL DEFAULT 0,
    unhealthy_samples INTEGER NOT NULL DEFAULT 0,
    measurement_loss_samples INTEGER NOT NULL DEFAULT 0,
    min_latency_ns INTEGER,
    median_latency_ns INTEGER,
    max_latency_ns INTEGER,
    sample_blob BLOB,
    outcome_summary TEXT,
    config_generation TEXT,
    check_revision_id TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(check_internal_id, observer_internal_id, scheduled_at)
);

CREATE INDEX idx_rounds_check_scheduled ON rounds(check_internal_id, scheduled_at DESC);

CREATE TABLE rollups (
    check_internal_id INTEGER NOT NULL REFERENCES checks(internal_id),
    observer_internal_id INTEGER NOT NULL REFERENCES observers(internal_id),
    resolution_seconds INTEGER NOT NULL,
    bucket_start TEXT NOT NULL,
    scheduled_rounds INTEGER NOT NULL DEFAULT 0,
    completed_rounds INTEGER NOT NULL DEFAULT 0,
    partial_rounds INTEGER NOT NULL DEFAULT 0,
    configured_sample_slots INTEGER NOT NULL DEFAULT 0,
    attempted_samples INTEGER NOT NULL DEFAULT 0,
    latency_bearing_samples INTEGER NOT NULL DEFAULT 0,
    healthy_samples INTEGER NOT NULL DEFAULT 0,
    unhealthy_samples INTEGER NOT NULL DEFAULT 0,
    measurement_loss_samples INTEGER NOT NULL DEFAULT 0,
    outcome_counts TEXT NOT NULL DEFAULT '{}',
    min_latency_ns INTEGER,
    max_latency_ns INTEGER,
    sum_latency_ns INTEGER NOT NULL DEFAULT 0,
    histogram_version INTEGER NOT NULL DEFAULT 1,
    histogram_blob BLOB,
    no_data_counts TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (check_internal_id, observer_internal_id, resolution_seconds, bucket_start)
);

CREATE TABLE check_current_state (
    check_internal_id INTEGER NOT NULL REFERENCES checks(internal_id),
    observer_internal_id INTEGER NOT NULL REFERENCES observers(internal_id),
    state TEXT NOT NULL DEFAULT 'unknown',
    last_round_at TEXT,
    last_latency_ns INTEGER,
    last_measurement_loss_ratio REAL,
    last_health_failure_ratio REAL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (check_internal_id, observer_internal_id)
);

CREATE TABLE alert_states (
    internal_id INTEGER PRIMARY KEY AUTOINCREMENT,
    rule_id TEXT NOT NULL,
    check_internal_id INTEGER NOT NULL REFERENCES checks(internal_id),
    observer_internal_id INTEGER NOT NULL REFERENCES observers(internal_id),
    state TEXT NOT NULL DEFAULT 'normal',
    state_entered_at TEXT NOT NULL DEFAULT (datetime('now')),
    first_condition_true_at TEXT,
    last_evaluated_at TEXT,
    last_notification_at TEXT,
    fingerprint TEXT,
    last_metric_value REAL,
    UNIQUE(rule_id, check_internal_id, observer_internal_id)
);

CREATE TABLE alert_events (
    internal_id INTEGER PRIMARY KEY AUTOINCREMENT,
    rule_id TEXT NOT NULL,
    check_internal_id INTEGER NOT NULL REFERENCES checks(internal_id),
    observer_internal_id INTEGER NOT NULL REFERENCES observers(internal_id),
    event_type TEXT NOT NULL,
    from_state TEXT NOT NULL,
    to_state TEXT NOT NULL,
    metric_value REAL,
    threshold_value REAL,
    occurred_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_alert_events_check ON alert_events(check_internal_id, occurred_at DESC);
CREATE INDEX idx_alert_events_rule ON alert_events(rule_id, occurred_at DESC);

CREATE TABLE notification_outbox (
    internal_id INTEGER PRIMARY KEY AUTOINCREMENT,
    alert_event_internal_id INTEGER NOT NULL REFERENCES alert_events(internal_id),
    notifier_id TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    attempt_count INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TEXT NOT NULL DEFAULT (datetime('now')),
    last_attempt_at TEXT,
    last_error TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX idx_outbox_pending ON notification_outbox(status, next_attempt_at);
