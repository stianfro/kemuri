use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleClassification {
    HealthyResponse,
    UnhealthyResponse,
    MeasurementLoss,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip() {
        let variants = [
            SampleClassification::HealthyResponse,
            SampleClassification::UnhealthyResponse,
            SampleClassification::MeasurementLoss,
        ];
        for v in variants {
            let yaml = serde_yaml::to_string(&v).unwrap();
            let parsed: SampleClassification = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(v, parsed);
        }
    }
}
