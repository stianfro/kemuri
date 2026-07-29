use std::time::Duration;

use kemuri_config::ResolvedDnsParams;
use kemuri_core::{CheckId, ObserverId, ProbeKind, ProfileId, SampleOutcome, TargetId};
use kemuri_probes::{
    DnsProbe, DnsProbeConfig, DnsProtocol, DnsResponseCode, Probe, ResolvedCheck, RoundContext,
};

fn make_dns_check(name: &str, record_type: &str, server: Option<&str>) -> ResolvedCheck {
    ResolvedCheck {
        check_id: CheckId::new("test-dns").unwrap(),
        target_id: TargetId::new("test-target").unwrap(),
        profile_id: ProfileId::new("test-profile").unwrap(),
        address: name.to_owned(),
        probe_kind: ProbeKind::Dns,
        timeout: Duration::from_secs(5),
        sample_count: 1,
        settings: kemuri_probes::ProbeSettings::Dns(ResolvedDnsParams {
            domain: name.to_owned(),
            record_type: Some(record_type.to_owned()),
            resolver: server.map(str::to_owned),
            protocol: "udp".to_owned(),
            expected_rcode: "noerror".to_owned(),
            require_answer: false,
        }),
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
async fn dns_query_a_record_success() {
    let config = DnsProbeConfig {
        protocol: DnsProtocol::Udp,
        expected_rcode: DnsResponseCode::NoError,
        require_answer: false,
    };
    let probe = DnsProbe::new(config);
    let result = probe
        .execute_round(
            make_context(),
            make_dns_check("example.com", "A", Some("1.1.1.1")),
        )
        .await
        .unwrap();

    assert_eq!(result.results.len(), 1);
    assert!(
        matches!(
            result.results[0].outcome,
            SampleOutcome::Success | SampleOutcome::UnexpectedResponse
        ),
        "expected success or unexpected response, got: {:?}",
        result.results[0].outcome
    );
    assert!(result.results[0].metadata.is_some());
    let meta = result.results[0].metadata.as_ref().unwrap();
    assert!(meta.contains_key("response_code"));
    assert!(meta.contains_key("answer_count"));
}

#[tokio::test]
async fn dns_query_nxdomain() {
    let config = DnsProbeConfig {
        protocol: DnsProtocol::Udp,
        expected_rcode: DnsResponseCode::NXDomain,
        require_answer: false,
    };
    let probe = DnsProbe::new(config);
    let result = probe
        .execute_round(
            make_context(),
            make_dns_check("nonexistent.invalid.example.com", "A", Some("1.1.1.1")),
        )
        .await
        .unwrap();

    assert_eq!(result.results.len(), 1);
    assert!(
        matches!(
            result.results[0].outcome,
            SampleOutcome::Success | SampleOutcome::UnexpectedResponse | SampleOutcome::DnsError
        ),
        "expected success/nxdomain/dns_error, got: {:?}",
        result.results[0].outcome
    );
}

#[tokio::test]
async fn dns_query_timeout() {
    let config = DnsProbeConfig {
        protocol: DnsProtocol::Udp,
        expected_rcode: DnsResponseCode::NoError,
        require_answer: false,
    };
    let probe = DnsProbe::new(config);
    let mut check = make_dns_check("example.com", "A", Some("192.0.2.1"));
    check.timeout = Duration::from_millis(200);

    let result = tokio::time::timeout(
        Duration::from_secs(3),
        probe.execute_round(make_context(), check),
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(result.results.len(), 1);
    assert!(
        matches!(
            result.results[0].outcome,
            SampleOutcome::Timeout | SampleOutcome::DnsError
        ),
        "expected timeout or dns error, got: {:?}",
        result.results[0].outcome
    );
}

#[tokio::test]
async fn dns_query_connection_refused() {
    let config = DnsProbeConfig {
        protocol: DnsProtocol::Udp,
        expected_rcode: DnsResponseCode::NoError,
        require_answer: false,
    };
    let probe = DnsProbe::new(config);
    let result = probe
        .execute_round(
            make_context(),
            make_dns_check("example.com", "A", Some("127.0.0.1:1")),
        )
        .await
        .unwrap();

    assert_eq!(result.results.len(), 1);
    assert_ne!(result.results[0].outcome, SampleOutcome::Success);
}

#[tokio::test]
async fn dns_metadata_fields() {
    let config = DnsProbeConfig {
        protocol: DnsProtocol::Udp,
        expected_rcode: DnsResponseCode::NoError,
        require_answer: false,
    };
    let probe = DnsProbe::new(config);
    let result = probe
        .execute_round(
            make_context(),
            make_dns_check("example.com", "A", Some("1.1.1.1")),
        )
        .await
        .unwrap();

    if let Some(ref meta) = result.results[0].metadata {
        assert!(meta.contains_key("query_name"));
        assert!(meta.contains_key("record_type"));
        assert!(meta.contains_key("server"));
        assert!(meta.contains_key("protocol"));
        assert_eq!(meta.get("protocol").unwrap(), "udp");
    }
}
