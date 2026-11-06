//! Global transient radio routing; no modem waveform is sent through lossy Opus.
use crate::{App, db};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, HashMap},
    time::{Duration, Instant},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    target: String,
    mode: String,
    data: String,
}
impl Request {
    pub fn digital(target: String, mode: &str, data: String) -> anyhow::Result<Self> {
        anyhow::ensure!(
            !target.is_empty()
                && target.len() <= 64
                && matches!(mode, "ft8" | "sstv")
                && valid_payload(mode, &data, 0),
            "Invalid protected radio payload"
        );
        Ok(Self {
            target,
            mode: mode.into(),
            data,
        })
    }
    pub fn is_voice(&self) -> bool {
        self.mode == "voice"
    }
}
struct Usage {
    voice_at: Option<Instant>,
    voice_window: Instant,
    frames: u32,
    ft8: Option<Instant>,
    sstv: Option<Instant>,
}
impl Default for Usage {
    fn default() -> Self {
        Self {
            voice_at: None,
            voice_window: Instant::now(),
            frames: 0,
            ft8: None,
            sstv: None,
        }
    }
}
#[derive(Default)]
pub struct Radios {
    usage: HashMap<String, Usage>,
    sequence: u64,
    channels: HashMap<u16, (String, Instant)>,
}
fn valid_payload(mode: &str, data: &str, bitrate: u32) -> bool {
    match mode {
        "voice" => hex::decode(data).ok().is_some_and(|p| {
            !p.is_empty()
                && p.len() <= bitrate as usize / 400 + 64
                && opus::packet::get_nb_samples(&p, 48000).ok() == Some(960)
        }),
        "ft8" => {
            !data.is_empty() && data.len() <= 40 && data.bytes().all(|c| (32..=126).contains(&c))
        }
        "sstv" => data.len() == 64 * 48 * 2 && hex::decode(data).is_ok(),
        _ => false,
    }
}
impl Radios {
    fn activity(&mut self, channel: u16, actor: &str, now: Instant, duration: Duration) -> bool {
        let interference = self
            .channels
            .get(&channel)
            .is_some_and(|(last, until)| last != actor && *until > now);
        self.channels
            .insert(channel, (actor.into(), now + duration));
        interference
    }
    pub fn transmitting(&self, user: &str) -> bool {
        self.usage
            .get(user)
            .and_then(|u| u.voice_at)
            .is_some_and(|at| at.elapsed() < Duration::from_millis(250))
    }
    pub fn clean(&mut self, online: &BTreeSet<String>) {
        // Disconnect/reconnect must not reset digital transmission cooldowns.
        self.usage.retain(|user, usage| {
            online.contains(user)
                || [usage.voice_at, usage.ft8, usage.sstv]
                    .into_iter()
                    .flatten()
                    .any(|at| at.elapsed() < Duration::from_secs(600))
        });
        self.channels
            .retain(|_, (_, until)| *until > Instant::now());
    }
    pub fn transmit(
        &mut self,
        app: &App,
        instance: &str,
        actor: &str,
        request: Request,
    ) -> anyhow::Result<Vec<(String, String, Value)>> {
        anyhow::ensure!(
            valid_payload(&request.mode, &request.data, app.config.radio_bitrate),
            "Invalid radio payload"
        );
        let worlds = app.worlds.lock().unwrap();
        let origin = worlds
            .instances
            .get(instance)
            .ok_or_else(|| anyhow::anyhow!("Instance ended"))?;
        let source = origin
            .manifest
            .radios
            .iter()
            .find(|r| r.id == request.target)
            .ok_or_else(|| anyhow::anyhow!("No such radio"))?;
        anyhow::ensure!(
            app.config.radio_channels.contains(&source.channel),
            "Radio channel disabled by administrator"
        );
        let player = origin
            .players
            .get(actor)
            .ok_or_else(|| anyhow::anyhow!("Not joined"))?;
        anyhow::ensure!(
            (0..3)
                .map(|j| (player.pose[j] - source.position[j]).powi(2))
                .sum::<f32>()
                <= 9.0,
            "Radio is out of reach"
        );
        if request.mode == "voice" {
            anyhow::ensure!(
                origin.manifest.allow_voice_bypass,
                "World forbids radio voice bypass"
            );
        }
        anyhow::ensure!(
            self.usage.contains_key(actor) || self.usage.len() < 1024,
            "Radio counter budget reached"
        );
        let usage = self.usage.entry(actor.into()).or_default();
        let now = Instant::now();
        if request.mode == "voice" {
            if usage.voice_window.elapsed() >= Duration::from_secs(1) {
                usage.voice_window = now;
                usage.frames = 0;
            }
            anyhow::ensure!(usage.frames < 55, "Radio voice rate exceeded");
            usage.frames += 1;
            usage.voice_at = Some(now);
        } else {
            let (last, interval) = if request.mode == "sstv" {
                (&mut usage.sstv, app.config.radio_sstv_interval)
            } else {
                (&mut usage.ft8, app.config.radio_ft8_interval)
            };
            anyhow::ensure!(
                last.is_none_or(|last| last.elapsed() >= Duration::from_secs(interval)),
                "Radio transmission rate exceeded"
            );
            *last = Some(now);
        }
        self.sequence += 1;
        let interference = self.activity(
            source.channel,
            actor,
            now,
            Duration::from_millis(if request.mode == "voice" {
                250
            } else if request.mode == "ft8" {
                1000
            } else {
                2000
            }),
        );
        let db = app.db.lock().unwrap();
        let mut deliveries = Vec::new();
        for (id, destination) in &worlds.instances {
            if request.mode == "voice" && !destination.manifest.allow_voice_bypass {
                continue;
            }
            for receiver in &destination.manifest.radios {
                if receiver.channel != source.channel {
                    continue;
                }
                for listener in destination.players.values() {
                    if listener.id == actor
                        || db::blocked(
                            &db,
                            actor,
                            &listener.id,
                            if request.mode == "voice" { 1 } else { 2 },
                        )
                    {
                        continue;
                    }
                    if request.mode == "voice"
                        && (0..3)
                            .map(|j| (listener.pose[j] - receiver.position[j]).powi(2))
                            .sum::<f32>()
                            > 900.0
                    {
                        continue;
                    }
                    deliveries.push((id.clone(),listener.id.clone(),json!({"radio":receiver.id,"channel":source.channel,"sender":actor,"mode":request.mode,"data":request.data,"sequence":self.sequence,"interference":interference})));
                }
            }
        }
        Ok(deliveries)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cooldowns_survive_disconnect_but_expire() {
        let mut radios = Radios::default();
        radios.usage.insert(
            "user".into(),
            Usage {
                ft8: Some(Instant::now()),
                ..Usage::default()
            },
        );
        radios.clean(&BTreeSet::new());
        assert!(radios.usage.contains_key("user"));
        radios.usage.get_mut("user").unwrap().ft8 = Some(Instant::now() - Duration::from_secs(601));
        radios.clean(&BTreeSet::new());
        assert!(radios.usage.is_empty());
    }
    #[test]
    fn channel_interference_is_transient_and_isolated() {
        let mut radios = Radios::default();
        let now = Instant::now();
        let duration = Duration::from_secs(1);
        assert!(!radios.activity(1, "a", now, duration));
        assert!(radios.activity(1, "b", now + Duration::from_millis(100), duration));
        assert!(!radios.activity(2, "b", now + Duration::from_millis(100), duration));
        assert!(!radios.activity(1, "c", now + Duration::from_secs(2), duration));
    }
    #[test]
    fn protected_payload_budgets() {
        assert!(valid_payload("ft8", "CQ TEST FROM VR", 8000));
        assert!(!valid_payload("ft8", "CQ\nINJECT", 8000));
        assert!(!valid_payload("ft8", &"A".repeat(41), 8000));
        assert!(valid_payload("sstv", &"ab".repeat(64 * 48), 8000));
        assert!(!valid_payload("sstv", &"zz".repeat(64 * 48), 8000));
        assert!(!valid_payload("voice", "zz", 8000));
    }
}
