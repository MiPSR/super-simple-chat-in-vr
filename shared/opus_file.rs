//! Thin memory-only libopusfile host. No custom Ogg parser or codec, and no
//! path/network callbacks are exposed to uploaded audio or creator scripts.
use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};

pub const MAX_OPUS_BYTES: usize = 32 * 1024 * 1024;
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static RESIDENT: AtomicUsize = AtomicUsize::new(0);
struct Reservation(usize);
impl Reservation {
    fn new(bytes: usize) -> Result<Self, String> {
        if ACTIVE.fetch_add(1, Ordering::AcqRel) >= 32 {
            ACTIVE.fetch_sub(1, Ordering::AcqRel);
            return Err("At most 32 packaged audio decoders per client".into());
        }
        if RESIDENT
            .try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current
                    .checked_add(bytes)
                    .filter(|total| *total <= 64 * 1024 * 1024)
            })
            .is_err()
        {
            ACTIVE.fetch_sub(1, Ordering::AcqRel);
            return Err("Packaged audio exceeds 64 MiB resident compressed data".into());
        }
        Ok(Self(bytes))
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        RESIDENT.fetch_sub(self.0, Ordering::AcqRel);
        ACTIVE.fetch_sub(1, Ordering::AcqRel);
    }
}
#[link(name = "opusfile")]
unsafe extern "C" {
    fn op_open_memory(data: *const u8, size: usize, error: *mut i32) -> *mut c_void;
    fn op_free(file: *mut c_void);
    fn op_pcm_total(file: *const c_void, link: i32) -> i64;
    fn op_pcm_tell(file: *const c_void) -> i64;
    fn op_pcm_seek(file: *mut c_void, offset: i64) -> i32;
    fn op_link_count(file: *const c_void) -> i32;
    fn op_channel_count(file: *const c_void, link: i32) -> i32;
    fn op_read_float_stereo(file: *mut c_void, pcm: *mut f32, size: i32) -> i32;
}

pub struct OpusFile {
    handle: *mut c_void,
    _bytes: Vec<u8>,
    _reservation: Reservation,
    pub frames: i64,
}
impl Drop for OpusFile {
    fn drop(&mut self) {
        // SAFETY: open owns this handle; backing bytes outlive op_free.
        unsafe { op_free(self.handle) };
    }
}
impl OpusFile {
    pub fn open(bytes: Vec<u8>) -> Result<Self, String> {
        if bytes.len() > MAX_OPUS_BYTES || !bytes.starts_with(b"OggS") {
            return Err("Packaged audio must be Ogg Opus, at most 32 MiB".into());
        }
        let mut error = 0;
        let reservation = Reservation::new(bytes.len())?;
        // SAFETY: the Vec's allocation is retained unchanged until Drop.
        let handle = unsafe { op_open_memory(bytes.as_ptr(), bytes.len(), &mut error) };
        if handle.is_null() {
            return Err(format!("Invalid Ogg Opus file ({error})"));
        }
        let mut file = Self {
            handle,
            _bytes: bytes,
            _reservation: reservation,
            frames: 0,
        };
        // SAFETY: op_open_memory returned a valid seekable decoder.
        let (links, channels, frames) = unsafe {
            (
                op_link_count(handle),
                op_channel_count(handle, -1),
                op_pcm_total(handle, -1),
            )
        };
        if links != 1 || !(1..=2).contains(&channels) || !(1..=48_000 * 600).contains(&frames) {
            return Err("Audio must be single-link mono/stereo Opus, at most ten minutes".into());
        }
        file.frames = frames;
        Ok(file)
    }
    pub fn position(&self) -> f64 {
        // SAFETY: valid owned decoder, accessed synchronously.
        unsafe { op_pcm_tell(self.handle) }.max(0) as f64 / 48000.0
    }
    pub fn seek(&mut self, seconds: f64) -> Result<(), String> {
        if !seconds.is_finite() || seconds < 0.0 || seconds * 48000.0 > self.frames as f64 {
            return Err("Audio seek exceeds clip duration".into());
        }
        // SAFETY: offset is bounded to this decoder's PCM duration.
        let result = unsafe { op_pcm_seek(self.handle, (seconds * 48000.0) as i64) };
        if result != 0 {
            return Err(format!("Opus seek failed ({result})"));
        }
        Ok(())
    }
    pub fn read(&mut self, frames: usize) -> Result<Vec<[f32; 2]>, String> {
        if !(1..=4800).contains(&frames) {
            return Err("Audio read exceeds frame budget".into());
        }
        let mut pcm = vec![[0.0f32; 2]; frames];
        // SAFETY: stereo contiguous buffer holds exactly 2*frames floats.
        let count = unsafe {
            op_read_float_stereo(self.handle, pcm.as_mut_ptr().cast(), (frames * 2) as i32)
        };
        if count < 0 {
            return Err(format!("Opus decode failed ({count})"));
        }
        pcm.truncate(count as usize);
        for frame in &mut pcm {
            for sample in frame {
                *sample = if sample.is_finite() {
                    sample.clamp(-1.0, 1.0)
                } else {
                    0.0
                };
            }
        }
        Ok(pcm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn memory_only_opus_decode_seek_and_cleanup() {
        let output = std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=330:duration=0.2",
                "-c:a",
                "libopus",
                "-f",
                "opus",
                "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        for _ in 0..8 {
            let mut clip = OpusFile::open(output.stdout.clone()).unwrap();
            assert_eq!(clip.frames, 9600);
            let mut decoded = 0;
            let mut energy = 0.0;
            loop {
                let pcm = clip.read(4800).unwrap();
                if pcm.is_empty() {
                    break;
                }
                decoded += pcm.len();
                energy += pcm.iter().map(|v| v[0].abs()).sum::<f32>();
            }
            assert_eq!(decoded, 9600);
            assert!(energy > 1.0);
            assert!((clip.position() - 0.2).abs() < 0.001);
            clip.seek(0.1).unwrap();
            assert!((clip.position() - 0.1).abs() < 0.001);
            assert!(!clip.read(960).unwrap().is_empty());
            assert!(clip.seek(f64::NAN).is_err());
            assert!(clip.seek(-1.0).is_err());
            assert!(clip.seek(1.0).is_err());
            assert!(clip.read(0).is_err());
            assert!(clip.read(4801).is_err());
        }
        for bytes in [
            vec![],
            b"RIFFnot-opus".to_vec(),
            output.stdout[..20].to_vec(),
        ] {
            assert!(OpusFile::open(bytes).is_err());
        }
        let clips: Vec<_> = (0..32)
            .map(|_| OpusFile::open(output.stdout.clone()).unwrap())
            .collect();
        assert!(OpusFile::open(output.stdout.clone()).is_err());
        drop(clips);
        assert_eq!(ACTIVE.load(Ordering::Acquire), 0);
        assert_eq!(RESIDENT.load(Ordering::Acquire), 0);
        assert!(OpusFile::open(output.stdout).is_ok());
    }
}
