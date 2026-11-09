//! Godot's native generator/mixer handles playback; libopusfile only decodes.
use godot::{classes::RefCounted, prelude::*};
#[path = "../../shared/opus_file.rs"]
mod opus_file;

#[derive(GodotClass)]
#[class(base=RefCounted,init)]
struct PackagedOpus {
    clip: Option<opus_file::OpusFile>,
    lease: Option<crate::audio_budget::Lease>,
    last_error: String,
    base: Base<RefCounted>,
}
#[godot_api]
impl PackagedOpus {
    #[func]
    fn open_clip(&mut self, bytes: PackedByteArray) -> GString {
        self.close();
        if bytes.len() > opus_file::MAX_OPUS_BYTES {
            return "Audio exceeds 32 MiB".into();
        }
        let lease = match crate::audio_budget::Lease::reserve(bytes.len()) {
            Ok(lease) => lease,
            Err(error) => return error.into(),
        };
        match opus_file::OpusFile::open(bytes.to_vec()) {
            Ok(clip) => {
                self.clip = Some(clip);
                self.lease = Some(lease);
                GString::new()
            }
            Err(error) => error.as_str().into(),
        }
    }
    #[func]
    fn read_pcm(&mut self, frames: i32) -> PackedVector2Array {
        let Some(clip) = &mut self.clip else {
            return PackedVector2Array::new();
        };
        match clip.read(frames.max(0) as usize) {
            Ok(pcm) => PackedVector2Array::from(
                pcm.iter()
                    .map(|v| Vector2::new(v[0], v[1]))
                    .collect::<Vec<_>>()
                    .as_slice(),
            ),
            Err(error) => {
                self.clip = None;
                self.lease = None;
                self.last_error = error;
                PackedVector2Array::new()
            }
        }
    }
    #[func]
    fn seek(&mut self, seconds: f64) -> GString {
        match self
            .clip
            .as_mut()
            .ok_or_else(|| "Audio clip is closed".to_string())
            .and_then(|clip| clip.seek(seconds))
        {
            Ok(()) => GString::new(),
            Err(error) => error.as_str().into(),
        }
    }
    #[func]
    fn duration(&self) -> f64 {
        self.clip
            .as_ref()
            .map_or(0.0, |c| c.frames as f64 / 48000.0)
    }
    #[func]
    fn position(&self) -> f64 {
        self.clip.as_ref().map_or(0.0, |c| c.position())
    }
    #[func]
    fn error(&mut self) -> GString {
        std::mem::take(&mut self.last_error).as_str().into()
    }
    #[func]
    fn close(&mut self) {
        self.clip = None;
        self.lease = None;
        self.last_error.clear();
    }
}
