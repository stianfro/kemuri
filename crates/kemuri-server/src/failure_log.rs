use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

const REPORT_INTERVAL: Duration = Duration::from_secs(60);

static FAILURE_LOGS: LazyLock<FailureLogLimiter> =
    LazyLock::new(|| FailureLogLimiter::new(REPORT_INTERVAL));

struct FailureEntry {
    last_reported: Instant,
    suppressed: u64,
}

pub(crate) struct FailureLogLimiter {
    report_interval: Duration,
    entries: Mutex<HashMap<(&'static str, String), FailureEntry>>,
}

impl FailureLogLimiter {
    fn new(report_interval: Duration) -> Self {
        Self {
            report_interval,
            entries: Mutex::new(HashMap::new()),
        }
    }

    fn failure(&self, component: &'static str, class: &str) -> Option<u64> {
        let now = Instant::now();
        let mut entries = self.entries.lock().unwrap();
        let key = (component, class.to_owned());

        match entries.get_mut(&key) {
            None => {
                entries.insert(
                    key,
                    FailureEntry {
                        last_reported: now,
                        suppressed: 0,
                    },
                );
                Some(0)
            }
            Some(entry) if now.duration_since(entry.last_reported) >= self.report_interval => {
                let suppressed = entry.suppressed;
                entry.last_reported = now;
                entry.suppressed = 0;
                Some(suppressed)
            }
            Some(entry) => {
                entry.suppressed += 1;
                None
            }
        }
    }

    fn recovery(&self, component: &'static str, class: &str) -> Option<u64> {
        self.entries
            .lock()
            .unwrap()
            .remove(&(component, class.to_owned()))
            .map(|entry| entry.suppressed)
    }
}

pub(crate) fn failure(component: &'static str, class: &str) -> Option<u64> {
    FAILURE_LOGS.failure(component, class)
}

pub(crate) fn recovery(component: &'static str, class: &str) -> Option<u64> {
    FAILURE_LOGS.recovery(component, class)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suppresses_repeated_failures_and_reports_on_recovery() {
        let limiter = FailureLogLimiter::new(Duration::from_secs(60));

        assert_eq!(limiter.failure("writer", "database"), Some(0));
        assert_eq!(limiter.failure("writer", "database"), None);
        assert_eq!(limiter.failure("writer", "database"), None);
        assert_eq!(limiter.recovery("writer", "database"), Some(2));
        assert_eq!(limiter.recovery("writer", "database"), None);
        assert_eq!(limiter.failure("writer", "database"), Some(0));
    }

    #[test]
    fn keeps_components_and_error_classes_independent() {
        let limiter = FailureLogLimiter::new(Duration::from_secs(60));

        assert_eq!(limiter.failure("writer", "database"), Some(0));
        assert_eq!(limiter.failure("rollup", "database"), Some(0));
        assert_eq!(limiter.failure("writer", "queue"), Some(0));
        assert_eq!(limiter.failure("writer", "database"), None);
    }
}
