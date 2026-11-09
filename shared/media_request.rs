//! Bounded creator/player controls for authoritative media timelines, not fetching.
use crate::media_addresses as addresses;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub target: String,
    pub action: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub value: f64,
}
impl Request {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.target.is_empty()
            || self.target.len() > 64
            || self.source.len() > 4096
            || !self.value.is_finite()
        {
            return Err("Invalid media request bounds");
        }
        match self.action.as_str() {
            "load" if !valid_source(&self.source) => {
                Err("Only public HTTP(S) media sources are allowed")
            }
            "seek" if !(0.0..=604800.0).contains(&self.value) => Err("Invalid seek"),
            "rate" if !(0.25..=2.0).contains(&self.value) => Err("Invalid media rate"),
            "load" | "seek" | "rate" | "play" | "pause" | "stop" => Ok(()),
            _ => Err("Unknown media action"),
        }
    }
}
pub fn valid_source(source: &str) -> bool {
    if source.len() > 4096 || source.chars().any(char::is_control) {
        return false;
    }
    let Ok(url) = url::Url::parse(source) else {
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
    fn creator_requests_are_bounded_public_and_typed() {
        for source in [
            "file:///etc/passwd",
            "http://127.1/",
            "http://10.0.0.1/",
            "http://[::ffff:127.0.0.1]/",
            "https://localhost./",
            "https://private.local/x",
            "https://user:secret@example.com/x",
            "edl://https://example.com/x",
            "https://example.com/\n",
        ] {
            let request = Request {
                target: "screen".into(),
                action: "load".into(),
                source: source.into(),
                value: 0.0,
            };
            assert!(request.validate().is_err(), "{source}");
        }
        let mut request = Request {
            target: "screen".into(),
            action: "load".into(),
            source: "https://example.com/video.mp4".into(),
            value: 0.0,
        };
        assert!(request.validate().is_ok());
        for (action, value, valid) in [
            ("seek", 42.0, true),
            ("seek", -1.0, false),
            ("rate", 2.0, true),
            ("rate", 3.0, false),
            ("pause", 0.0, true),
            ("exec", 0.0, false),
            ("stop", f64::NAN, false),
        ] {
            request.action = action.into();
            request.value = value;
            assert_eq!(request.validate().is_ok(), valid);
        }
        request.action = "load".into();
        request.value = 0.0;
        request.target = "x".repeat(65);
        assert!(request.validate().is_err());
        request.target = "screen".into();
        request.source = "x".repeat(4097);
        assert!(request.validate().is_err());
        assert!(
            serde_json::from_str::<Request>(
                r#"{"target":"screen","action":"stop","identity":"owner"}"#
            )
            .is_err()
        );
    }
}
