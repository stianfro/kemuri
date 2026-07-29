use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use kemuri_config::{KemuriConfig, ResolvedDnsParams, ResolvedTcpParams};
use kemuri_core::{CheckId, ProbeKind, ProfileId, SampleOutcome, TargetId};
use kemuri_probes::{
    DnsProbe, DnsProbeConfig, ResolvedCheck, RoundContext, SyntheticProbe, TcpProbe, TcpProbeConfig,
};
use kemuri_server::ProbeRegistry;
use kemuri_storage::StorageManager;

fn make_round_context() -> RoundContext {
    RoundContext {
        observer_id: kemuri_core::ObserverId::new("test-observer").unwrap(),
        scheduled_at: Duration::from_secs(0),
        deadline: Duration::from_secs(5),
    }
}

fn make_resolved_check(
    check_id: &str,
    target_id: &str,
    address: &str,
    probe_kind: ProbeKind,
    settings: kemuri_probes::ProbeSettings,
) -> ResolvedCheck {
    ResolvedCheck {
        check_id: CheckId::new(check_id).unwrap(),
        target_id: TargetId::new(target_id).unwrap(),
        profile_id: ProfileId::new("test-profile").unwrap(),
        address: address.to_owned(),
        probe_kind,
        timeout: Duration::from_secs(5),
        sample_count: 1,
        settings,
    }
}

fn encode_results(results: &[kemuri_probes::SampleResult]) -> Vec<kemuri_core::SampleRecord> {
    results
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let classification = match r.outcome {
                SampleOutcome::Success => kemuri_core::SampleClassification::HealthyResponse,
                SampleOutcome::UnexpectedResponse => {
                    kemuri_core::SampleClassification::UnhealthyResponse
                }
                _ => kemuri_core::SampleClassification::MeasurementLoss,
            };
            kemuri_core::SampleRecord {
                sample_index: i as u16,
                offset_us: 0,
                outcome: r.outcome,
                classification,
                latency_ns: r.latency.map(|d| d.as_nanos() as u64),
                elapsed_ns: r.latency.map(|d| d.as_nanos() as u64),
                metadata: if let Some(ref meta) = r.metadata {
                    Some(serde_json::to_vec(meta).unwrap_or_default())
                } else {
                    r.detail.as_ref().map(|s| s.as_bytes().to_vec())
                },
            }
        })
        .collect()
}

fn make_test_config() -> KemuriConfig {
    serde_yaml::from_str(
        r#"
version: 1
profiles:
  - kind: http
    id: http-default
    url: http://127.0.0.1
    interval: 30s
    timeout: 5s
targets:
  - id: web-1
    address: 127.0.0.1
    name: Web Server 1
    group_path: web
    checks:
      - id: synth-check
        profile: http-default
"#,
    )
    .unwrap()
}

#[tokio::test]
async fn e2e_synthetic_probe_pipeline() {
    let storage = StorageManager::open_in_memory().await.unwrap();
    let pool = storage.pool().clone();

    kemuri_storage::reconcile(&pool, &make_test_config())
        .await
        .unwrap();

    sqlx::query("INSERT OR IGNORE INTO observers (observer_id) VALUES ('local')")
        .execute(&pool)
        .await
        .unwrap();
    let (observer_internal_id,): (i64,) =
        sqlx::query_as("SELECT internal_id FROM observers WHERE observer_id = 'local'")
            .fetch_one(&pool)
            .await
            .unwrap();

    let mut registry = ProbeRegistry::new();
    registry.register(Arc::new(SyntheticProbe::success(Duration::from_millis(10))));

    let ctx = make_round_context();
    let check = make_resolved_check(
        "synth-check",
        "web-1",
        "127.0.0.1",
        ProbeKind::Http,
        kemuri_probes::ProbeSettings::Defaults,
    );

    let probe = registry.get(ProbeKind::Http).unwrap();
    let round = probe.execute_round(ctx, check).await.unwrap();

    assert_eq!(round.results.len(), 1);
    assert_eq!(round.results[0].outcome, SampleOutcome::Success);

    let target_row = kemuri_storage::TargetRepo::get_by_target_id(&pool, "web-1")
        .await
        .unwrap();
    assert!(target_row.is_some());
    let target_row = target_row.unwrap();

    let check_row = kemuri_storage::CheckRepo::get(&pool, target_row.internal_id, "synth-check")
        .await
        .unwrap();
    assert!(check_row.is_some());
    let check_row = check_row.unwrap();

    let records = encode_results(&round.results);
    let blob = kemuri_core::encode_samples(&records);

    let insert = kemuri_storage::InsertRound {
        check_internal_id: check_row.internal_id,
        observer_internal_id,
        scheduled_at: chrono::Utc::now().to_rfc3339(),
        started_at: Some(chrono::Utc::now().to_rfc3339()),
        finished_at: Some(chrono::Utc::now().to_rfc3339()),
        execution_status: "complete".to_owned(),
        stop_reason: None,
        configured_samples: 1,
        attempted_samples: 1,
        latency_bearing_samples: 1,
        healthy_samples: 1,
        unhealthy_samples: 0,
        measurement_loss_samples: 0,
        min_latency_ns: Some(10_000_000),
        median_latency_ns: Some(10_000_000),
        max_latency_ns: Some(10_000_000),
        sample_blob: Some(blob),
        outcome_summary: Some("1/1 healthy".to_owned()),
        config_generation: None,
        check_revision_id: None,
    };

    let result = storage.write_round(insert).await;
    assert!(result.is_ok(), "write_round failed: {:?}", result.err());
}

