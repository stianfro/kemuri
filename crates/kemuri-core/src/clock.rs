use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
    fn system_time(&self) -> SystemTime;
}

pub struct RealClock;

impl Clock for RealClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn system_time(&self) -> SystemTime {
        SystemTime::now()
    }
}

pub struct FakeClock {
    origin: Instant,
    system_origin: SystemTime,
    offset: Mutex<Duration>,
}

impl FakeClock {
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
            system_origin: SystemTime::now(),
            offset: Mutex::new(Duration::ZERO),
        }
    }

    pub fn advance(&self, duration: Duration) {
        let mut offset = self.offset.lock().expect("fake clock lock poisoned");
        *offset += duration;
    }
}

impl Default for FakeClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for FakeClock {
    fn now(&self) -> Instant {
        let offset = self.offset.lock().expect("fake clock lock poisoned");
        self.origin + *offset
    }

    fn system_time(&self) -> SystemTime {
        let offset = self.offset.lock().expect("fake clock lock poisoned");
        self.system_origin + *offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_clock_advances() {
        let clock = RealClock;
        let t1 = clock.now();
        let t2 = clock.now();
        assert!(t2 >= t1);
    }

    #[test]
    fn fake_clock_advance() {
        let clock = FakeClock::new();
        let t1 = clock.now();
        clock.advance(Duration::from_millis(100));
        let t2 = clock.now();
        assert!(t2.duration_since(t1) >= Duration::from_millis(100));
    }

    #[test]
    fn fake_clock_system_time() {
        let clock = FakeClock::new();
        let t1 = clock.system_time();
        clock.advance(Duration::from_secs(60));
        let t2 = clock.system_time();
        assert!(t2.duration_since(t1).unwrap() >= Duration::from_secs(60));
    }
}
