//! Per-peer scheduling only. Never changes simulation ticks or reliable events.
use std::time::{Duration,Instant};

pub fn recently_changed(full: bool, clock: u64, changed: u64, tps: u32) -> bool {
    full || clock.saturating_sub(changed)<=u64::from(tps)
}

#[derive(Clone, Copy, Debug)]
pub struct Policy {
    pub minimum_hz: u32,
    pub maximum_hz: u32,
    pub automatic: bool,
    pub suppress_unchanged: bool,
}

impl Policy {
    pub fn validate(self, tps: u32) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.minimum_hz >= 1 && self.minimum_hz <= tps
                && self.maximum_hz >= self.minimum_hz && self.maximum_hz <= 120,
            "Network rates must satisfy 1 <= minimum <= TPS and minimum <= maximum <= 120"
        );
        anyhow::ensure!(self.bounds(tps).0 <= self.bounds(tps).1,
            "No integer tick divisor satisfies the configured network rates");
        Ok(())
    }

    fn bounds(self, tps: u32) -> (u32, u32) {
        (tps.div_ceil(self.maximum_hz), tps / self.minimum_hz)
    }

    pub fn clamp(self, tps: u32, requested: u32) -> u32 {
        let (minimum, maximum) = self.bounds(tps);
        requested.clamp(minimum, maximum)
    }
}

pub struct Rates {
    requested_send: u32,
    requested_receive: u32,
    pub send: u32,
    pub receive: u32,
    poor: u8,
    healthy: u8,
    input_next: [Option<Instant>;3],
}

impl Rates {
    pub fn new(policy: Policy, tps: u32, send: u32, receive: u32) -> Self {
        let send = policy.clamp(tps, send);
        let receive = policy.clamp(tps, receive);
        Self { requested_send: send, requested_receive: receive,
            send, receive, poor: 0, healthy: 0, input_next:[None;3] }
    }
    pub fn allow_input(&mut self, kind: u8, tps: u32, now: Instant) -> bool {
        let slot=match kind {3=>0,12=>1,14=>2,_=>return true};
        let interval=Duration::from_secs_f64(self.send as f64/tps as f64);
        let next=self.input_next[slot].unwrap_or(now);
        let tolerance=Duration::from_millis(1).min(interval/10);
        if next>now+interval+tolerance {return false;}
        self.input_next[slot]=Some(next.max(now)+interval);
        true
    }

    /// Call once every five seconds. Hysteresis avoids reacting to a single
    /// delayed packet; recovering never overrides an explicit lower-rate choice.
    pub fn observe(&mut self, policy: Policy, tps: u32, rtt: Duration, loss: f64) -> bool {
        if !policy.automatic {
            return false;
        }
        let before = (self.send, self.receive);
        if rtt >= Duration::from_millis(350) || loss >= 0.08 {
            self.poor = self.poor.saturating_add(1);
            self.healthy = 0;
            if self.poor >= 3 {
                self.send = policy.clamp(tps, self.send.saturating_mul(2));
                self.receive = policy.clamp(tps, self.receive.saturating_mul(2));
                self.poor = 0;
            }
        } else if rtt <= Duration::from_millis(180) && loss <= 0.02 {
            self.healthy = self.healthy.saturating_add(1);
            self.poor = 0;
            if self.healthy >= 6 {
                self.send = policy.clamp(tps, self.send.div_ceil(2).max(self.requested_send));
                self.receive = policy.clamp(tps, self.receive.div_ceil(2).max(self.requested_receive));
                self.healthy = 0;
            }
        } else {
            self.poor = 0;
            self.healthy = 0;
        }
        let changed=before != (self.send,self.receive);
        if changed {self.input_next=[None;3];}
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn policy() -> Policy {
        Policy { minimum_hz: 5, maximum_hz: 60, automatic: true, suppress_unchanged: false }
    }
    #[test]
    fn asymmetric_rates_adapt_and_recover_without_overriding_user_limits() {
        let mut rates = Rates::new(policy(), 60, 1, 4);
        for _ in 0..2 {
            assert!(!rates.observe(policy(),60,Duration::from_millis(500),0.1));
        }
        assert!(rates.observe(policy(),60,Duration::from_millis(500),0.1));
        assert_eq!((rates.send,rates.receive),(2,8));
        for _ in 0..60 { rates.observe(policy(),60,Duration::from_millis(500),0.1); }
        assert_eq!((rates.send,rates.receive),(12,12));
        for _ in 0..60 { rates.observe(policy(),60,Duration::from_millis(40),0.0); }
        assert_eq!((rates.send,rates.receive),(1,4));
        let mut disabled = policy();
        disabled.automatic = false;
        assert!(!rates.observe(disabled,60,Duration::from_secs(2),1.0));
        assert_eq!((rates.send,rates.receive),(1,4));
    }
    #[test]
    fn nondefault_tps_and_impossible_integer_schedules() {
        let policy = Policy { minimum_hz: 5, maximum_hz: 20, ..policy() };
        assert!(policy.validate(90).is_ok());
        let rates = Rates::new(policy,90,1,4);
        assert_eq!((rates.send,rates.receive),(5,5));
        assert!(Policy { minimum_hz: 40, maximum_hz: 40, ..policy }.validate(60).is_err());
        assert!(Policy { minimum_hz: 0, ..policy }.validate(60).is_err());
    }
    #[test]
    fn input_pacing_bounds_pose_tracking_and_controls_without_jitter_halving() {
        let mut rates=Rates::new(policy(),60,1,4);
        let now=Instant::now();
        assert!(rates.allow_input(3,60,now));
        assert!(rates.allow_input(3,60,now));
        assert!(!rates.allow_input(3,60,now));
        assert!(rates.allow_input(14,60,now));
        assert!(rates.allow_input(12,60,now));
        for index in 1..60 {
            assert!(rates.allow_input(3,60,now+Duration::from_secs_f64(index as f64/60.0)));
        }
        assert!(!rates.allow_input(3,60,now+Duration::from_secs_f64(59.0/60.0)));
    }
    #[test]
    fn motion_aggregation_repeats_recent_changes_until_full_keyframe() {
        assert!(recently_changed(false,100,100,60));
        assert!(recently_changed(false,160,100,60));
        assert!(!recently_changed(false,161,100,60));
        assert!(recently_changed(true,1000,100,60));
    }
}
