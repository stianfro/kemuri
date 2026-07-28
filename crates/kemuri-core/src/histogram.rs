const BINS_PER_DECADE: usize = 12;
const DECADES: usize = 8;
const DATA_BINS: usize = BINS_PER_DECADE * DECADES;
const NUM_BINS: usize = DATA_BINS + 2;
const UNDERFLOW_BIN: usize = 0;
const OVERFLOW_BIN: usize = DATA_BINS + 1;
const LOG10_LOWER: f64 = 3.0;
const HISTOGRAM_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Histogram {
    version: u32,
    bins: [u64; NUM_BINS],
    min: Option<u64>,
    max: Option<u64>,
    sum: u64,
    count: u64,
}

impl Histogram {
    pub fn new() -> Self {
        Self {
            version: HISTOGRAM_VERSION,
            bins: [0; NUM_BINS],
            min: None,
            max: None,
            sum: 0,
            count: 0,
        }
    }

    pub fn record(&mut self, value_ns: u64) {
        let bin = value_to_bin(value_ns);
        self.bins[bin] += 1;
        self.count += 1;
        self.sum = self.sum.saturating_add(value_ns);
        self.min = Some(self.min.map_or(value_ns, |m| m.min(value_ns)));
        self.max = Some(self.max.map_or(value_ns, |m| m.max(value_ns)));
    }

    pub fn merge(&mut self, other: &Histogram) {
        for (dst, src) in self.bins.iter_mut().zip(other.bins.iter()) {
            *dst = dst.saturating_add(*src);
        }
        self.count = self.count.saturating_add(other.count);
        self.sum = self.sum.saturating_add(other.sum);
        match (self.min, other.min) {
            (Some(a), Some(b)) => self.min = Some(a.min(b)),
            (None, Some(b)) => self.min = Some(b),
            _ => {}
        }
        match (self.max, other.max) {
            (Some(a), Some(b)) => self.max = Some(a.max(b)),
            (None, Some(b)) => self.max = Some(b),
            _ => {}
        }
    }

    pub fn quantile(&self, p: f64) -> Option<u64> {
        if self.count == 0 || !(0.0..=1.0).contains(&p) {
            return None;
        }
        if p <= 0.0 {
            return self.min;
        }
        if p >= 1.0 {
            return self.max;
        }
        let target = (p * self.count as f64).ceil() as u64;
        let mut accumulated: u64 = 0;
        for (i, &count) in self.bins.iter().enumerate() {
            accumulated += count;
            if accumulated >= target {
                return Some(bin_representative(i));
            }
        }
        self.max
    }

    pub fn min(&self) -> Option<u64> {
        self.min
    }

    pub fn max(&self) -> Option<u64> {
        self.max
    }

    pub fn sum(&self) -> u64 {
        self.sum
    }

    pub fn count(&self) -> u64 {
        self.count
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn bins(&self) -> &[u64; NUM_BINS] {
        &self.bins
    }

    pub fn underflow_count(&self) -> u64 {
        self.bins[UNDERFLOW_BIN]
    }

    pub fn overflow_count(&self) -> u64 {
        self.bins[OVERFLOW_BIN]
    }

    pub fn bin_representatives() -> Vec<u64> {
        (0..NUM_BINS).map(bin_representative).collect()
    }

    pub fn num_bins() -> usize {
        NUM_BINS
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(1 + 4 + NUM_BINS * 8 + 8 + 8 + 8);
        buf.push(self.version as u8);
        buf.extend_from_slice(&self.count.to_le_bytes());
        buf.extend_from_slice(&self.sum.to_le_bytes());
        let min_val = self.min.unwrap_or(0);
        let max_val = self.max.unwrap_or(0);
        buf.extend_from_slice(&min_val.to_le_bytes());
        buf.extend_from_slice(&max_val.to_le_bytes());
        for &count in &self.bins {
            buf.extend_from_slice(&count.to_le_bytes());
        }
        buf
    }

    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() < 1 + 8 + 8 + 8 + 8 + NUM_BINS * 8 {
            return None;
        }
        let version = data[0] as u32;
        let mut offset = 1;
        let count = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
        offset += 8;
        let sum = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
        offset += 8;
        let min_val = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
        offset += 8;
        let max_val = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
        offset += 8;
        let mut bins = [0u64; NUM_BINS];
        for bin in &mut bins {
            *bin = u64::from_le_bytes(data[offset..offset + 8].try_into().ok()?);
            offset += 8;
        }
        Some(Self {
            version,
            bins,
            min: if min_val > 0 || count > 0 {
                Some(min_val)
            } else {
                None
            },
            max: if max_val > 0 || count > 0 {
                Some(max_val)
            } else {
                None
            },
            sum,
            count,
        })
    }
}

impl Default for Histogram {
    fn default() -> Self {
        Self::new()
    }
}

