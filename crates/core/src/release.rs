//! Reads quality, source, year and title out of scene-style release names
//! such as `Movie.Name.2019.1080p.BluRay.x264-GROUP`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Resolution {
    Unknown,
    R480,
    R576,
    R720,
    R1080,
    R2160,
}

impl Resolution {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "?",
            Self::R480 => "480p",
            Self::R576 => "576p",
            Self::R720 => "720p",
            Self::R1080 => "1080p",
            Self::R2160 => "2160p",
        }
    }

    pub const ALL: [Self; 5] = [Self::R480, Self::R576, Self::R720, Self::R1080, Self::R2160];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Source {
    Unknown,
    Cam,
    Telesync,
    Dvd,
    Hdtv,
    WebRip,
    WebDl,
    BluRay,
    Remux,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "?",
            Self::Cam => "CAM",
            Self::Telesync => "TS",
            Self::Dvd => "DVD",
            Self::Hdtv => "HDTV",
            Self::WebRip => "WEBRip",
            Self::WebDl => "WEB-DL",
            Self::BluRay => "BluRay",
            Self::Remux => "Remux",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedRelease {
    pub title: String,
    pub year: Option<i32>,
    pub resolution: Resolution,
    pub source: Source,
    pub codec: Option<&'static str>,
    pub hdr: bool,
    pub proper: bool,
    pub group: Option<String>,
    /// Lowercased tokens, for word rules.
    pub tokens: Vec<String>,
}

impl ParsedRelease {
    pub fn quality_label(&self) -> String {
        match (self.source, self.resolution) {
            (Source::Unknown, r) => r.label().to_string(),
            (s, Resolution::Unknown) => s.label().to_string(),
            (s, r) => format!("{}-{}", s.label(), r.label()),
        }
    }
}

fn is_sep(c: char) -> bool {
    matches!(c, '.' | ' ' | '_' | '-' | '[' | ']' | '(' | ')' | '+')
}

/// Normalises a title for comparison: lowercase alphanumerics, single spaces,
/// `&` read as `and`.
pub fn normalize_title(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for word in s
        .split(|c: char| !c.is_alphanumeric() && c != '&' && c != '\'')
        .filter(|w| !w.is_empty())
    {
        let w: String = word.chars().filter(|c| *c != '\'').collect();
        let w = if w == "&" { "and".to_string() } else { w.to_lowercase() };
        if w.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&w);
    }
    out
}

pub fn parse(name: &str) -> ParsedRelease {
    let group = name
        .rsplit_once('-')
        .map(|(_, g)| g.trim())
        .filter(|g| !g.is_empty() && !g.contains(['.', ' ']) && g.len() <= 20)
        .map(str::to_string);

    let raw: Vec<&str> = name.split(is_sep).filter(|t| !t.is_empty()).collect();
    let tokens: Vec<String> = raw.iter().map(|t| t.to_lowercase()).collect();
    let has = |t: &str| tokens.iter().any(|x| x == t);
    let has_pair = |a: &str, b: &str| tokens.windows(2).any(|w| w[0] == a && w[1] == b);

    let resolution = if has("2160p") || has("4k") || has("uhd") {
        Resolution::R2160
    } else if has("1080p") || has("1080i") {
        Resolution::R1080
    } else if has("720p") {
        Resolution::R720
    } else if has("576p") {
        Resolution::R576
    } else if has("480p") || has("sd") {
        Resolution::R480
    } else {
        Resolution::Unknown
    };

    let source = if has("remux") {
        Source::Remux
    } else if has("bluray") || has("blu") && has("ray") || has("bdrip") || has("brrip") {
        Source::BluRay
    } else if has("webdl") || has_pair("web", "dl") || has("amzn") || has("nf") || has("dsnp") {
        Source::WebDl
    } else if has("webrip") || has("web") {
        Source::WebRip
    } else if has("hdtv") {
        Source::Hdtv
    } else if has("dvdrip") || has("dvd") || has("dvdr") {
        Source::Dvd
    } else if has("ts") || has("telesync") || has("hdts") {
        Source::Telesync
    } else if has("cam") || has("camrip") || has("hdcam") {
        Source::Cam
    } else {
        Source::Unknown
    };

    let codec = if has("x265") || has("h265") || has("hevc") {
        Some("x265")
    } else if has("x264") || has("h264") || has("avc") {
        Some("x264")
    } else if has("av1") {
        Some("AV1")
    } else {
        None
    };

    let hdr = has("hdr") || has("hdr10") || has("dv") || has("dovi");
    let proper = has("proper") || has("repack");

    // The title is everything before the last plausible year, or before the
    // first quality token when there is no year.
    let year_pos = raw.iter().enumerate().skip(1).rev().find_map(|(i, t)| {
        let y: i32 = t.parse().ok()?;
        (t.len() == 4 && (1900..=2100).contains(&y)).then_some((i, y))
    });
    let title_end = match year_pos {
        Some((i, _)) => i,
        None => tokens
            .iter()
            .position(|t| {
                matches!(
                    t.as_str(),
                    "2160p" | "1080p" | "1080i" | "720p" | "576p" | "480p" | "bluray" | "webrip"
                        | "webdl" | "web" | "hdtv" | "dvdrip" | "remux" | "x264" | "x265" | "hdcam" | "cam" | "camrip" | "hdts"
                )
            })
            .unwrap_or(raw.len()),
    };
    let title = raw[..title_end].join(" ");

    ParsedRelease {
        title,
        year: year_pos.map(|(_, y)| y),
        resolution,
        source,
        codec,
        hdr,
        proper,
        group,
        tokens,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_name() {
        let r = parse("The.Matrix.1999.1080p.BluRay.x264-GROUP");
        assert_eq!(r.title, "The Matrix");
        assert_eq!(r.year, Some(1999));
        assert_eq!(r.resolution, Resolution::R1080);
        assert_eq!(r.source, Source::BluRay);
        assert_eq!(r.codec, Some("x264"));
        assert_eq!(r.group.as_deref(), Some("GROUP"));
        assert_eq!(r.quality_label(), "BluRay-1080p");
    }

    #[test]
    fn year_in_title() {
        let r = parse("Blade.Runner.2049.2017.2160p.UHD.BluRay.REMUX.HDR.HEVC-X");
        assert_eq!(r.title, "Blade Runner 2049");
        assert_eq!(r.year, Some(2017));
        assert_eq!(r.resolution, Resolution::R2160);
        assert_eq!(r.source, Source::Remux);
        assert!(r.hdr);
    }

    #[test]
    fn web_dl_with_spaces() {
        let r = parse("Dune Part Two (2024) 1080p WEB-DL DDP5.1 H.264");
        assert_eq!(r.title, "Dune Part Two");
        assert_eq!(r.year, Some(2024));
        assert_eq!(r.source, Source::WebDl);
    }

    #[test]
    fn cam_and_no_year() {
        let r = parse("Some.Movie.HDCAM.x264");
        assert_eq!(r.source, Source::Cam);
        assert_eq!(r.year, None);
        assert_eq!(r.title, "Some Movie");
    }

    #[test]
    fn normalize() {
        assert_eq!(normalize_title("Fast & Furious"), "fast and furious");
        assert_eq!(normalize_title("Schindler's List"), "schindlers list");
        assert_eq!(normalize_title("Spider-Man: No Way Home"), "spider man no way home");
    }
}
