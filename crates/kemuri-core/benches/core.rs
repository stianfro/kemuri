use criterion::{Criterion, criterion_group, criterion_main};
use kemuri_core::{Histogram, SampleClassification, SampleOutcome, SampleRecord, encode_samples};

fn sample_encoding(c: &mut Criterion) {
    let samples: Vec<SampleRecord> = (0..100)
        .map(|index| SampleRecord {
            sample_index: index,
            offset_us: u32::from(index) * 1_000,
            outcome: SampleOutcome::Success,
            classification: SampleClassification::HealthyResponse,
            latency_ns: Some(10_000_000 + u64::from(index)),
            elapsed_ns: Some(10_000_000 + u64::from(index)),
            metadata: None,
        })
        .collect();
    c.bench_function("encode_100_samples", |bencher| {
        bencher.iter(|| encode_samples(&samples))
    });
}

fn histogram_aggregation(c: &mut Criterion) {
    c.bench_function("aggregate_1000_latencies", |bencher| {
        bencher.iter(|| {
            let mut histogram = Histogram::new();
            for latency in 1..=1_000_u64 {
                histogram.record(latency * 100_000);
            }
            histogram
        })
    });
}

criterion_group!(benches, sample_encoding, histogram_aggregation);
criterion_main!(benches);
