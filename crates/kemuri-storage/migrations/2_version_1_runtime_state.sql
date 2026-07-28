ALTER TABLE checks ADD COLUMN profile_id TEXT;
ALTER TABLE checks ADD COLUMN config_generation TEXT;
ALTER TABLE checks ADD COLUMN redacted_resolved_config TEXT NOT NULL DEFAULT '{}';
ALTER TABLE checks ADD COLUMN observer_assignment TEXT NOT NULL DEFAULT 'local';
ALTER TABLE alert_events ADD COLUMN reason TEXT;

CREATE INDEX idx_targets_group_cursor
    ON targets(active, group_path, target_id, internal_id);
CREATE INDEX idx_checks_profile_cursor
    ON checks(active, profile_id, target_internal_id, check_id, internal_id);
CREATE INDEX idx_checks_observer
    ON checks(observer_assignment, active);
CREATE INDEX idx_rounds_range_cursor
    ON rounds(check_internal_id, observer_internal_id, scheduled_at, internal_id);
CREATE INDEX idx_rounds_generation_revision
    ON rounds(config_generation, check_revision_id);
CREATE INDEX idx_alert_states_profile_query
    ON alert_states(rule_id, state, check_internal_id, internal_id);
CREATE INDEX idx_alert_events_cursor
    ON alert_events(occurred_at, internal_id);