#[tokio::test]
async fn e2e_tcp_probe_executes() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        drop(stream);
        tokio::time::sleep(Duration::from_millis(100)).await;
    });

    let mut registry = ProbeRegistry::new();
    registry.register(Arc::new(TcpProbe::new(TcpProbeConfig::default())));

    let ctx = make_round_context();
    let check = make_resolved_check(
        "tcp-check",
        "tcp-target",
        "127.0.0.1",
        ProbeKind::Tcp,
        kemuri_probes::ProbeSettings::Tcp(ResolvedTcpParams {
            host: "127.0.0.1".to_owned(),
            port,
            address_family: "auto".to_owned(),
            source_address: None,
            tls: None,
        }),
    );

    let probe = registry.get(ProbeKind::Tcp).unwrap();
    let round = probe.execute_round(ctx, check).await.unwrap();

    assert_eq!(round.results.len(), 1);
    assert_eq!(round.results[0].outcome, SampleOutcome::Success);
    assert!(round.results[0].metadata.is_some());
    assert!(
        round.results[0]
            .metadata
            .as_ref()
            .unwrap()
            .contains_key("resolved_ip")
    );

    server.abort();
}

#[tokio::test]
async fn e2e_dns_probe_executes() {
    let mut registry = ProbeRegistry::new();
    registry.register(Arc::new(DnsProbe::new(DnsProbeConfig::default())));

    let ctx = make_round_context();
    let check = make_resolved_check(
        "dns-check",
        "dns-target",
        "example.com",
        ProbeKind::Dns,
        kemuri_probes::ProbeSettings::Dns(ResolvedDnsParams {
            domain: "example.com".to_owned(),
            record_type: Some("A".to_owned()),
            resolver: Some("1.1.1.1".to_owned()),
            protocol: "udp".to_owned(),
            expected_rcode: "noerror".to_owned(),
            require_answer: false,
        }),
    );

    let probe = registry.get(ProbeKind::Dns).unwrap();
    let round = probe.execute_round(ctx, check).await.unwrap();

    assert_eq!(round.results.len(), 1);
    assert!(
        matches!(
            round.results[0].outcome,
            SampleOutcome::Success | SampleOutcome::UnexpectedResponse
        ),
        "expected success or unexpected response, got: {:?}",
        round.results[0].outcome
    );
    assert!(round.results[0].metadata.is_some());
    let meta = round.results[0].metadata.as_ref().unwrap();
    assert!(meta.contains_key("response_code"));
    assert!(meta.contains_key("answer_count"));
}

#[tokio::test]
async fn e2e_metadata_stored_and_retrieved() {
    let mut meta = HashMap::new();
    meta.insert("resolved_ip".to_owned(), "127.0.0.1".to_owned());
    meta.insert("ip_family".to_owned(), "ipv4".to_owned());
    meta.insert("port".to_owned(), "80".to_owned());

    let sample_result = kemuri_probes::SampleResult {
        outcome: SampleOutcome::Success,
        latency: Some(Duration::from_millis(5)),
        detail: Some("connected".to_owned()),
        metadata: Some(meta),
    };

    let records = encode_results(&[sample_result]);
    let blob = kemuri_core::encode_samples(&records);

    let decoded = kemuri_core::decode_samples(&blob).unwrap();
    assert_eq!(decoded.len(), 1);
    assert!(decoded[0].metadata.is_some());

    let metadata_bytes = decoded[0].metadata.as_ref().unwrap();
    let parsed: HashMap<String, String> = serde_json::from_slice(metadata_bytes).unwrap();
    assert_eq!(parsed.get("resolved_ip").unwrap(), "127.0.0.1");
    assert_eq!(parsed.get("ip_family").unwrap(), "ipv4");
    assert_eq!(parsed.get("port").unwrap(), "80");
}

#[tokio::test]
async fn e2e_all_probe_types_registered() {
    let mut registry = ProbeRegistry::new();
    registry.register(Arc::new(SyntheticProbe::success(Duration::from_millis(10))));
    registry.register(Arc::new(TcpProbe::new(TcpProbeConfig::default())));
    registry.register(Arc::new(DnsProbe::new(DnsProbeConfig::default())));

    assert!(registry.get(ProbeKind::Http).is_some());
    assert!(registry.get(ProbeKind::Tcp).is_some());
    assert!(registry.get(ProbeKind::Dns).is_some());
}
