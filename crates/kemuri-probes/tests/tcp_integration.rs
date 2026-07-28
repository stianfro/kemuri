use std::collections::HashMap;
use std::time::Duration;

use kemuri_core::{CheckId, ObserverId, ProbeKind, ProfileId, SampleOutcome, TargetId};
use kemuri_probes::{Probe, ResolvedCheck, RoundContext, TcpProbe, TcpProbeConfig};

fn make_tcp_check(host: &str, port: u16) -> ResolvedCheck {
    let mut params = HashMap::new();
    params.insert("host".to_owned(), host.to_owned());
    params.insert("port".to_owned(), port.to_string());
    ResolvedCheck {
        check_id: CheckId::new("test-tcp").unwrap(),
        target_id: TargetId::new("test-target").unwrap(),
        profile_id: ProfileId::new("test-profile").unwrap(),
        address: host.to_owned(),
        probe_kind: ProbeKind::Tcp,
        timeout: Duration::from_secs(5),
        sample_count: 1,
        params,
    }
}

fn make_context() -> RoundContext {
    RoundContext {
        observer_id: ObserverId::new("test-observer").unwrap(),
        scheduled_at: Duration::from_secs(0),
        deadline: Duration::from_secs(5),
    }
}

#[tokio::test]
async fn tcp_connect_success() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        drop(stream);
        tokio::time::sleep(Duration::from_millis(100)).await;
    });

    let probe = TcpProbe::new(TcpProbeConfig::default());
    let result = probe
        .execute_round(make_context(), make_tcp_check("127.0.0.1", port))
        .await
        .unwrap();

    assert_eq!(result.results.len(), 1);
    assert_eq!(result.results[0].outcome, SampleOutcome::Success);
    assert!(result.results[0].latency.is_some());
    assert!(result.results[0].metadata.is_some());
    let meta = result.results[0].metadata.as_ref().unwrap();
    assert_eq!(meta.get("port").unwrap(), &port.to_string());
    assert_eq!(meta.get("ip_family").unwrap(), "ipv4");
    server.abort();
}

#[tokio::test]
async fn tcp_connect_refused() {
    let probe = TcpProbe::new(TcpProbeConfig::default());
    let result = probe
        .execute_round(make_context(), make_tcp_check("127.0.0.1", 1))
        .await
        .unwrap();

    assert_eq!(result.results.len(), 1);
    assert_ne!(result.results[0].outcome, SampleOutcome::Success);
}

#[tokio::test]
async fn tcp_connect_timeout() {
    let probe = TcpProbe::new(TcpProbeConfig::default());
    let mut check = make_tcp_check("192.0.2.1", 80);
    check.timeout = Duration::from_millis(200);

    let result = tokio::time::timeout(
        Duration::from_secs(2),
        probe.execute_round(make_context(), check),
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(result.results.len(), 1);
    assert_eq!(result.results[0].outcome, SampleOutcome::Timeout);
}

#[tokio::test]
async fn tcp_connect_dns_failure() {
    let probe = TcpProbe::new(TcpProbeConfig::default());
    let result = probe
        .execute_round(make_context(), make_tcp_check("nonexistent.invalid", 80))
        .await
        .unwrap();

    assert_eq!(result.results.len(), 1);
    assert_eq!(result.results[0].outcome, SampleOutcome::DnsError);
}

#[tokio::test]
async fn tcp_metadata_contains_resolved_ip() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        drop(stream);
        tokio::time::sleep(Duration::from_millis(100)).await;
    });

    let probe = TcpProbe::new(TcpProbeConfig::default());
    let result = probe
        .execute_round(make_context(), make_tcp_check("127.0.0.1", port))
        .await
        .unwrap();

    let meta = result.results[0].metadata.as_ref().unwrap();
    assert!(meta.contains_key("resolved_ip"));
    assert!(meta.contains_key("ip_family"));
    assert!(meta.contains_key("port"));
    server.abort();
}
