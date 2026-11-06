use std::{env, net::SocketAddr, path::PathBuf};

#[derive(Clone)]
pub struct Config {
    pub data: PathBuf,
    pub listen: SocketAddr,
    pub enet: SocketAddr,
    pub public_url: String,
    pub admin_token: String,
    pub discord_id: String,
    pub discord_secret: String,
    pub tps: u32,
    pub voice_bitrate: u32,
    pub audio_effect_max: f32,
    pub radio_channels: std::collections::BTreeSet<u16>,
    pub radio_bitrate: u32,
    pub radio_presets: Vec<String>,
    pub radio_noise: f32,
    pub radio_interference: f32,
    pub radio_filter_hz: f32,
    pub radio_ft8_interval: u64,
    pub radio_sstv_interval: u64,
}
impl Config {
    pub fn load() -> anyhow::Result<Self> {
        let get = |k: &str, d: &str| env::var(k).unwrap_or_else(|_| d.into());
        let config = Self {
            data: get("SSC_DATA", "data").into(),
            listen: get("SSC_LISTEN", "127.0.0.1:8080").parse()?,
            enet: get("SSC_ENET", "127.0.0.1:7777").parse()?,
            public_url: get("SSC_PUBLIC_URL", "http://127.0.0.1:8080"),
            admin_token: env::var("SSC_ADMIN_TOKEN").map_err(|_| {
                anyhow::anyhow!("Set SSC_ADMIN_TOKEN to a random secret of at least 32 characters")
            })?,
            discord_id: get("SSC_DISCORD_ID", ""),
            discord_secret: get("SSC_DISCORD_SECRET", ""),
            tps: get("SSC_TPS", "60").parse()?,
            voice_bitrate: get("SSC_VOICE_BITRATE", "24000").parse()?,
            audio_effect_max: get("SSC_AUDIO_EFFECT_MAX", "0.9").parse()?,
            radio_channels: get("SSC_RADIO_CHANNELS", "1,2,3,4")
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().parse())
                .collect::<Result<_, _>>()?,
            radio_bitrate: get("SSC_RADIO_BITRATE", "8000").parse()?,
            radio_presets: radio_presets(&get("SSC_RADIO_PRESETS", "standard,narrow,weak"))?,
            radio_noise: get("SSC_RADIO_NOISE", "0.15").parse()?,
            radio_interference: get("SSC_RADIO_INTERFERENCE", "0.35").parse()?,
            radio_filter_hz: get("SSC_RADIO_FILTER_HZ", "1500").parse()?,
            radio_ft8_interval: get("SSC_RADIO_FT8_INTERVAL", "2").parse()?,
            radio_sstv_interval: get("SSC_RADIO_SSTV_INTERVAL", "30").parse()?,
        };
        anyhow::ensure!(config.admin_token.len() >= 32, "admin secret too short");
        anyhow::ensure!(
            config.radio_interference.is_finite()
                && (0.0..=0.8).contains(&config.radio_interference),
            "Radio interference must be 0..0.8"
        );
        anyhow::ensure!(
            config.radio_noise.is_finite() && (0.0..=0.8).contains(&config.radio_noise),
            "Radio noise must be 0..0.8"
        );
        anyhow::ensure!(
            config.radio_filter_hz.is_finite()
                && (300.0..=3000.0).contains(&config.radio_filter_hz),
            "Radio filter must be 300..3000 Hz"
        );
        anyhow::ensure!(
            (1..=120).contains(&config.radio_ft8_interval)
                && (5..=600).contains(&config.radio_sstv_interval),
            "Radio intervals out of range"
        );
        anyhow::ensure!(
            config.radio_channels.len() <= 32
                && config.radio_channels.iter().all(|c| (1..=9999).contains(c)),
            "At most 32 radio channels in 1..9999"
        );
        anyhow::ensure!(
            (8000..=16000).contains(&config.radio_bitrate),
            "Radio bitrate must be 8000..16000"
        );
        anyhow::ensure!(
            config.audio_effect_max.is_finite() && (0.0..=1.0).contains(&config.audio_effect_max),
            "Audio effect ceiling must be 0..1"
        );
        anyhow::ensure!((1..=120).contains(&config.tps), "TPS must be 1..120");
        anyhow::ensure!(
            (8000..=64000).contains(&config.voice_bitrate),
            "Voice bitrate must be 8000..64000"
        );
        anyhow::ensure!(
            config.public_url.starts_with("https://")
                || (config.listen.ip().is_loopback()
                    && config.public_url.starts_with("http://127.0.0.1:")),
            "HTTPS public URL required except explicit loopback development"
        );
        Ok(config)
    }
}

fn radio_presets(value: &str) -> anyhow::Result<Vec<String>> {
    let presets: Vec<String> = value.split(',').map(|s| s.trim().to_owned()).collect();
    let unique: std::collections::BTreeSet<_> = presets.iter().collect();
    anyhow::ensure!(
        !presets.is_empty()
            && presets.len() <= 3
            && unique.len() == presets.len()
            && presets
                .iter()
                .all(|name| matches!(name.as_str(), "standard" | "narrow" | "weak")),
        "Radio presets must be a nonempty unique subset of standard,narrow,weak"
    );
    Ok(presets)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn radio_preset_policy_is_explicit_and_bounded() {
        assert_eq!(radio_presets("narrow, weak").unwrap(), ["narrow", "weak"]);
        for invalid in ["", "standard,standard", "ultra", "weak,", ",narrow"] {
            assert!(radio_presets(invalid).is_err());
        }
    }
}
