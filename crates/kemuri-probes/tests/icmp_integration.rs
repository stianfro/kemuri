use std::time::Duration;

use kemuri_core::{CheckId, ObserverId, ProbeKind, ProfileId, SampleOutcome, TargetId};
use kemuri_probes::{
    AddressFamily, IcmpProbe, IcmpProbeConfig, Probe, ResolvedCheck, RoundContext,
};

fn make_icmp_check(address: &str) -> ResolvedCheck {
    ResolvedCheck {
        check_id: CheckId::new("test-icmp").unwrap(),
        target_id: TargetId::new("test-target").unwrap(),
        profile_id: ProfileId::new("test-profile").unwrap(),
        address: address.to_owned(),
        probe_kind: ProbeKind::Icmp,
        timeout: Duration::from_secs(5),
        sample_count: 1,
        settings: kemuri_probes::ProbeSettings::Defaults,
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
#[ignore]
async fn icmp_ping_localhost_ipv4() {
    let probe = IcmpProbe::new(IcmpProbeConfig::default());
    let result = probe
        .execute_round(make_context(), make_icmp_check("127.0.0.1"))
        .await
        .unwrap();
    assert!(!result.results.is_empty());
    assert!(
        result
            .results
            .iter()
            .any(|r| r.outcome == SampleOutcome::Success),
        "expected at least one success, got: {:?}",
        result.results
    );
}

#[tokio::test]
#[ignore]
async fn icmp_ping_localhost_ipv6() {
    let config = IcmpProbeConfig {
        address_family: AddressFamily::Ipv6,
        ..Default::default()
    };
    let probe = IcmpProbe::new(config);
    let result = probe
        .execute_round(make_context(), make_icmp_check("::1"))
        .await
        .unwrap();
    assert!(!result.results.is_empty());
    assert!(
        result
            .results
            .iter()
            .any(|r| r.outcome == SampleOutcome::Success),
        "expected at least one success, got: {:?}",
        result.results
    );
}

#[tokio::test]
#[ignore]
async fn icmp_ping_timeout() {
    let probe = IcmpProbe::new(IcmpProbeConfig::default());
    let mut check = make_icmp_check("192.0.2.1");
    check.timeout = Duration::from_millis(500);
    let result = probe.execute_round(make_context(), check).await.unwrap();
    assert!(!result.results.is_empty());
    assert!(
        result
            .results
            .iter()
            .any(|r| r.outcome == SampleOutcome::Timeout),
        "expected timeout, got: {:?}",
        result.results
    );
}

#[tokio::test]
#[ignore]
async fn icmp_ping_dns_failure() {
    let probe = IcmpProbe::new(IcmpProbeConfig::default());
    let result = probe
        .execute_round(make_context(), make_icmp_check("nonexistent.invalid"))
        .await;
    assert!(result.is_err());
}

#[tokio::test]
#[ignore]
async fn icmp_multiple_samples() {
    let probe = IcmpProbe::new(IcmpProbeConfig::default());
    let mut check = make_icmp_check("127.0.0.1");
    check.sample_count = 3;
    check.timeout = Duration::from_secs(10);
    let result = probe.execute_round(make_context(), check).await.unwrap();
    assert_eq!(result.results.len(), 3);
    let successes = result
        .results
        .iter()
        .filter(|r| r.outcome == SampleOutcome::Success)
        .count();
    assert!(
        successes >= 1,
        "expected at least one success, got: {:?}",
        result.results
    );
}
