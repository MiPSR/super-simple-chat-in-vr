//! Ephemeral authoritative timelines. The game server never downloads media.
use serde::{Deserialize, Serialize};
use std::time::Instant;
#[path = "../../../shared/media_addresses.rs"]
mod addresses;
pub fn now() -> f64 {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    EPOCH.get_or_init(Instant::now).elapsed().as_secs_f64()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Surface {
    pub id: String,
    pub position: [f32; 3],
    pub size: [f32; 2],
    #[serde(default)]
    pub public_controls: bool,
}
#[derive(Serialize)]
pub struct Session {
    pub source: String,
    pub position: f64,
    pub playing: bool,
    pub rate: f64,
    pub generation: u64,
    #[serde(skip)]
    changed: Instant,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            source: String::new(),
            position: 0.0,
            playing: false,
            rate: 1.0,
            generation: 0,
            changed: Instant::now(),
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub target: String,
    pub action: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub value: f64,
}
impl Session {
    pub fn target(&self) -> f64 {
        self.position
            + if self.playing {
                self.changed.elapsed().as_secs_f64() * self.rate
            } else {
                0.0
            }
    }
    pub fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({"source":self.source,"position":self.target(),"playing":self.playing,"rate":self.rate,"generation":self.generation,"sampled_at":now()})
    }
    pub fn apply(&mut self, request: &Request) -> anyhow::Result<()> {
        // Validate before changing any timeline field.
        match request.action.as_str() {
            "load" => {
                anyhow::ensure!(
                    valid_source(&request.source),
                    "Only public HTTP(S) media sources are allowed"
                );
            }
            "play" | "pause" => {
                anyhow::ensure!(!self.source.is_empty(), "No media loaded");
            }
            "seek" => {
                anyhow::ensure!(
                    request.value.is_finite() && (0.0..=604800.0).contains(&request.value),
                    "Invalid seek"
                );
            }
            "rate" => {
                anyhow::ensure!(
                    request.value.is_finite() && (0.25..=2.0).contains(&request.value),
                    "Invalid media rate"
                );
            }
            "stop" => (),
            _ => anyhow::bail!("Unknown media action"),
        }
        self.position = self.target();
        self.changed = Instant::now();
        match request.action.as_str() {
            "load" => {
                self.source = request.source.clone();
                self.position = 0.0;
                self.playing = true;
                self.rate = 1.0;
                self.generation += 1;
            }
            "play" => self.playing = true,
            "pause" => self.playing = false,
            "seek" => self.position = request.value,
            "rate" => self.rate = request.value,
            "stop" => {
                self.source.clear();
                self.position = 0.0;
                self.playing = false;
            }
            _ => unreachable!(),
        }
        Ok(())
    }
}
pub fn valid_source(source: &str) -> bool {
    if source.len() > 4096 || source.chars().any(char::is_control) {
        return false;
    }
    let Ok(url) = reqwest::Url::parse(source) else {
        return false;
    };
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return false;
    }
    let Some(host) = url.host_str() else {
        return false;
    };
    if let Ok(ip) = host.trim_matches(['[', ']']).parse::<std::net::IpAddr>() {
        return addresses::public_ip(ip);
    }
    host.contains('.')
        && ![
            "localhost",
            ".localhost",
            ".local",
            ".internal",
            ".home",
            ".lan",
        ]
        .iter()
        .any(|suffix| host.trim_end_matches('.').ends_with(suffix))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timeline_and_sources() {
        for source in [
            "file:///etc/passwd",
            "http://127.1/a",
            "http://[::ffff:127.0.0.1]/",
            "http://10.0.0.1/",
            "https://user@example.com/",
            "http://localhost./",
            "http://100.64.0.1/",
        ] {
            assert!(!valid_source(source), "{source}");
        }
        assert!(valid_source("https://www.youtube.com/watch?v=test"));
        let mut session = Session::default();
        let request = |action: &str, value| Request {
            target: "screen".into(),
            action: action.into(),
            source: "https://example.com/video.mp4".into(),
            value,
        };
        session.apply(&request("load", 0.0)).unwrap();
        session.apply(&request("pause", 0.0)).unwrap();
        session.apply(&request("seek", 42.0)).unwrap();
        assert_eq!(session.target(), 42.0);
        assert!(session.apply(&request("seek", -1.0)).is_err());
        assert_eq!(session.target(), 42.0);
        session.apply(&request("stop", 0.0)).unwrap();
        assert!(session.source.is_empty());
    }
}
