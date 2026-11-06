//! Validate the bounded EDL wrapper emitted by mpv's maintained yt-dlp hook.
//! libmpv still parses/plays media. Known directives may refer only to network
//! sources; escaped metadata is not interpreted as a filename.
pub fn network_source(source: &str) -> bool {
    validate(source, 0)
}
fn validate(source: &str, depth: usize) -> bool {
    if depth > 4 || source.len() > 262144 {
        return false;
    }
    if source.starts_with("http://") || source.starts_with("https://") {
        return source.len() <= 16384
            && url::Url::parse(source).is_ok_and(|u| {
                matches!(u.scheme(), "http" | "https")
                    && u.host().is_some()
                    && u.username().is_empty()
                    && u.password().is_none()
            });
    }
    let Some(source) = source.strip_prefix("edl://") else {
        return false;
    };
    let mut parser = Parser { source, at: 0 };
    let (mut entries, mut files) = (0, 0);
    while parser.at < source.len() {
        if matches!(source.as_bytes()[parser.at], b';' | b'\n') {
            parser.at += 1;
            continue;
        }
        entries += 1;
        if entries > 2048 {
            return false;
        }
        let header = if source.as_bytes()[parser.at] == b'!' {
            let start = parser.at;
            while parser.at < source.len()
                && !matches!(source.as_bytes()[parser.at], b',' | b';' | b'\n')
            {
                parser.at += 1;
            }
            Some(&source[start..parser.at])
        } else {
            None
        };
        if let Some(h) = header {
            if !matches!(
                h,
                "!new_stream"
                    | "!no_clip"
                    | "!no_chapters"
                    | "!global_tags"
                    | "!track_meta"
                    | "!mp4_dash"
                    | "!delay_open"
            ) {
                return false;
            }
            if source.as_bytes().get(parser.at) == Some(&b',') {
                parser.at += 1;
            }
        }
        let (mut params, mut has_file) = (0, false);
        while parser.at < source.len() && !matches!(source.as_bytes()[parser.at], b';' | b'\n') {
            let Some((name, value)) = parser.atom() else {
                return false;
            };
            params += 1;
            if params > 64 {
                return false;
            }
            if let Some(header) = header {
                let Some(name) = name else {
                    return false;
                };
                let valid = match header {
                    "!global_tags" => {
                        matches!(
                            name,
                            "artist"
                                | "album"
                                | "album_artist"
                                | "composer"
                                | "date"
                                | "genre"
                                | "series"
                                | "track"
                                | "title"
                                | "track_number"
                                | "uploader"
                                | "channel_url"
                                | "ytdl_playlist"
                                | "ytdl_playlist_index"
                                | "ytdl_playlist_title"
                                | "ytdl_playlist_id"
                                | "chapter"
                                | "season"
                                | "episode"
                                | "ytdl_is_live"
                                | "release_year"
                                | "ytdl_description"
                        ) && value.len() <= 16384
                    }
                    "!track_meta" => match name {
                        "title" | "lang" => value.len() <= 4096,
                        "byterate" | "index" => number(value, -1.0, 1_000_000_000.0),
                        "flags" => flags(value),
                        _ => false,
                    },
                    "!delay_open" => match name {
                        "media_type" => matches!(value, "video" | "audio" | "sub"),
                        "codec" => matches!(
                            value,
                            "null"
                                | "h264"
                                | "hevc"
                                | "vp8"
                                | "vp9"
                                | "av1"
                                | "aac"
                                | "opus"
                                | "mp3"
                                | "vorbis"
                                | "flac"
                                | "ass"
                                | "subrip"
                                | "webvtt"
                        ),
                        "w" | "h" => number(value, 0.0, 4096.0),
                        "fps" => number(value, 0.0, 240.0),
                        "samplerate" => number(value, 0.0, 192000.0),
                        "flags" => flags(value),
                        _ => false,
                    },
                    "!mp4_dash" => name == "init" && validate(value, depth + 1),
                    _ => false,
                };
                if !valid {
                    return false;
                }
            } else {
                let name = name.unwrap_or(match params {
                    1 => "file",
                    2 => "start",
                    3 => "length",
                    _ => "",
                });
                match name {
                    "file" => {
                        if has_file || !validate(value, depth + 1) {
                            return false;
                        }
                        has_file = true;
                        files += 1;
                    }
                    "start" | "length" if number(value, 0.0, 604800.0) => (),
                    _ => return false,
                }
            }
            if source.as_bytes().get(parser.at) == Some(&b',') {
                parser.at += 1;
            }
        }
        if header.is_none() && !has_file {
            return false;
        }
    }
    files > 0
}
fn flags(value: &str) -> bool {
    value.len() <= 64 && value.split('+').all(|f| matches!(f, "default" | "forced"))
}
fn number(value: &str, min: f64, max: f64) -> bool {
    value
        .parse::<f64>()
        .is_ok_and(|v| v.is_finite() && (min..=max).contains(&v))
}
struct Parser<'a> {
    source: &'a str,
    at: usize,
}
impl<'a> Parser<'a> {
    fn atom(&mut self) -> Option<(Option<&'a str>, &'a str)> {
        let bytes = self.source.as_bytes();
        let start = self.at;
        while self.at < bytes.len() && !b"=%,;\n!".contains(&bytes[self.at]) {
            self.at += 1;
        }
        let name = if bytes.get(self.at) == Some(&b'=') {
            let name = self.source.get(start..self.at)?;
            if name.is_empty() {
                return None;
            }
            self.at += 1;
            Some(name)
        } else {
            self.at = start;
            None
        };
        let value = if bytes.get(self.at) == Some(&b'%') {
            self.at += 1;
            let start = self.at;
            while bytes.get(self.at).is_some_and(u8::is_ascii_digit) {
                self.at += 1;
            }
            if self.at - start > 7 || bytes.get(self.at) != Some(&b'%') {
                return None;
            }
            let size = self.source.get(start..self.at)?.parse::<usize>().ok()?;
            self.at += 1;
            let end = self.at.checked_add(size)?;
            let value = self.source.get(self.at..end)?;
            self.at = end;
            value
        } else {
            let start = self.at;
            while self.at < bytes.len() && !b",;\n!".contains(&bytes[self.at]) {
                self.at += 1;
            }
            self.source.get(start..self.at)?
        };
        if self.at < bytes.len() && !b",;\n".contains(&bytes[self.at]) {
            return None;
        }
        Some((name, value))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn quoted(source: &str) -> String {
        format!("%{}%{source}", source.len())
    }
    #[test]
    fn bounded_network_only_edl() {
        let url = "https://example.com/video?a=1,b=2;next=3";
        let valid = format!(
            "edl://!new_stream;!no_clip;!no_chapters;{},length=635;!global_tags,title={}",
            quoted(url),
            quoted("Hello, 世界; file:///not-a-reference")
        );
        assert!(network_source(&valid));
        assert!(network_source(&format!(
            "edl://!mp4_dash,init={};{},length=1",
            quoted("https://example.com/init"),
            quoted(url)
        )));
        for bad in [
            "edl://file:///etc/passwd",
            "edl://http://example.com,file=/etc/passwd",
            "edl://!mp4_dash,init=file:///etc/passwd;http://example.com",
            "edl://!unknown,file=http://example.com;http://example.com",
            "edl://%999999999999999999999%http://example.com",
            "http:private.mp4",
            "memory://data",
            "edl://!global_tags,file=http://example.com;http://example.com",
        ] {
            assert!(!network_source(bad), "{bad}");
        }
        let mut nested = "file:///etc/passwd".to_string();
        for _ in 0..3 {
            nested = format!("edl://{}", quoted(&nested));
        }
        assert!(!network_source(&nested));
        let mut nested = url.to_string();
        for _ in 0..8 {
            nested = format!("edl://{}", quoted(&nested));
        }
        assert!(!network_source(&nested));
    }
}
