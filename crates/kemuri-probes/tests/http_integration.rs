use std::collections::HashMap;
use std::time::Duration;

use kemuri_core::{CheckId, ObserverId, ProbeKind, ProfileId, SampleOutcome, TargetId};
use kemuri_probes::{
    HttpConnectionMode, HttpProbe, HttpProbeConfig, Probe, ResolvedCheck, RoundContext,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn make_check() -> ResolvedCheck {
    ResolvedCheck {
        check_id: CheckId::new("test-check").unwrap(),
        target_id: TargetId::new("test-target").unwrap(),
        profile_id: ProfileId::new("test-profile").unwrap(),
        address: "localhost".to_owned(),
        probe_kind: ProbeKind::Http,
        timeout: Duration::from_secs(10),
        sample_count: 1,
        params: HashMap::new(),
    }
}

fn make_context() -> RoundContext {
    RoundContext {
        observer_id: ObserverId::new("test-observer").unwrap(),
        scheduled_at: Duration::from_secs(0),
        deadline: Duration::from_secs(10),
    }
}

#[tokio::test]
async fn http_probe_success_200() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = stream.read(&mut buf).await.unwrap();
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .await;
        let _ = stream.shutdown().await;
    });

    let config = HttpProbeConfig {
        url: format!("http://127.0.0.1:{}", port),
        expected_status: Some(200),
        ..Default::default()
    };
    let probe = HttpProbe::new(config).unwrap();
    let result = probe
        .execute_round(make_context(), make_check())
        .await
        .unwrap();

    assert_eq!(result.results.len(), 1);
    assert_eq!(result.results[0].outcome, SampleOutcome::Success);
    server.abort();
}

#[tokio::test]
async fn http_probe_unexpected_status() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = stream.read(&mut buf).await;
        let _ = stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n")
            .await;
        let _ = stream.shutdown().await;
    });

    let config = HttpProbeConfig {
        url: format!("http://127.0.0.1:{}", port),
        expected_status: Some(200),
        ..Default::default()
    };
    let probe = HttpProbe::new(config).unwrap();
    let result = probe
        .execute_round(make_context(), make_check())
        .await
        .unwrap();

    assert_eq!(result.results.len(), 1);
    assert_eq!(result.results[0].outcome, SampleOutcome::UnexpectedResponse);
    server.abort();
}

#[tokio::test]
async fn http_probe_connection_refused() {
    let config = HttpProbeConfig {
        url: "http://127.0.0.1:1".to_owned(),
        ..Default::default()
    };
    let probe = HttpProbe::new(config).unwrap();
    let result = probe
        .execute_round(make_context(), make_check())
        .await
        .unwrap();

    assert_eq!(result.results.len(), 1);
    assert_ne!(result.results[0].outcome, SampleOutcome::Success);
}

#[tokio::test]
async fn http_probe_timeout() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_secs(30)).await;
        drop(stream);
    });

    let config = HttpProbeConfig {
        url: format!("http://127.0.0.1:{}", port),
        ..Default::default()
    };
    let probe = HttpProbe::new(config).unwrap();
    let mut check = make_check();
    check.timeout = Duration::from_millis(100);

    let result = tokio::time::timeout(
        Duration::from_secs(2),
        probe.execute_round(make_context(), check),
    )
    .await
    .unwrap()
    .unwrap();

    assert_eq!(result.results.len(), 1);
    assert_eq!(result.results[0].outcome, SampleOutcome::Timeout);
    server.abort();
}

#[tokio::test]
async fn http_probe_status_range() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = stream.read(&mut buf).await;
        let _ = stream
            .write_all(b"HTTP/1.1 301 Moved\r\nContent-Length: 0\r\n\r\n")
            .await;
        let _ = stream.shutdown().await;
    });

    let config = HttpProbeConfig {
        url: format!("http://127.0.0.1:{}", port),
        expected_status_range: Some((200, 399)),
        follow_redirects: false,
        ..Default::default()
    };
    let probe = HttpProbe::new(config).unwrap();
    let result = probe
        .execute_round(make_context(), make_check())
        .await
        .unwrap();

    assert_eq!(result.results[0].outcome, SampleOutcome::Success);
    server.abort();
}

#[tokio::test]
async fn http_probe_connection_mode_per_round() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = stream.read(&mut buf).await;
        let _ = stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .await;
        let _ = stream.shutdown().await;
    });

    let config = HttpProbeConfig {
        url: format!("http://127.0.0.1:{}", port),
        connection_mode: HttpConnectionMode::PerRound,
        ..Default::default()
    };
    let probe = HttpProbe::new(config).unwrap();
    let result = probe
        .execute_round(make_context(), make_check())
        .await
        .unwrap();

    assert_eq!(result.results[0].outcome, SampleOutcome::Success);
    server.abort();
}

#[tokio::test]
async fn http_probe_uses_resolved_check_request() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut request = vec![0u8; 4096];
        let count = stream.read(&mut request).await.unwrap();
        let request = String::from_utf8_lossy(&request[..count]);
        assert!(request.starts_with("POST /configured HTTP/1.1"));
        assert!(request.to_ascii_lowercase().contains("x-kemuri: yes"));
        assert!(request.ends_with("probe-body"));
        stream
            .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
            .await
            .unwrap();
    });

    let probe = HttpProbe::new(HttpProbeConfig::default()).unwrap();
    let mut check = make_check();
    check.params.insert(
        "url".to_owned(),
        format!("http://127.0.0.1:{port}/configured"),
    );
    check.params.insert("method".to_owned(), "POST".to_owned());
    check
        .params
        .insert("expected_status".to_owned(), "204".to_owned());
    check
        .params
        .insert("headers".to_owned(), r#"{"X-Kemuri":"yes"}"#.to_owned());
    check
        .params
        .insert("body".to_owned(), "probe-body".to_owned());

    let result = probe.execute_round(make_context(), check).await.unwrap();
    assert_eq!(result.results[0].outcome, SampleOutcome::Success);
    server.await.unwrap();
}
