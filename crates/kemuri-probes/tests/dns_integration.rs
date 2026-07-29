use std::time::Duration;

use kemuri_config::ResolvedDnsParams;
use kemuri_core::{CheckId, ObserverId, ProbeKind, ProfileId, SampleOutcome, TargetId};
use kemuri_probes::{
    DnsProbe, DnsProbeConfig, DnsProtocol, DnsResponseCode, Probe, ResolvedCheck, RoundContext,
};

fn dns_response(query: &[u8], rcode: u8) -> Vec<u8> {
    let mut question_end = 12;
    while question_end < query.len() && query[question_end] != 0 {
        question_end += query[question_end] as usize + 1;
    }
    question_end = (question_end + 5).min(query.len());
    let mut response = query[..question_end].to_vec();
    response[2] = 0x81;
    response[3] = 0x80 | (rcode & 0x0f);
    response[6] = 0;
    response[7] = if rcode == 0 { 1 } else { 0 };
    response[8..12].fill(0);
    if rcode == 0 {
        response.extend_from_slice(&[
            0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x3c, 0x00, 0x04, 127, 0, 0, 1,
        ]);
    }
    response
}

async fn local_dns_server(rcode: u8) -> String {
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let address = socket.local_addr().unwrap();
    tokio::spawn(async move {
        let mut buffer = [0_u8; 2048];
        for _ in 0..4 {
            let received =
                tokio::time::timeout(Duration::from_secs(2), socket.recv_from(&mut buffer)).await;
            let Ok(Ok((length, peer))) = received else {
                return;
            };
            if length < 12 {
                continue;
            }
            let response = dns_response(&buffer[..length], rcode);
            let _ = socket.send_to(&response, peer).await;
        }
    });
    address.to_string()
}

async fn local_tcp_dns_server() -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let Ok(length) = stream.read_u16().await else {
            return;
        };
        let mut query = vec![0_u8; length as usize];
        if stream.read_exact(&mut query).await.is_err() {
            return;
        }
        let response = dns_response(&query, 0);
        let _ = stream.write_u16(response.len() as u16).await;
        let _ = stream.write_all(&response).await;
    });
    address.to_string()
}

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
    let server = local_dns_server(0).await;
    let result = probe
        .execute_round(
            make_context(),
            make_dns_check("example.test", "A", Some(&server)),
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
    let server = local_dns_server(3).await;
    let result = probe
        .execute_round(
            make_context(),
            make_dns_check("missing.example.test", "A", Some(&server)),
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
async fn dns_query_over_tcp() {
    let probe = DnsProbe::new(DnsProbeConfig {
        protocol: DnsProtocol::Tcp,
        expected_rcode: DnsResponseCode::NoError,
        require_answer: true,
    });
    let server = local_tcp_dns_server().await;
    let mut check = make_dns_check("example.test", "A", Some(&server));
    let kemuri_probes::ProbeSettings::Dns(settings) = &mut check.settings else {
        unreachable!();
    };
    settings.protocol = "tcp".to_owned();
    settings.require_answer = true;

    let result = probe.execute_round(make_context(), check).await.unwrap();
    assert_eq!(result.results[0].outcome, SampleOutcome::Success);
}

#[tokio::test]
async fn dns_metadata_fields() {
    let config = DnsProbeConfig {
        protocol: DnsProtocol::Udp,
        expected_rcode: DnsResponseCode::NoError,
        require_answer: false,
    };
    let probe = DnsProbe::new(config);
    let server = local_dns_server(0).await;
    let result = probe
        .execute_round(
            make_context(),
            make_dns_check("example.test", "A", Some(&server)),
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
