use crate::http::{ApiError, ApiResult};
use serde::{Deserialize, Serialize};
mod state;
// Publication uses the same maintained container/header/duration validation as
// playback; PCM read/seek methods are only needed by the native client.
#[allow(dead_code)]
#[path = "../../shared/opus_file.rs"]
mod packaged_opus;
pub use state::{REPLICATED_STATE_BYTES, StateVariable};
use std::{
    collections::HashSet,
    io::{Read, Seek},
    path::Path,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub scene: String,
    #[serde(default = "capacity")]
    pub capacity: usize,
    #[serde(default)]
    pub spawn: [f32; 3],
    #[serde(default)]
    pub objects: Vec<Object>,
    #[serde(default)]
    pub scripts: Vec<Script>,
    #[serde(default)]
    pub variables: std::collections::BTreeMap<String, StateVariable>,
    #[serde(default)]
    pub prefabs: std::collections::BTreeMap<String, Prefab>,
    #[serde(default)]
    pub actions: Vec<String>,
    #[serde(default)]
    pub contacts: Vec<Contact>,
    #[serde(default = "yes")]
    pub allow_voice_bypass: bool,
    #[serde(default)]
    pub video_surfaces: Vec<crate::world::media::Surface>,
    #[serde(default)]
    pub audio_zones: Vec<AudioZone>,
    #[serde(default)]
    pub sounds: Vec<Sound>,
    #[serde(default)]
    pub radios: Vec<Radio>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Radio {
    pub id: String,
    pub position: [f32; 3],
    pub channel: u16,
    #[serde(default)]
    pub preset: RadioPreset,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RadioPreset {
    #[default]
    Standard,
    Narrow,
    Weak,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sound {
    pub id: String,
    pub path: String,
    #[serde(default)]
    pub position: [f32; 3],
    #[serde(default = "unit_volume")]
    pub volume: f32,
    #[serde(default)]
    pub autoplay: bool,
    #[serde(default)]
    pub r#loop: bool,
}
fn unit_volume() -> f32 {
    1.0
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AudioEffects {
    pub reverb: f32,
    pub echo: f32,
    pub distortion: f32,
    pub compression: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudioZone {
    pub id: String,
    pub position: [f32; 3],
    pub size: [f32; 3],
    #[serde(default)]
    pub effects: AudioEffects,
}
fn yes() -> bool {
    true
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Contact {
    pub id: String,
    pub bone: String,
    pub anchor: String,
    pub offset: [f32; 3],
    pub radius: f32,
    #[serde(default)]
    pub grabbable: bool,
}
fn capacity() -> usize {
    16
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Object {
    pub id: String,
    pub component: String,
    #[serde(default)]
    pub position: [f32; 3],
    #[serde(default)]
    pub size: [f32; 3],
    #[serde(default)]
    pub physics: Option<Physics>,
    #[serde(default)]
    pub prefab: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Prefab {
    pub scene: String,
    pub component: String,
    #[serde(default)]
    pub physics: Physics,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Physics {
    pub dynamic: bool,
    pub shape: String,
    pub size: [f32; 3],
    pub mass: f32,
    pub gravity_scale: f32,
    pub health: f32,
    pub damage: f32,
    pub lifetime: f32,
    pub speed: f32,
    pub turn_speed: f32,
    pub projectile: Option<String>,
    pub fire_interval: f32,
}
impl Default for Physics {
    fn default() -> Self {
        Self {
            dynamic: true,
            shape: "box".into(),
            size: [0.5; 3],
            mass: 1.0,
            gravity_scale: 1.0,
            health: 0.0,
            damage: 0.0,
            lifetime: 0.0,
            speed: 5.0,
            turn_speed: 1.5,
            projectile: None,
            fire_interval: 0.5,
        }
    }
}
impl Physics {
    pub fn valid(&self) -> bool {
        matches!(self.shape.as_str(), "box" | "sphere")
            && self
                .size
                .iter()
                .all(|v| v.is_finite() && *v >= 0.01 && *v <= 1000.0)
            && [
                (self.mass, 0.01, 100000.0),
                (self.gravity_scale, 0.0, 10.0),
                (self.health, 0.0, 100000.0),
                (self.damage, 0.0, 100000.0),
                (self.lifetime, 0.0, 3600.0),
                (self.speed, 0.0, 100.0),
                (self.turn_speed, 0.0, 10.0),
                (self.fire_interval, 0.1, 60.0),
            ]
            .iter()
            .all(|(value, min, max)| value.is_finite() && value >= min && value <= max)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Script {
    pub path: String,
    pub side: String,
    pub attachment: String,
    pub capabilities: Vec<String>,
}
fn safe_path(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 240
        && !s.contains(['\\', ':', '\0'])
        && !s.starts_with('/')
        && s.split('/').all(|c| !c.is_empty() && c != "." && c != "..")
}
pub fn validate<R: Read + Seek>(source: R, kind: &str, budget: u64) -> ApiResult<Manifest> {
    let mut zip =
        zip::ZipArchive::new(source).map_err(|_| ApiError::bad("Expected a ZIP package"))?;
    if zip.len() > 2048 {
        return Err(ApiError::bad("At most 2048 package entries"));
    }
    let mut files = HashSet::new();
    let mut total = 0u64;
    for i in 0..zip.len() {
        let f = zip
            .by_index(i)
            .map_err(|_| ApiError::bad("Invalid or unsupported ZIP entry"))?;
        if f.is_dir() {
            continue;
        }
        if !safe_path(f.name())
            || f.unix_mode()
                .is_some_and(|m| (m & 0o170000) != 0 && (m & 0o170000) != 0o100000)
            || !files.insert(f.name().to_string())
        {
            return Err(ApiError::bad("Unsafe/duplicate package entry"));
        }
        total = total
            .checked_add(f.size())
            .ok_or_else(|| ApiError::bad("Size overflow"))?;
        if total > budget.saturating_mul(2).min(2_000_000_000) || f.size() > 512_000_000 {
            return Err(ApiError::bad(
                "Expanded package or entry exceeds memory budget",
            ));
        }
        let ext = Path::new(f.name())
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if !matches!(
            ext,
            "json" | "glb" | "vrm" | "png" | "jpg" | "jpeg" | "webp" | "opus" | "luau"
        ) {
            return Err(ApiError::bad(format!(
                "Unsupported file type: {}",
                f.name()
            )));
        }
        let expected = f.size();
        if f.name() == "manifest.json" && expected > 262144 {
            return Err(ApiError::bad("Manifest exceeds 256 KiB"));
        }
        if ext == "luau" {
            if expected > 65536 {
                return Err(ApiError::bad("Script exceeds 64 KiB"));
            }
            let mut bytes = Vec::new();
            f.take(expected + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| ApiError::bad("Invalid compressed script data"))?;
            if bytes.len() as u64 != expected || std::str::from_utf8(&bytes).is_err() {
                return Err(ApiError::bad("Script length/UTF-8 is invalid"));
            }
        } else {
            // Read one byte beyond the advertised size, without allocating the
            // entry. This verifies real expanded length and the ZIP CRC, even
            // for otherwise unused files. Forged metadata cannot evade budgets.
            let actual = std::io::copy(&mut f.take(expected + 1), &mut std::io::sink())
                .map_err(|_| ApiError::bad("Invalid compressed package data/CRC"))?;
            if actual != expected {
                return Err(ApiError::bad("Package entry length mismatch"));
            }
        }
    }
    let manifest: Manifest = {
        let f = zip
            .by_name("manifest.json")
            .map_err(|_| ApiError::bad("Missing manifest.json"))?;
        if f.size() > 262144 {
            return Err(ApiError::bad("Manifest exceeds 256 KiB"));
        }
        let mut bytes = Vec::new();
        f.take(262145)
            .read_to_end(&mut bytes)
            .map_err(|_| ApiError::bad("Invalid manifest data"))?;
        if bytes.len() > 262144 {
            return Err(ApiError::bad("Manifest exceeds 256 KiB"));
        }
        serde_json::from_slice(&bytes)
            .map_err(|e| ApiError::bad(format!("Invalid manifest: {e}")))?
    };
    if manifest.schema != 1
        || manifest.kind != kind
        || manifest.name.is_empty()
        || manifest.name.len() > 120
        || manifest.description.len() > 4096
        || !(1..=64).contains(&manifest.capacity)
        || !manifest.spawn.iter().all(|x| x.is_finite())
    {
        return Err(ApiError::bad(
            "Unsupported manifest version or invalid metadata",
        ));
    }
    let require = |s: &str| -> ApiResult<()> {
        if safe_path(s) && files.contains(s) {
            Ok(())
        } else {
            Err(ApiError::bad(format!("Missing/unsafe reference: {s}")))
        }
    };
    require(&manifest.scene)?;
    if !manifest.scene.ends_with(".glb") && !(kind == "avatar" && manifest.scene.ends_with(".vrm"))
    {
        return Err(ApiError::bad("Scene must be an embedded GLB/VRM file"));
    }
    for (id, prefab) in &manifest.prefabs {
        require(&prefab.scene)?;
        if id.is_empty()
            || id.len() > 64
            || !prefab.scene.ends_with(".glb")
            || !matches!(prefab.component.as_str(), "prop" | "vehicle" | "projectile")
            || !prefab.physics.valid()
            || prefab
                .physics
                .projectile
                .as_ref()
                .is_some_and(|id| !manifest.prefabs.contains_key(id))
        {
            return Err(ApiError::bad(
                "Invalid prefab definition or projectile reference",
            ));
        }
    }
    let mut ids = HashSet::new();
    if manifest.variables.len() > 128
        || manifest
            .variables
            .iter()
            .map(|(key, value)| key.len() + value.default_value().to_string().len() + 4)
            .sum::<usize>()
            > REPLICATED_STATE_BYTES
        || manifest.variables.iter().any(|(key, value)| {
            key.is_empty()
                || key.len() > 64
                || !key
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
                || !value.valid()
        })
    {
        return Err(ApiError::bad("Invalid replicated variable declaration"));
    }
    let mut radio_ids = HashSet::new();
    if manifest.radios.len() > 8
        || manifest.radios.iter().any(|r| {
            r.id.is_empty()
                || r.id.len() > 64
                || !radio_ids.insert(&r.id)
                || !(1..=9999).contains(&r.channel)
                || r.position
                    .iter()
                    .any(|v| !v.is_finite() || v.abs() > 10000.0)
        })
    {
        return Err(ApiError::bad("Invalid radio component"));
    }
    let mut audio_ids = HashSet::new();
    let mut sound_ids = HashSet::new();
    if manifest.sounds.len() > 8
        || manifest.sounds.iter().any(|s| {
            s.id.is_empty()
                || s.id.len() > 64
                || !sound_ids.insert(&s.id)
                || !s.path.ends_with(".opus")
                || !files.contains(&s.path)
                || !s.volume.is_finite()
                || !(0.0..=1.0).contains(&s.volume)
                || s.position
                    .iter()
                    .any(|v| !v.is_finite() || v.abs() > 10000.0)
        })
    {
        return Err(ApiError::bad("Invalid packaged Opus sound declaration"));
    }
    if manifest.audio_zones.len() > 16
        || manifest.audio_zones.iter().any(|z| {
            z.id.is_empty()
                || z.id.len() > 64
                || !audio_ids.insert(&z.id)
                || z.position
                    .iter()
                    .any(|v| !v.is_finite() || v.abs() > 10000.0)
                || z.size
                    .iter()
                    .any(|v| !v.is_finite() || !(0.1..=1000.0).contains(v))
                || [z.effects.reverb, z.effects.echo, z.effects.distortion]
                    .iter()
                    .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        })
    {
        return Err(ApiError::bad("Invalid acoustic zone or effect range"));
    }
    let mut media_ids = HashSet::new();
    if manifest.video_surfaces.len() > 4
        || manifest.video_surfaces.iter().any(|s| {
            s.id.is_empty()
                || s.id.len() > 64
                || !media_ids.insert(&s.id)
                || s.position
                    .iter()
                    .any(|v| !v.is_finite() || v.abs() > 10000.0)
                || s.size
                    .iter()
                    .any(|v| !v.is_finite() || !(0.1..=20.0).contains(v))
        })
    {
        return Err(ApiError::bad("Invalid video surface budget or geometry"));
    }
    let mut contact_ids = HashSet::new();
    if manifest.contacts.len() > 64
        || manifest.contacts.iter().any(|c| {
            c.id.is_empty()
                || c.id.len() > 64
                || !contact_ids.insert(&c.id)
                || c.bone.is_empty()
                || c.bone.len() > 64
                || !matches!(
                    c.anchor.as_str(),
                    "head" | "left_hand" | "right_hand" | "hips"
                )
                || !c.offset.iter().all(|v| v.is_finite() && v.abs() <= 2.0)
                || !c.radius.is_finite()
                || !(0.01..=1.0).contains(&c.radius)
        })
    {
        return Err(ApiError::bad("Invalid avatar contact collider"));
    }
    for o in &manifest.objects {
        if o.id.is_empty()
            || o.id.len() > 64
            || !ids.insert(&o.id)
            || !matches!(
                o.component.as_str(),
                "switch" | "door" | "prop" | "radio" | "video" | "static" | "vehicle"
            )
            || !o
                .position
                .iter()
                .chain(o.size.iter())
                .all(|v| v.is_finite() && v.abs() < 100000.0)
        {
            return Err(ApiError::bad("Invalid object ID/component/transform"));
        }
        if o.prefab
            .as_ref()
            .is_some_and(|id| !manifest.prefabs.contains_key(id))
            || o.physics.as_ref().is_some_and(|p| {
                !p.valid()
                    || p.projectile
                        .as_ref()
                        .is_some_and(|id| !manifest.prefabs.contains_key(id))
            })
        {
            return Err(ApiError::bad("Invalid object physics or prefab reference"));
        }
    }
    if manifest.objects.len() > 4096
        || manifest.scripts.len() > 32
        || manifest.prefabs.len() > 256
        || manifest.actions.len() > 32
        || manifest
            .actions
            .iter()
            .any(|a| a.is_empty() || a.len() > 64)
    {
        return Err(ApiError::bad("Too many components"));
    }
    for script in &manifest.scripts {
        require(&script.path)?;
        if !script.path.ends_with(".luau")
            || !matches!(script.side.as_str(), "server" | "client")
            || (kind == "avatar"
                && (script.side != "client"
                    || script
                        .capabilities
                        .iter()
                        .any(|c| !matches!(c.as_str(), "visual" | "audio"))))
            || script.capabilities.iter().any(|c| {
                !matches!(
                    c.as_str(),
                    "state" | "objects" | "visual" | "audio" | "media" | "radio"
                )
            })
            || (script.attachment != "world"
                && !(kind == "avatar" && script.attachment == "avatar")
                && !ids.contains(&script.attachment))
        {
            return Err(ApiError::bad("Unsupported script declaration/capability"));
        }
        if zip
            .by_name(&script.path)
            .map_err(ApiError::internal)?
            .size()
            > 65536
        {
            return Err(ApiError::bad("Script exceeds 64 KiB"));
        }
    }
    let mut audio_bytes = 0u64;
    let mut image_bytes = 0u64;
    for path in &files {
        let mut f = zip.by_name(path).map_err(ApiError::internal)?;
        if path.ends_with(".glb") || path.ends_with(".vrm") {
            image_bytes += validate_glb(&mut f, kind == "avatar" && path == &manifest.scene)?;
        } else if path.ends_with(".png")
            || path.ends_with(".jpg")
            || path.ends_with(".jpeg")
            || path.ends_with(".webp")
        {
            if f.size() > 16 * 1024 * 1024 {
                return Err(ApiError::bad("Compressed image exceeds 16 MiB"));
            }
            let mut bytes = Vec::new();
            f.take(16 * 1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| ApiError::bad("Invalid image data"))?;
            validate_image(&bytes, &mut image_bytes)?;
        } else if path.ends_with(".opus") {
            audio_bytes = audio_bytes.saturating_add(f.size());
            if f.size() > 32 * 1024 * 1024 || audio_bytes > 64 * 1024 * 1024 {
                return Err(ApiError::bad(
                    "Packaged Opus exceeds 32 MiB per file or 64 MiB total",
                ));
            }
            let mut bytes = Vec::new();
            f.take(packaged_opus::MAX_OPUS_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(ApiError::internal)?;
            packaged_opus::OpusFile::open(bytes).map_err(ApiError::bad)?;
        }
        if image_bytes > 256 * 1024 * 1024 {
            return Err(ApiError::bad(
                "Package decoded image budget exceeds 256 MiB",
            ));
        }
    }
    Ok(manifest)
}
fn validate_glb(r: &mut impl Read, avatar: bool) -> ApiResult<u64> {
    let mut head = [0u8; 20];
    r.read_exact(&mut head)
        .map_err(|_| ApiError::bad("Truncated GLB"))?;
    let u32at = |i| u32::from_le_bytes(head[i..i + 4].try_into().unwrap());
    let len = u32at(12) as usize;
    if &head[..4] != b"glTF"
        || u32at(4) != 2
        || &head[16..20] != b"JSON"
        || len > 8_000_000
        || !len.is_multiple_of(4)
        || u32at(8) < 20 + len as u32
    {
        return Err(ApiError::bad("Invalid GLB 2.0 header/JSON budget"));
    }
    let mut bytes = vec![0; len];
    r.read_exact(&mut bytes)
        .map_err(|_| ApiError::bad("Truncated GLB JSON"))?;
    let doc: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| ApiError::bad("Malformed GLB JSON"))?;
    let total = u32at(8) as u64;
    let mut consumed = 20 + len as u64;
    let mut binary = None;
    let mut image_bytes = 0;
    while consumed < total {
        if total - consumed < 8 {
            return Err(ApiError::bad("Truncated GLB chunk header"));
        }
        let mut chunk = [0; 8];
        r.read_exact(&mut chunk)
            .map_err(|_| ApiError::bad("Truncated GLB chunk header"))?;
        let size = u32::from_le_bytes(chunk[..4].try_into().unwrap()) as u64;
        consumed += 8;
        if !size.is_multiple_of(4)
            || size > total - consumed
            || &chunk[4..] != b"BIN\0"
            || binary.is_some()
        {
            return Err(ApiError::bad("Invalid/unsupported GLB chunk layout"));
        }
        validate_glb_structure(&doc, Some(size))?;
        let mut chunk_reader = (&mut *r).take(size);
        validate_embedded_images(&mut chunk_reader, &doc, &mut image_bytes)?;
        let remaining = chunk_reader.limit();
        let actual = std::io::copy(&mut chunk_reader, &mut std::io::sink())
            .map_err(|_| ApiError::bad("Invalid GLB binary data"))?;
        if actual != remaining {
            return Err(ApiError::bad("Truncated GLB binary data"));
        }
        binary = Some(size);
        consumed += size;
    }
    let mut trailing = [0];
    if r.read(&mut trailing)
        .map_err(|_| ApiError::bad("Invalid GLB data"))?
        != 0
    {
        return Err(ApiError::bad("GLB declared length mismatch"));
    }
    validate_glb_structure(&doc, binary)?;
    if doc["asset"]["version"] != "2.0" {
        return Err(ApiError::bad("Unsupported GLB version"));
    }
    for table in ["buffers", "images"] {
        if let Some(entries) = doc[table].as_array() {
            for entry in entries {
                if entry.get("uri").is_some() {
                    return Err(ApiError::bad(
                        "GLB resources must use embedded buffer views, not external/data URIs",
                    ));
                }
            }
        }
    }
    if let Some(ext) = doc["extensionsRequired"].as_array() {
        for e in ext {
            if !matches!(
                e.as_str(),
                Some(
                    "VRMC_vrm"
                        | "VRMC_springBone"
                        | "VRMC_materials_mtoon"
                        | "KHR_materials_unlit"
                        | "KHR_texture_transform"
                        | "EXT_texture_webp"
                )
            ) {
                return Err(ApiError::bad("Unsupported required glTF extension"));
            }
        }
    }
    if avatar && doc["extensions"]["VRMC_vrm"]["specVersion"] != "1.0" {
        return Err(ApiError::bad("Avatar scene must contain VRM 1.0 metadata"));
    }
    Ok(image_bytes)
}

fn validate_image(bytes: &[u8], decoded_bytes: &mut u64) -> ApiResult<()> {
    use image::{ImageFormat, ImageReader, Limits};
    use std::io::Cursor;
    let format =
        image::guess_format(bytes).map_err(|_| ApiError::bad("Invalid image signature"))?;
    if !matches!(
        format,
        ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
    ) {
        return Err(ApiError::bad("Only PNG/JPEG/WebP images are supported"));
    }
    let reader = || {
        let mut limits = Limits::default();
        limits.max_image_width = Some(8192);
        limits.max_image_height = Some(8192);
        limits.max_alloc = Some(64 * 1024 * 1024);
        let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
        reader.limits(limits);
        reader
    };
    let (width, height) = reader()
        .into_dimensions()
        .map_err(|_| ApiError::bad("Invalid image dimensions"))?;
    // Conservative RGBA16 estimate includes high-bit-depth source images.
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return Err(ApiError::bad("Image dimensions/decoded size exceed budget"));
    }
    let allocation = u64::from(width) * u64::from(height) * 8;
    if allocation > 64 * 1024 * 1024 {
        return Err(ApiError::bad("Image decoded size exceeds 64 MiB"));
    }
    *decoded_bytes = decoded_bytes
        .checked_add(allocation)
        .filter(|n| *n <= 256 * 1024 * 1024)
        .ok_or_else(|| ApiError::bad("Decoded image budget exceeds 256 MiB"))?;
    reader()
        .decode()
        .map_err(|_| ApiError::bad("Malformed image or decoder resource limit exceeded"))?;
    Ok(())
}

fn validate_embedded_images(
    r: &mut impl Read,
    doc: &serde_json::Value,
    decoded_bytes: &mut u64,
) -> ApiResult<()> {
    let mut ranges = std::collections::BTreeMap::new();
    if let Some(images) = doc["images"].as_array() {
        if images.len() > 256 {
            return Err(ApiError::bad("At most 256 embedded images"));
        }
        for image in images {
            let format = match image["mimeType"].as_str() {
                Some("image/png") => image::ImageFormat::Png,
                Some("image/jpeg") => image::ImageFormat::Jpeg,
                Some("image/webp") => image::ImageFormat::WebP,
                _ => return Err(ApiError::bad("Unsupported embedded image MIME type")),
            };
            let index = image["bufferView"]
                .as_u64()
                .and_then(|i| usize::try_from(i).ok())
                .ok_or_else(|| ApiError::bad("Image requires embedded buffer view"))?;
            let view = doc["bufferViews"]
                .as_array()
                .and_then(|views| views.get(index))
                .ok_or_else(|| ApiError::bad("Image buffer view out of bounds"))?;
            if view.get("byteStride").is_some() {
                return Err(ApiError::bad("Image buffer view cannot have a stride"));
            }
            let offset = view
                .get("byteOffset")
                .map_or(Some(0), serde_json::Value::as_u64)
                .ok_or_else(|| ApiError::bad("Invalid image offset"))?;
            let length = view["byteLength"]
                .as_u64()
                .filter(|n| *n <= 16 * 1024 * 1024)
                .ok_or_else(|| ApiError::bad("Compressed image exceeds 16 MiB"))?;
            let entry = ranges.entry((offset, length)).or_insert((format, 0u64));
            if entry.0 != format {
                return Err(ApiError::bad("Conflicting embedded image MIME types"));
            }
            entry.1 += 1;
        }
    }
    let mut cursor = 0;
    for ((offset, length), (format, count)) in ranges {
        let gap = offset
            .checked_sub(cursor)
            .ok_or_else(|| ApiError::bad("Overlapping embedded images"))?;
        if std::io::copy(&mut (&mut *r).take(gap), &mut std::io::sink())
            .map_err(|_| ApiError::bad("Invalid image buffer"))?
            != gap
        {
            return Err(ApiError::bad("Truncated image buffer"));
        }
        let mut bytes = vec![0; length as usize];
        r.read_exact(&mut bytes)
            .map_err(|_| ApiError::bad("Truncated embedded image"))?;
        if image::guess_format(&bytes).ok() != Some(format) {
            return Err(ApiError::bad("Embedded image signature/MIME mismatch"));
        }
        let mut allocation = 0;
        validate_image(&bytes, &mut allocation)?;
        *decoded_bytes = allocation
            .checked_mul(count)
            .and_then(|n| decoded_bytes.checked_add(n))
            .filter(|n| *n <= 256 * 1024 * 1024)
            .ok_or_else(|| ApiError::bad("Decoded image budget exceeds 256 MiB"))?;
        cursor = offset + length;
    }
    Ok(())
}

fn validate_glb_structure(doc: &serde_json::Value, binary: Option<u64>) -> ApiResult<()> {
    use serde_json::Value;
    let table = |name: &str| -> ApiResult<&[Value]> {
        match doc.get(name) {
            None => Ok(&[]),
            Some(Value::Array(values)) if values.len() <= 16384 => Ok(values),
            _ => Err(ApiError::bad(format!("Invalid/oversized GLB {name} table"))),
        }
    };
    let buffers = table("buffers")?;
    if buffers.len() > 1 {
        return Err(ApiError::bad("GLB requires a single embedded buffer"));
    }
    let buffer_len = if let Some(buffer) = buffers.first() {
        let size = buffer["byteLength"]
            .as_u64()
            .ok_or_else(|| ApiError::bad("Invalid GLB buffer length"))?;
        if binary.is_none_or(|actual| size > actual || actual - size > 3) {
            return Err(ApiError::bad("GLB buffer exceeds binary chunk"));
        }
        size
    } else {
        0
    };
    let views = table("bufferViews")?;
    let images = table("images")?;
    if !images.is_empty() && binary.is_none() {
        return Err(ApiError::bad("Images require embedded binary data"));
    }
    for view in views {
        let offset = view.get("byteOffset").map_or(Some(0), Value::as_u64);
        let length = view["byteLength"].as_u64();
        if buffers.is_empty()
            || view["buffer"].as_u64() != Some(0)
            || offset
                .zip(length)
                .and_then(|(a, b)| a.checked_add(b))
                .is_none_or(|end| end > buffer_len)
        {
            return Err(ApiError::bad("GLB buffer view out of bounds"));
        }
    }
    let mut decoded_bytes = 0u64;
    for accessor in table("accessors")? {
        let component = match accessor["componentType"].as_u64() {
            Some(5120 | 5121) => 1u64,
            Some(5122 | 5123) => 2,
            Some(5125 | 5126) => 4,
            _ => return Err(ApiError::bad("Invalid GLB accessor component type")),
        };
        let (rows, columns) = match accessor["type"].as_str() {
            Some("SCALAR") => (1u64, 1),
            Some("VEC2") => (2, 1),
            Some("VEC3") => (3, 1),
            Some("VEC4") => (4, 1),
            Some("MAT2") => (2, 2),
            Some("MAT3") => (3, 3),
            Some("MAT4") => (4, 4),
            _ => return Err(ApiError::bad("Invalid GLB accessor shape")),
        };
        let column_bytes = rows * component;
        // Matrix columns are four-byte aligned even for byte/short components.
        let element = if columns > 1 {
            column_bytes.div_ceil(4) * 4 * columns
        } else {
            column_bytes
        };
        let tail = if columns > 1 {
            column_bytes.div_ceil(4) * 4 * (columns - 1) + column_bytes
        } else {
            element
        };
        let alignment = if columns > 1 { 4 } else { component };
        let count = accessor["count"]
            .as_u64()
            .filter(|count| *count > 0)
            .ok_or_else(|| ApiError::bad("Invalid GLB accessor count"))?;
        // Importers can expand packed integers into floats, and sparse accessors
        // can allocate arrays without any full-size backing buffer view.
        decoded_bytes = count
            .checked_mul(rows * columns * 4)
            .and_then(|size| decoded_bytes.checked_add(size))
            .filter(|total| *total <= 256 * 1024 * 1024)
            .ok_or_else(|| ApiError::bad("GLB accessor decoded budget exceeds 256 MiB"))?;
        if let Some(view) = accessor.get("bufferView") {
            validate_accessor_range(
                views,
                view,
                accessor.get("byteOffset"),
                count,
                (element, tail),
                alignment,
                true,
            )?;
        } else if accessor
            .get("byteOffset")
            .is_some_and(|offset| offset.as_u64() != Some(0))
        {
            return Err(ApiError::bad("GLB accessor offset without buffer view"));
        }
        if let Some(sparse) = accessor.get("sparse") {
            let sparse_count = sparse["count"]
                .as_u64()
                .filter(|n| *n > 0 && *n <= count)
                .ok_or_else(|| ApiError::bad("Invalid GLB sparse count"))?;
            let index_bytes = match sparse["indices"]["componentType"].as_u64() {
                Some(5121) => 1,
                Some(5123) => 2,
                Some(5125) => 4,
                _ => return Err(ApiError::bad("Invalid GLB sparse index type")),
            };
            validate_accessor_range(
                views,
                &sparse["indices"]["bufferView"],
                sparse["indices"].get("byteOffset"),
                sparse_count,
                (index_bytes, index_bytes),
                index_bytes,
                false,
            )?;
            validate_accessor_range(
                views,
                &sparse["values"]["bufferView"],
                sparse["values"].get("byteOffset"),
                sparse_count,
                (element, tail),
                alignment,
                false,
            )?;
        }
    }
    let nodes = table("nodes")?;
    let mut parents = vec![None; nodes.len()];
    for (parent, node) in nodes.iter().enumerate() {
        if !node.is_object() {
            return Err(ApiError::bad("Invalid GLB node"));
        }
        if let Some(children) = node.get("children") {
            let children = children
                .as_array()
                .ok_or_else(|| ApiError::bad("Invalid GLB children"))?;
            for child in children {
                let child = child
                    .as_u64()
                    .filter(|i| *i < nodes.len() as u64)
                    .ok_or_else(|| ApiError::bad("GLB child index out of bounds"))?
                    as usize;
                if parents[child].replace(parent).is_some() {
                    return Err(ApiError::bad("GLB node has multiple parents"));
                }
            }
        }
    }
    // Bounded iterative walks avoid stack overflow on hostile deep/cyclic graphs.
    for index in 0..nodes.len() {
        let mut current = Some(index);
        let mut depth = 0;
        while let Some(node) = current {
            depth += 1;
            if depth > 256 {
                return Err(ApiError::bad("GLB cyclic or excessively deep hierarchy"));
            }
            current = parents[node];
        }
    }
    let scenes = table("scenes")?;
    if doc
        .get("scene")
        .is_some_and(|index| index.as_u64().is_none_or(|i| i >= scenes.len() as u64))
    {
        return Err(ApiError::bad("GLB default scene out of bounds"));
    }
    for scene in scenes {
        if !scene.is_object() {
            return Err(ApiError::bad("Invalid GLB scene"));
        }
        if let Some(roots) = scene.get("nodes") {
            let roots = roots
                .as_array()
                .ok_or_else(|| ApiError::bad("Invalid GLB scene roots"))?;
            let mut seen = HashSet::new();
            for root in roots {
                let index = root
                    .as_u64()
                    .filter(|i| *i < nodes.len() as u64)
                    .ok_or_else(|| ApiError::bad("GLB scene root out of bounds"))?
                    as usize;
                if parents[index].is_some() || !seen.insert(index) {
                    return Err(ApiError::bad("Invalid/duplicate GLB scene root"));
                }
            }
        }
    }
    Ok(())
}

fn validate_accessor_range(
    views: &[serde_json::Value],
    index: &serde_json::Value,
    offset: Option<&serde_json::Value>,
    count: u64,
    layout: (u64, u64),
    alignment: u64,
    allow_stride: bool,
) -> ApiResult<()> {
    let (element, tail) = layout;
    let view = index
        .as_u64()
        .and_then(|i| usize::try_from(i).ok())
        .and_then(|i| views.get(i))
        .ok_or_else(|| ApiError::bad("GLB accessor buffer view out of bounds"))?;
    let offset = offset
        .map_or(Some(0), serde_json::Value::as_u64)
        .ok_or_else(|| ApiError::bad("Invalid GLB accessor offset"))?;
    let stride = match view.get("byteStride") {
        None => element,
        Some(value) if allow_stride => value
            .as_u64()
            .filter(|n| (4..=252).contains(n) && n.is_multiple_of(4) && *n >= element)
            .ok_or_else(|| ApiError::bad("Invalid GLB accessor stride"))?,
        _ => return Err(ApiError::bad("Sparse GLB buffer view cannot have a stride")),
    };
    let end = count
        .checked_sub(1)
        .and_then(|n| n.checked_mul(stride))
        .and_then(|n| n.checked_add(tail))
        .and_then(|n| n.checked_add(offset));
    let base = view
        .get("byteOffset")
        .map_or(Some(0), serde_json::Value::as_u64);
    if !offset.is_multiple_of(alignment)
        || base
            .and_then(|base| base.checked_add(offset))
            .is_none_or(|start| !start.is_multiple_of(alignment))
        || end
            .zip(view["byteLength"].as_u64())
            .is_none_or(|(end, length)| end > length)
    {
        return Err(ApiError::bad("GLB accessor range/alignment is invalid"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    #[test]
    fn radio_object_presets_are_typed_and_backwards_compatible() {
        let mut declaration = serde_json::json!({"id":"desk","position":[0,1,0],"channel":1});
        let default: Radio = serde_json::from_value(declaration.clone()).unwrap();
        assert_eq!(default.preset, RadioPreset::Standard);
        for name in ["standard", "narrow", "weak"] {
            declaration["preset"] = serde_json::json!(name);
            let radio: Radio = serde_json::from_value(declaration.clone()).unwrap();
            assert_eq!(serde_json::to_value(radio).unwrap()["preset"], name);
        }
        for invalid in [
            serde_json::json!("unbounded"),
            serde_json::json!(10),
            serde_json::Value::Null,
        ] {
            declaration["preset"] = invalid;
            assert!(serde_json::from_value::<Radio>(declaration.clone()).is_err());
        }
    }
    #[test]
    fn decoded_images_and_embedded_ranges_are_bounded() {
        let png = |width, height| {
            let mut bytes = Cursor::new(Vec::new());
            image::DynamicImage::new_rgba8(width, height)
                .write_to(&mut bytes, image::ImageFormat::Png)
                .unwrap();
            bytes.into_inner()
        };
        let small = png(2, 2);
        let mut budget = 0;
        validate_image(&small, &mut budget).unwrap();
        assert_eq!(budget, 32);
        for format in [image::ImageFormat::Jpeg, image::ImageFormat::WebP] {
            let mut encoded = Cursor::new(Vec::new());
            image::DynamicImage::new_rgb8(2, 2)
                .write_to(&mut encoded, format)
                .unwrap();
            validate_image(encoded.get_ref(), &mut 0).unwrap();
        }
        assert!(validate_image(&small[..small.len() / 2], &mut 0).is_err());
        assert!(validate_image(&png(1, 8193), &mut 0).is_err());
        assert!(validate_image(&small, &mut (256 * 1024 * 1024)).is_err());
        let mut doc = serde_json::json!({"bufferViews":[{"buffer":0,"byteOffset":4,"byteLength":small.len()}],"images":[{"bufferView":0,"mimeType":"image/png"}]});
        let mut binary = vec![0; 4];
        binary.extend(&small);
        validate_embedded_images(&mut Cursor::new(&binary), &doc, &mut 0).unwrap();
        // Identical image views are decoded once but each declaration is charged.
        let duplicate = doc["images"][0].clone();
        doc["images"].as_array_mut().unwrap().push(duplicate);
        let mut budget = 0;
        validate_embedded_images(&mut Cursor::new(&binary), &doc, &mut budget).unwrap();
        assert_eq!(budget, 64);
        doc["images"][0]["mimeType"] = serde_json::json!("image/jpeg");
        assert!(validate_embedded_images(&mut Cursor::new(&binary), &doc, &mut 0).is_err());
        doc["images"] = serde_json::json!([{"bufferView":0,"mimeType":"image/png"},{"bufferView":1,"mimeType":"image/png"}]);
        doc["bufferViews"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"buffer":0,"byteOffset":5,"byteLength":small.len()-1}));
        assert!(validate_embedded_images(&mut Cursor::new(&binary), &doc, &mut 0).is_err());
    }
    #[test]
    fn accessor_ranges_sparse_allocations_and_matrix_padding() {
        let valid = serde_json::json!({
            "buffers":[{"byteLength":32}], "bufferViews":[{"buffer":0,"byteLength":32}],
            "accessors":[{"bufferView":0,"componentType":5126,"type":"VEC3","count":2}]
        });
        assert!(validate_glb_structure(&valid, Some(32)).is_ok());
        for (key, value) in [
            ("count", serde_json::json!(3)),
            ("byteOffset", serde_json::json!(1)),
            ("byteOffset", serde_json::json!(u64::MAX)),
            ("bufferView", serde_json::json!(1)),
            ("count", serde_json::json!(u64::MAX)),
            ("type", serde_json::json!("VEC5")),
        ] {
            let mut bad = valid.clone();
            bad["accessors"][0][key] = value;
            assert!(
                validate_glb_structure(&bad, Some(32)).is_err(),
                "accepted {key}"
            );
        }
        let mut sparse = valid.clone();
        sparse["accessors"][0] =
            serde_json::json!({"componentType":5126,"type":"VEC3","count":u64::MAX});
        assert!(validate_glb_structure(&sparse, Some(32)).is_err());
        sparse["accessors"][0] = serde_json::json!({"componentType":5126,"type":"VEC3","count":2,"sparse":{"count":1,"indices":{"bufferView":0,"componentType":5121},"values":{"bufferView":0}}});
        assert!(validate_glb_structure(&sparse, Some(32)).is_ok());
        sparse["accessors"][0]["sparse"]["values"]["byteOffset"] = serde_json::json!(28);
        assert!(validate_glb_structure(&sparse, Some(32)).is_err());
        // Last matrix column need not include trailing padding, but successive
        // columns/elements still start at four-byte boundaries (11 + 12 bytes).
        let mut matrix = valid.clone();
        matrix["buffers"][0]["byteLength"] = serde_json::json!(23);
        matrix["bufferViews"][0]["byteLength"] = serde_json::json!(23);
        matrix["accessors"][0] =
            serde_json::json!({"bufferView":0,"componentType":5121,"type":"MAT3","count":2});
        assert!(validate_glb_structure(&matrix, Some(24)).is_ok());
        matrix["bufferViews"][0]["byteLength"] = serde_json::json!(22);
        assert!(validate_glb_structure(&matrix, Some(24)).is_err());
    }
    #[test]
    fn glb_chunk_ranges_and_hierarchies_are_bounded() {
        let package = |doc: serde_json::Value, bin: Option<&[u8]>| {
            let mut json = serde_json::to_vec(&doc).unwrap();
            while !json.len().is_multiple_of(4) {
                json.push(b' ');
            }
            let mut bytes = Vec::new();
            bytes.extend(b"glTF");
            bytes.extend(2u32.to_le_bytes());
            bytes.extend(
                (20u32 + json.len() as u32 + bin.map_or(0, |b| 8 + b.len() as u32)).to_le_bytes(),
            );
            bytes.extend((json.len() as u32).to_le_bytes());
            bytes.extend(b"JSON");
            bytes.extend(json);
            if let Some(bin) = bin {
                bytes.extend((bin.len() as u32).to_le_bytes());
                bytes.extend(b"BIN\0");
                bytes.extend(bin);
            }
            bytes
        };
        let valid = serde_json::json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":4}],"bufferViews":[{"buffer":0,"byteLength":4}],"nodes":[{"children":[1]},{}],"scenes":[{"nodes":[0]}]});
        let bytes = package(valid.clone(), Some(&[0; 4]));
        assert!(validate_glb(&mut Cursor::new(&bytes), false).is_ok());
        let mut encoded = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(2, 2)
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let mut pixels = encoded.into_inner();
        let image_len = pixels.len();
        while !pixels.len().is_multiple_of(4) {
            pixels.push(0);
        }
        let image_doc = serde_json::json!({"asset":{"version":"2.0"},"buffers":[{"byteLength":pixels.len()}],"bufferViews":[{"buffer":0,"byteLength":image_len}],"images":[{"bufferView":0,"mimeType":"image/png"}]});
        assert_eq!(
            validate_glb(
                &mut Cursor::new(package(image_doc.clone(), Some(&pixels))),
                false
            )
            .unwrap(),
            32
        );
        pixels[0] = 0;
        assert!(validate_glb(&mut Cursor::new(package(image_doc, Some(&pixels))), false).is_err());
        for length in 0..bytes.len() {
            assert!(
                validate_glb(&mut Cursor::new(&bytes[..length]), false).is_err(),
                "accepted truncation {length}"
            );
        }
        let mut extra = bytes.clone();
        extra.push(0);
        assert!(validate_glb(&mut Cursor::new(extra), false).is_err());
        for (pointer, value) in [
            ("/buffers/0/byteLength", serde_json::json!(5)),
            ("/bufferViews/0/byteOffset", serde_json::json!(u64::MAX)),
            ("/bufferViews/0/byteLength", serde_json::json!(5)),
            ("/bufferViews/0/buffer", serde_json::json!(1)),
            ("/nodes/1", serde_json::json!({"children":[0]})),
            ("/nodes/0/children", serde_json::json!([1, 1])),
            ("/scenes/0/nodes", serde_json::json!([1])),
        ] {
            let mut bad = valid.clone();
            if pointer == "/bufferViews/0/byteOffset" {
                bad["bufferViews"][0]["byteOffset"] = value;
            } else {
                *bad.pointer_mut(pointer).unwrap() = value;
            }
            assert!(
                validate_glb(&mut Cursor::new(package(bad, Some(&[0; 4]))), false).is_err(),
                "accepted {pointer}"
            );
        }
        let deep: Vec<_> = (0..257)
            .map(|i| {
                if i < 256 {
                    serde_json::json!({"children":[i+1]})
                } else {
                    serde_json::json!({})
                }
            })
            .collect();
        assert!(validate_glb_structure(&serde_json::json!({"nodes":deep}), None).is_err());
    }
    #[test]
    fn packaged_audio_publication_checks_codec_and_declarations() {
        let opus = std::process::Command::new("ffmpeg")
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
        assert!(opus.status.success());
        let mut document = br#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{}]}"#.to_vec();
        while !document.len().is_multiple_of(4) {
            document.push(b' ');
        }
        let mut glb = b"glTF".to_vec();
        glb.extend(2u32.to_le_bytes());
        glb.extend((20 + document.len() as u32).to_le_bytes());
        glb.extend((document.len() as u32).to_le_bytes());
        glb.extend(b"JSON");
        glb.extend(document);
        let package = |audio: &[u8], declaration: serde_json::Value| {
            let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
            for (path, bytes) in [
                ("manifest.json", serde_json::to_vec(&serde_json::json!({"schema":1,"kind":"world","name":"Audio","scene":"world.glb","sounds":[declaration]})).unwrap()),
                ("world.glb", glb.clone()), ("audio/bell.opus", audio.to_vec())
            ] {
                writer.start_file(path, zip::write::SimpleFileOptions::default()).unwrap();
                writer.write_all(&bytes).unwrap();
            }
            writer.finish().unwrap().into_inner()
        };
        let declaration = serde_json::json!({"id":"bell","path":"audio/bell.opus"});
        let valid = package(&opus.stdout, declaration.clone());
        assert!(validate(Cursor::new(valid.clone()), "world", 1_000_000).is_ok());
        let footer = valid.len() - 22;
        assert_eq!(&valid[footer..footer + 4], b"PK\x05\x06");
        let central =
            u32::from_le_bytes(valid[footer + 16..footer + 20].try_into().unwrap()) as usize;
        assert_eq!(&valid[central..central + 4], b"PK\x01\x02");
        let mut bad_crc = valid.clone();
        bad_crc[central + 16] ^= 1;
        let error = validate(Cursor::new(bad_crc), "world", 1_000_000)
            .err()
            .unwrap();
        assert_eq!(error.0, axum::http::StatusCode::BAD_REQUEST);
        let mut bad_size = valid.clone();
        let declared = u32::from_le_bytes(bad_size[central + 24..central + 28].try_into().unwrap());
        bad_size[central + 24..central + 28].copy_from_slice(&(declared - 1).to_le_bytes());
        let error = validate(Cursor::new(bad_size), "world", 1_000_000)
            .err()
            .unwrap();
        assert_eq!(error.0, axum::http::StatusCode::BAD_REQUEST);
        let mut invalid_script = zip::ZipWriter::new_append(Cursor::new(valid)).unwrap();
        invalid_script
            .start_file(
                "scripts/invalid.luau",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        invalid_script.write_all(&[0xff, 0xfe]).unwrap();
        let error = validate(
            Cursor::new(invalid_script.finish().unwrap().into_inner()),
            "world",
            1_000_000,
        )
        .err()
        .unwrap();
        assert_eq!(error.0, axum::http::StatusCode::BAD_REQUEST);
        assert!(error.1.contains("UTF-8"));
        let mut fake = b"OggS".to_vec();
        fake.extend([0; 24]);
        fake.extend(b"OpusHead");
        fake.extend([0; 32]);
        assert!(
            validate(
                Cursor::new(package(&fake, declaration.clone())),
                "world",
                1_000_000
            )
            .is_err()
        );
        for bad in [
            serde_json::json!({"id":"bell","path":"audio/missing.opus"}),
            serde_json::json!({"id":"bell","path":"audio/bell.opus","volume":2}),
            serde_json::json!({"id":"bell","path":"audio/bell.wav"}),
        ] {
            assert!(validate(Cursor::new(package(&opus.stdout, bad)), "world", 1_000_000).is_err());
        }
    }
    #[test]
    fn paths() {
        for p in ["../a", "/a", "a\\b", "a/../b", "a//b", "C:x", "a\0"] {
            assert!(!safe_path(p));
        }
        assert!(safe_path("scripts/main.luau"));
    }
}