fn value_to_bin(v_ns: u64) -> usize {
    if v_ns < 1000 {
        return UNDERFLOW_BIN;
    }
    if v_ns >= 100_000_000_000 {
        return OVERFLOW_BIN;
    }
    let log_val = (v_ns as f64).log10();
    let bin = ((log_val - LOG10_LOWER) * BINS_PER_DECADE as f64).floor() as usize;
    (bin + 1).min(DATA_BINS)
}

fn bin_representative(bin: usize) -> u64 {
    if bin == UNDERFLOW_BIN {
        return 500;
    }
    if bin == OVERFLOW_BIN {
        return 100_000_000_000;
    }
    let data_bin = bin - 1;
    let log_mid = LOG10_LOWER + (data_bin as f64 + 0.5) / BINS_PER_DECADE as f64;
    10f64.powf(log_mid).round() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_histogram() {
        let h = Histogram::new();
        assert_eq!(h.count(), 0);
        assert_eq!(h.min(), None);
        assert_eq!(h.max(), None);
        assert_eq!(h.sum(), 0);
        assert_eq!(h.quantile(0.5), None);
    }

    #[test]
    fn record_single_value() {
        let mut h = Histogram::new();
        h.record(1_000_000);
        assert_eq!(h.count(), 1);
        assert_eq!(h.min(), Some(1_000_000));
        assert_eq!(h.max(), Some(1_000_000));
        assert_eq!(h.sum(), 1_000_000);
    }

    #[test]
    fn underflow_bin() {
        let mut h = Histogram::new();
        h.record(100);
        assert_eq!(h.underflow_count(), 1);
        assert_eq!(h.count(), 1);
    }

    #[test]
    fn overflow_bin() {
        let mut h = Histogram::new();
        h.record(200_000_000_000);
        assert_eq!(h.overflow_count(), 1);
        assert_eq!(h.count(), 1);
    }

    #[test]
    fn boundary_1us() {
        let mut h = Histogram::new();
        h.record(1000);
        assert_eq!(h.underflow_count(), 0);
        assert_eq!(h.bins()[1], 1);
    }

    #[test]
    fn boundary_100s() {
        let mut h = Histogram::new();
        h.record(100_000_000_000);
        assert_eq!(h.overflow_count(), 1);
    }

    #[test]
    fn just_under_100s() {
        let mut h = Histogram::new();
        h.record(99_999_999_999);
        assert_eq!(h.overflow_count(), 0);
    }

    #[test]
    fn bin_placement_1ms() {
        let mut h = Histogram::new();
        h.record(1_000_000);
        assert_eq!(h.bins()[37], 1);
    }

    #[test]
    fn merge_histograms() {
        let mut h1 = Histogram::new();
        h1.record(1_000_000);
        h1.record(2_000_000);

        let mut h2 = Histogram::new();
        h2.record(5_000_000);
        h2.record(500);

        h1.merge(&h2);
        assert_eq!(h1.count(), 4);
        assert_eq!(h1.min(), Some(500));
        assert_eq!(h1.max(), Some(5_000_000));
        assert_eq!(h1.underflow_count(), 1);
    }

    #[test]
    fn quantile_median() {
        let mut h = Histogram::new();
        for v in [1_000_000, 2_000_000, 3_000_000, 10_000_000, 50_000_000] {
            h.record(v);
        }
        let median = h.quantile(0.5).unwrap();
        assert!((1_000_000..=10_000_000).contains(&median));
    }

    #[test]
    fn quantile_p0_returns_min() {
        let mut h = Histogram::new();
        h.record(5_000_000);
        h.record(10_000_000);
        assert_eq!(h.quantile(0.0), Some(5_000_000));
    }

    #[test]
    fn quantile_p100_returns_max() {
        let mut h = Histogram::new();
        h.record(5_000_000);
        h.record(10_000_000);
        assert_eq!(h.quantile(1.0), Some(10_000_000));
    }

    #[test]
    fn encode_decode_roundtrip() {
        let mut h = Histogram::new();
        for v in [
            500,
            1_000,
            10_000,
            1_000_000,
            1_000_000_000,
            150_000_000_000,
        ] {
            h.record(v);
        }
        let encoded = h.encode();
        let decoded = Histogram::decode(&encoded).unwrap();
        assert_eq!(h, decoded);
    }

    #[test]
    fn decode_truncated_data() {
        assert!(Histogram::decode(&[1, 2, 3]).is_none());
    }

    #[test]
    fn min_max_tracking() {
        let mut h = Histogram::new();
        h.record(5_000);
        h.record(50_000);
        h.record(500_000);
        assert_eq!(h.min(), Some(5_000));
        assert_eq!(h.max(), Some(500_000));
    }

    #[test]
    fn saturating_sum() {
        let mut h = Histogram::new();
        h.record(u64::MAX);
        h.record(1);
        assert_eq!(h.sum(), u64::MAX);
    }
}
