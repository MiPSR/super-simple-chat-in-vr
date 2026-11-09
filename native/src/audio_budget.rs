//! Aggregate owned compressed-clip memory and decoder count, including idle clips.
use std::sync::Mutex;

const MAX_BYTES: usize = 128 * 1024 * 1024;
const MAX_CLIPS: usize = 128;
#[derive(Default)]
struct Usage { bytes: usize, clips: usize }
static USAGE: Mutex<Usage> = Mutex::new(Usage { bytes: 0, clips: 0 });

pub struct Lease { bytes: usize }
impl Lease {
    pub fn reserve(bytes: usize) -> Result<Self, &'static str> {
        let mut usage = USAGE.lock().unwrap();
        if usage.clips >= MAX_CLIPS || bytes > MAX_BYTES.saturating_sub(usage.bytes) {
            return Err("Client packaged-audio budget exceeded (128 clips / 128 MiB)");
        }
        usage.bytes += bytes;
        usage.clips += 1;
        Ok(Self { bytes })
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let mut usage = USAGE.lock().unwrap();
        usage.bytes -= self.bytes;
        usage.clips -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aggregate_audio_capacity_is_reserved_before_copy_and_released() {
        let lease = Lease::reserve(MAX_BYTES).unwrap();
        assert!(Lease::reserve(1).is_err());
        drop(lease);
        let leases: Vec<_> = (0..MAX_CLIPS).map(|_| Lease::reserve(0).unwrap()).collect();
        assert!(Lease::reserve(0).is_err());
        drop(leases);
        assert!(Lease::reserve(MAX_BYTES).is_ok());
    }
}
