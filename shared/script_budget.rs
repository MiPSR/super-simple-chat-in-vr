//! Execution budgets must not charge unrelated scheduler stalls to a creator.
//! Linux thread CPU time bounds script work; wall time is a second backstop.
use std::time::{Duration, Instant};

pub struct ScriptBudget {
    wall: Instant,
    cpu: Option<Duration>,
    limit: Duration,
}
impl ScriptBudget {
    pub fn new(milliseconds: u64) -> Self {
        Self {
            wall: Instant::now(),
            cpu: thread_cpu(),
            limit: Duration::from_millis(milliseconds),
        }
    }
    pub fn reset(&mut self) {
        self.wall = Instant::now();
        self.cpu = thread_cpu();
    }
    pub fn expired(&self) -> bool {
        if self.wall.elapsed() >= Duration::from_millis(250) {
            return true;
        }
        match self.cpu.zip(thread_cpu()) {
            Some((start, now)) => now.checked_sub(start).is_none_or(|used| used >= self.limit),
            None => self.wall.elapsed() >= self.limit,
        }
    }
}
fn thread_cpu() -> Option<Duration> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: a valid timespec is written by the current-thread clock only.
    if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) } != 0 {
        return None;
    }
    Some(Duration::new(
        time.tv_sec.try_into().ok()?,
        time.tv_nsec.try_into().ok()?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sleeping_does_not_consume_execution_budget() {
        let budget = ScriptBudget::new(5);
        assert!(budget.cpu.is_some());
        std::thread::sleep(Duration::from_millis(25));
        assert!(!budget.expired());
    }
    #[test]
    fn work_expires_and_reset_restarts_budget() {
        let mut budget = ScriptBudget::new(2);
        while !budget.expired() {
            std::hint::spin_loop();
        }
        budget.reset();
        assert!(!budget.expired());
        budget.wall = Instant::now() - Duration::from_millis(251);
        assert!(budget.expired());
    }
}
