use serde::{Deserialize, Serialize};

use crate::{SampleClassification, SampleOutcome};

const ENCODING_VERSION: u8 = 1;
const FLAG_HAS_LATENCY: u8 = 0x01;
const FLAG_HAS_ELAPSED: u8 = 0x02;
const FLAG_HAS_METADATA: u8 = 0x04;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SampleRecord {
    pub sample_index: u16,
    pub offset_us: u32,
    pub outcome: SampleOutcome,
    pub classification: SampleClassification,
    pub latency_ns: Option<u64>,
    pub elapsed_ns: Option<u64>,
    pub metadata: Option<Vec<u8>>,
}

impl SampleOutcome {
    fn to_discriminant(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::Timeout => 1,
            Self::DnsError => 2,
            Self::NetworkUnreachable => 3,
            Self::ConnectionRefused => 4,
            Self::ConnectionReset => 5,
            Self::TlsError => 6,
            Self::ProtocolError => 7,
            Self::UnexpectedResponse => 8,
            Self::PermissionDenied => 9,
            Self::Cancelled => 10,
            Self::InternalError => 11,
        }
    }

    fn from_discriminant(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Success),
            1 => Some(Self::Timeout),
            2 => Some(Self::DnsError),
            3 => Some(Self::NetworkUnreachable),
            4 => Some(Self::ConnectionRefused),
            5 => Some(Self::ConnectionReset),
            6 => Some(Self::TlsError),
            7 => Some(Self::ProtocolError),
            8 => Some(Self::UnexpectedResponse),
            9 => Some(Self::PermissionDenied),
            10 => Some(Self::Cancelled),
            11 => Some(Self::InternalError),
            _ => None,
        }
    }
}

impl SampleClassification {
    fn to_discriminant(self) -> u8 {
        match self {
            Self::HealthyResponse => 0,
            Self::UnhealthyResponse => 1,
            Self::MeasurementLoss => 2,
        }
    }

    fn from_discriminant(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::HealthyResponse),
            1 => Some(Self::UnhealthyResponse),
            2 => Some(Self::MeasurementLoss),
            _ => None,
        }
    }
}

