//! One monotonic budget includes attempts, response bodies and retry backoff.
use super::{BackendError, error, retry};
use std::time::{Duration, Instant};
pub(super) trait Clock {
    fn now(&self) -> Duration;
    fn sleep(&self, duration: Duration);
}
pub(super) struct SystemClock(Instant);
impl Default for SystemClock {
    fn default() -> Self {
        Self(Instant::now())
    }
}
impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}
pub(super) struct Attempt<T> {
    pub result: T,
    pub retry: bool,
    pub retry_after: Option<u64>,
}
pub(super) fn run<T>(
    deadline: Duration,
    timeout: Duration,
    clock: &dyn Clock,
    mut attempt: impl FnMut(u32, Duration) -> Result<Attempt<T>, BackendError>,
) -> Result<T, BackendError> {
    let start = clock.now();
    let remaining = || deadline.saturating_sub(clock.now().saturating_sub(start));
    for index in 0..retry::ATTEMPTS {
        let budget = remaining();
        if budget.is_zero() {
            return Err(error("S3 operation deadline exceeded"));
        }
        let result = attempt(index, timeout.min(budget))?;
        if remaining().is_zero() {
            return Err(error("S3 operation deadline exceeded"));
        }
        if !result.retry || index + 1 == retry::ATTEMPTS {
            return Ok(result.result);
        }
        clock.sleep(retry::delay(index, result.retry_after)?.min(remaining()));
    }
    unreachable!("last attempt always returns")
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    struct FakeClock(Cell<Duration>);
    impl Clock for FakeClock {
        fn now(&self) -> Duration {
            self.0.get()
        }
        fn sleep(&self, duration: Duration) {
            self.0.set(self.now() + duration);
        }
    }
    #[test]
    fn retry_loop_caps_attempt_timeout_and_stops_at_deadline() {
        let clock = FakeClock(Cell::new(Duration::ZERO));
        let mut attempts = Vec::new();
        let result = run(
            Duration::from_secs(3),
            Duration::from_secs(2),
            &clock,
            |_, timeout| {
                attempts.push(timeout);
                clock.sleep(timeout);
                Ok(Attempt {
                    result: (),
                    retry: true,
                    retry_after: None,
                })
            },
        );
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("deadline exceeded")
        );
        assert_eq!(clock.now(), Duration::from_secs(3));
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0], Duration::from_secs(2));
        assert!(attempts[1] < Duration::from_secs(1));
    }
    #[test]
    fn backoff_cannot_start_an_attempt_after_deadline() {
        let clock = FakeClock(Cell::new(Duration::ZERO));
        let mut attempts = 0;
        let result = run(
            Duration::from_millis(20),
            Duration::from_secs(2),
            &clock,
            |_, _| {
                attempts += 1;
                Ok(Attempt {
                    result: (),
                    retry: true,
                    retry_after: Some(1),
                })
            },
        );
        assert!(result.is_err());
        assert_eq!(attempts, 1);
        assert_eq!(clock.now(), Duration::from_millis(20));
    }
}
