use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use kemuri_config::{ResolvedHttpParams, ResolvedProbeParams, SchedulerConfig};
use kemuri_core::{CheckId, CheckRevisionId, ProfileId, RealClock, TargetId};
use kemuri_server::Scheduler;

fn check(index: usize) -> kemuri_config::ResolvedCheckDef {
    kemuri_config::ResolvedCheckDef {
        check_id: CheckId::new(format!("check-{index}")).unwrap(),
        target_id: TargetId::new(format!("target-{index}")).unwrap(),
        target_address: "127.0.0.1".to_owned(),
        profile_id: ProfileId::new("http").unwrap(),
        probe_kind: kemuri_core::ProbeKind::Http,
        interval: Duration::from_secs(60),
        timeout: Duration::from_secs(5),
        revision_id: CheckRevisionId::new(format!("revision-{index}")).unwrap(),
        probe_params: ResolvedProbeParams::Http(ResolvedHttpParams {
            url: "http://127.0.0.1".to_owned(),
            method: None,
            headers: Default::default(),
            expected_status: Some(200),
            expected_status_range: None,
            body: None,
            follow_redirects: true,
            max_redirect_count: 10,
            connection_mode: "pooled".to_owned(),
            measure_until: "headers".to_owned(),
            user_agent: None,
            tls_validate: true,
            root_certificates: Vec::new(),
        }),
    }
}

#[tokio::test]
async fn five_hundred_checks_dispatch_once_without_starvation() {
    let config = SchedulerConfig {
        max_concurrent: 500,
        tick_interval: "1ms".to_owned(),
        default_jitter: "0%".to_owned(),
        ..SchedulerConfig::default()
    };
    let checks: Vec<_> = (0..500).map(check).collect();
    let (job_tx, mut job_rx) = tokio::sync::mpsc::channel(500);
    let (result_tx, _result_rx) = tokio::sync::mpsc::channel(500);
    let mut scheduler = Scheduler::new(config, checks, job_tx, result_tx, Arc::new(RealClock));
    scheduler.start();

    let jobs = tokio::time::timeout(Duration::from_secs(5), async {
        let mut jobs = Vec::with_capacity(500);
        while jobs.len() < 500 {
            jobs.push(job_rx.recv().await.expect("scheduler queue closed"));
        }
        jobs
    })
    .await
    .expect("not all checks were dispatched");
    scheduler.stop().await;

    let unique: HashSet<_> = jobs
        .iter()
        .map(|job| (job.target_id.clone(), job.check_id.clone()))
        .collect();
    assert_eq!(jobs.len(), 500);
    assert_eq!(unique.len(), 500);
}

#[tokio::test]
async fn skipped_rounds_survive_result_queue_backpressure() {
    let config = SchedulerConfig {
        max_concurrent: 1,
        tick_interval: "1ms".to_owned(),
        default_jitter: "0%".to_owned(),
        ..SchedulerConfig::default()
    };
    let checks: Vec<_> = (0..100).map(check).collect();
    let (job_tx, mut job_rx) = tokio::sync::mpsc::channel(100);
    let (result_tx, mut result_rx) = tokio::sync::mpsc::channel(1);
    let mut scheduler = Scheduler::new(config, checks, job_tx, result_tx, Arc::new(RealClock));
    scheduler.start();

    tokio::time::timeout(Duration::from_secs(5), job_rx.recv())
        .await
        .expect("first job was not dispatched")
        .expect("scheduler queue closed");

    let skipped = tokio::time::timeout(Duration::from_secs(5), async {
        let mut results = Vec::with_capacity(99);
        while results.len() < 99 {
            results.push(
                result_rx
                    .recv()
                    .await
                    .expect("scheduler result queue closed"),
            );
        }
        results
    })
    .await
    .expect("skipped rounds were lost under result queue backpressure");
    scheduler.stop().await;

    let unique: HashSet<_> = skipped
        .iter()
        .map(|result| (result.target_id.clone(), result.check_id.clone()))
        .collect();
    assert_eq!(skipped.len(), 99);
    assert_eq!(unique.len(), 99);
    assert!(skipped.iter().all(|result| {
        result.execution_status == kemuri_core::RoundExecutionStatus::SkippedBackpressure
    }));
}