pub fn encode_samples(records: &[SampleRecord]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.push(ENCODING_VERSION);
    buf.extend_from_slice(&(records.len() as u16).to_le_bytes());
    for record in records {
        buf.extend_from_slice(&record.sample_index.to_le_bytes());
        buf.extend_from_slice(&record.offset_us.to_le_bytes());
        buf.push(record.outcome.to_discriminant());
        buf.push(record.classification.to_discriminant());
        let mut flags: u8 = 0;
        if record.latency_ns.is_some() {
            flags |= FLAG_HAS_LATENCY;
        }
        if record.elapsed_ns.is_some() {
            flags |= FLAG_HAS_ELAPSED;
        }
        if record.metadata.is_some() {
            flags |= FLAG_HAS_METADATA;
        }
        buf.push(flags);
        if let Some(lat) = record.latency_ns {
            buf.extend_from_slice(&lat.to_le_bytes());
        }
        if let Some(elapsed) = record.elapsed_ns {
            buf.extend_from_slice(&elapsed.to_le_bytes());
        }
        if let Some(ref meta) = record.metadata {
            buf.extend_from_slice(&(meta.len() as u16).to_le_bytes());
            buf.extend_from_slice(meta);
        }
    }
    buf
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SampleBlobError {
    #[error("unsupported encoding version: {0}")]
    UnsupportedVersion(u8),
    #[error("invalid outcome discriminant: {0}")]
    InvalidOutcome(u8),
    #[error("invalid classification discriminant: {0}")]
    InvalidClassification(u8),
    #[error("truncated blob data")]
    Truncated,
}

pub fn decode_samples(data: &[u8]) -> Result<Vec<SampleRecord>, SampleBlobError> {
    if data.is_empty() {
        return Err(SampleBlobError::Truncated);
    }
    let version = data[0];
    if version != ENCODING_VERSION {
        return Err(SampleBlobError::UnsupportedVersion(version));
    }
    if data.len() < 3 {
        return Err(SampleBlobError::Truncated);
    }
    let count = u16::from_le_bytes([data[1], data[2]]) as usize;
    let mut offset = 3;
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        if offset + 9 > data.len() {
            return Err(SampleBlobError::Truncated);
        }
        let sample_index = u16::from_le_bytes([data[offset], data[offset + 1]]);
        offset += 2;
        let offset_us = u32::from_le_bytes([
            data[offset],
            data[offset + 1],
            data[offset + 2],
            data[offset + 3],
        ]);
        offset += 4;
        let outcome_disc = data[offset];
        offset += 1;
        let class_disc = data[offset];
        offset += 1;
        let flags = data[offset];
        offset += 1;
        let outcome = SampleOutcome::from_discriminant(outcome_disc)
            .ok_or(SampleBlobError::InvalidOutcome(outcome_disc))?;
        let classification = SampleClassification::from_discriminant(class_disc)
            .ok_or(SampleBlobError::InvalidClassification(class_disc))?;
        let mut latency_ns = None;
        let mut elapsed_ns = None;
        let mut metadata = None;
        if flags & FLAG_HAS_LATENCY != 0 {
            if offset + 8 > data.len() {
                return Err(SampleBlobError::Truncated);
            }
            latency_ns = Some(u64::from_le_bytes(
                data[offset..offset + 8]
                    .try_into()
                    .map_err(|_| SampleBlobError::Truncated)?,
            ));
            offset += 8;
        }
        if flags & FLAG_HAS_ELAPSED != 0 {
            if offset + 8 > data.len() {
                return Err(SampleBlobError::Truncated);
            }
            elapsed_ns = Some(u64::from_le_bytes(
                data[offset..offset + 8]
                    .try_into()
                    .map_err(|_| SampleBlobError::Truncated)?,
            ));
            offset += 8;
        }
        if flags & FLAG_HAS_METADATA != 0 {
            if offset + 2 > data.len() {
                return Err(SampleBlobError::Truncated);
            }
            let meta_len = u16::from_le_bytes([data[offset], data[offset + 1]]) as usize;
            offset += 2;
            if offset + meta_len > data.len() {
                return Err(SampleBlobError::Truncated);
            }
            metadata = Some(data[offset..offset + meta_len].to_vec());
            offset += meta_len;
        }
        records.push(SampleRecord {
            sample_index,
            offset_us,
            outcome,
            classification,
            latency_ns,
            elapsed_ns,
            metadata,
        });
    }
    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_record(
        index: u16,
        offset: u32,
        outcome: SampleOutcome,
        classification: SampleClassification,
    ) -> SampleRecord {
        SampleRecord {
            sample_index: index,
            offset_us: offset,
            outcome,
            classification,
            latency_ns: None,
            elapsed_ns: None,
            metadata: None,
        }
    }

    #[test]
    fn empty_records() {
        let records: Vec<SampleRecord> = vec![];
        let encoded = encode_samples(&records);
        let decoded = decode_samples(&encoded).unwrap();
        assert!(decoded.is_empty());
    }

    #[test]
    fn roundtrip_minimal() {
        let records = vec![make_record(
            0,
            0,
            SampleOutcome::Success,
            SampleClassification::HealthyResponse,
        )];
        let encoded = encode_samples(&records);
        let decoded = decode_samples(&encoded).unwrap();
        assert_eq!(records, decoded);
    }

    #[test]
    fn roundtrip_with_latency() {
        let mut rec = make_record(
            0,
            100,
            SampleOutcome::Success,
            SampleClassification::HealthyResponse,
        );
        rec.latency_ns = Some(1_500_000);
        let records = vec![rec];
        let encoded = encode_samples(&records);
        let decoded = decode_samples(&encoded).unwrap();
        assert_eq!(records, decoded);
    }

    #[test]
    fn roundtrip_with_all_fields() {
        let mut rec = make_record(
            5,
            2500,
            SampleOutcome::Timeout,
            SampleClassification::MeasurementLoss,
        );
        rec.latency_ns = Some(5_000_000_000);
        rec.elapsed_ns = Some(5_100_000_000);
        rec.metadata = Some(vec![0xDE, 0xAD, 0xBE, 0xEF]);
        let records = vec![rec];
        let encoded = encode_samples(&records);
        let decoded = decode_samples(&encoded).unwrap();
        assert_eq!(records, decoded);
    }

    #[test]
    fn roundtrip_multiple_records() {
        let mut rec1 = make_record(
            0,
            0,
            SampleOutcome::Success,
            SampleClassification::HealthyResponse,
        );
        rec1.latency_ns = Some(1_000_000);
        let mut rec2 = make_record(
            1,
            1000,
            SampleOutcome::ConnectionRefused,
            SampleClassification::UnhealthyResponse,
        );
        rec2.elapsed_ns = Some(500_000);
        let rec3 = make_record(
            2,
            2000,
            SampleOutcome::DnsError,
            SampleClassification::MeasurementLoss,
        );
        let records = vec![rec1, rec2, rec3];
        let encoded = encode_samples(&records);
        let decoded = decode_samples(&encoded).unwrap();
        assert_eq!(records, decoded);
    }

    #[test]
    fn reject_wrong_version() {
        let mut encoded = encode_samples(&[make_record(
            0,
            0,
            SampleOutcome::Success,
            SampleClassification::HealthyResponse,
        )]);
        encoded[0] = 99;
        assert!(matches!(
            decode_samples(&encoded),
            Err(SampleBlobError::UnsupportedVersion(99))
        ));
    }

    #[test]
    fn reject_truncated() {
        let encoded = encode_samples(&[make_record(
            0,
            0,
            SampleOutcome::Success,
            SampleClassification::HealthyResponse,
        )]);
        assert!(matches!(
            decode_samples(&encoded[..2]),
            Err(SampleBlobError::Truncated)
        ));
    }

    #[test]
    fn all_outcome_variants() {
        let outcomes = [
            SampleOutcome::Success,
            SampleOutcome::Timeout,
            SampleOutcome::DnsError,
            SampleOutcome::NetworkUnreachable,
            SampleOutcome::ConnectionRefused,
            SampleOutcome::ConnectionReset,
            SampleOutcome::TlsError,
            SampleOutcome::ProtocolError,
            SampleOutcome::UnexpectedResponse,
            SampleOutcome::PermissionDenied,
            SampleOutcome::Cancelled,
            SampleOutcome::InternalError,
        ];
        for (i, outcome) in outcomes.into_iter().enumerate() {
            let rec = make_record(i as u16, 0, outcome, SampleClassification::HealthyResponse);
            let encoded = encode_samples(std::slice::from_ref(&rec));
            let decoded = decode_samples(&encoded).unwrap();
            assert_eq!(rec, decoded[0]);
        }
    }

    #[test]
    fn forward_compatibility_extra_bytes() {
        let mut encoded = encode_samples(&[make_record(
            0,
            0,
            SampleOutcome::Success,
            SampleClassification::HealthyResponse,
        )]);
        encoded.extend_from_slice(&[0xAA, 0xBB]);
        let decoded = decode_samples(&encoded).unwrap();
        assert_eq!(decoded.len(), 1);
    }

    #[test]
    fn deterministic_encoding() {
        let rec = make_record(
            0,
            0,
            SampleOutcome::Success,
            SampleClassification::HealthyResponse,
        );
        let encoded1 = encode_samples(std::slice::from_ref(&rec));
        let encoded2 = encode_samples(&[rec]);
        assert_eq!(encoded1, encoded2);
    }
}
