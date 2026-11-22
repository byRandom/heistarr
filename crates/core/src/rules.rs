//! Pick rules: which releases are acceptable and which one wins.

use crate::release::{Language, ParsedRelease, Resolution, Source, normalize_title};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PickRules {
    /// Lowest acceptable resolution.
    pub min_resolution: Resolution,
    /// Highest wanted resolution; better ones are rejected (saves space).
    pub max_resolution: Resolution,
    /// Size limits per release, in MiB. 0 means no limit.
    pub min_size_mib: u64,
    pub max_size_mib: u64,
    /// Words that add points when present (case-insensitive, whole tokens).
    pub preferred_words: Vec<String>,
    /// Words that reject a release.
    pub blocked_words: Vec<String>,
    /// Torrents with fewer seeders are rejected.
    pub min_seeders: u32,
    pub allow_x265: bool,
    /// Wanted audio languages. Empty accepts any. A release with no language
    /// tag counts as English.
    #[serde(default)]
    pub languages: Vec<Language>,
    /// Whether `MULTi`/`DUAL` releases satisfy `languages`.
    #[serde(default = "yes")]
    pub accept_multi: bool,
    /// Replace the file when a better release appears, until the file reaches
    /// the cutoff.
    #[serde(default = "yes")]
    pub upgrades: bool,
    #[serde(default = "default_cutoff")]
    pub cutoff: Quality,
}

fn yes() -> bool {
    true
}

fn default_cutoff() -> Quality {
    Quality {
        resolution: Resolution::R1080,
        source: Source::BluRay,
    }
}

/// Resolution plus source: what upgrades and the cutoff compare.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quality {
    pub resolution: Resolution,
    pub source: Source,
}

impl Quality {
    pub fn of(r: &ParsedRelease) -> Self {
        Self {
            resolution: r.resolution,
            source: r.source,
        }
    }

    /// Orders qualities: resolution first, then source.
    pub fn rank(self) -> i32 {
        self.resolution as i32 * 100 + source_points(self.source)
    }

    pub fn label(self) -> String {
        format!("{}-{}", self.source.label(), self.resolution.label())
    }
}

/// Whether a release of quality `new` should replace a file of quality
/// `current`.
pub fn is_upgrade(rules: &PickRules, current: Quality, new: Quality) -> bool {
    rules.upgrades && current.rank() < rules.cutoff.rank() && new.rank() > current.rank()
}

impl Default for PickRules {
    fn default() -> Self {
        Self {
            min_resolution: Resolution::R720,
            max_resolution: Resolution::R1080,
            min_size_mib: 300,
            max_size_mib: 20_000,
            preferred_words: vec!["proper".into(), "repack".into()],
            blocked_words: vec!["hardcoded".into(), "hc".into(), "3d".into()],
            min_seeders: 1,
            allow_x265: true,
            languages: Vec::new(),
            accept_multi: true,
            upgrades: true,
            cutoff: default_cutoff(),
        }
    }
}

/// The outcome of checking one release against the rules.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub score: Option<i32>,
    pub rejections: Vec<String>,
}

/// What the release must be for.
pub struct Target<'a> {
    pub title: &'a str,
    pub year: Option<i32>,
}

fn source_points(s: Source) -> i32 {
    match s {
        Source::Remux => 60,
        Source::BluRay => 50,
        Source::WebDl => 45,
        Source::WebRip => 35,
        Source::Hdtv => 20,
        Source::Dvd => 10,
        Source::Unknown => 0,
        Source::Telesync | Source::Cam => -100,
    }
}

pub fn judge(
    rules: &PickRules,
    target: &Target<'_>,
    r: &ParsedRelease,
    size: u64,
    seeders: Option<u32>,
) -> Verdict {
    let mut rejections = Vec::new();

    if normalize_title(&r.title) != normalize_title(target.title) {
        rejections.push(format!("title \"{}\" does not match", r.title));
    }
    if let (Some(want), Some(got)) = (target.year, r.year)
        && (want - got).abs() > 1
    {
        rejections.push(format!("year {got} does not match {want}"));
    }
    if matches!(r.source, Source::Cam | Source::Telesync) {
        rejections.push(format!("{} source", r.source.label()));
    }
    if r.resolution == Resolution::Unknown {
        rejections.push("unknown resolution".into());
    } else if r.resolution < rules.min_resolution {
        rejections.push(format!(
            "{} is below {}",
            r.resolution.label(),
            rules.min_resolution.label()
        ));
    } else if r.resolution > rules.max_resolution {
        rejections.push(format!(
            "{} is above {}",
            r.resolution.label(),
            rules.max_resolution.label()
        ));
    }
    let mib = size / (1024 * 1024);
    if rules.min_size_mib > 0 && mib < rules.min_size_mib {
        rejections.push(format!("{mib} MiB is under {} MiB", rules.min_size_mib));
    }
    if rules.max_size_mib > 0 && mib > rules.max_size_mib {
        rejections.push(format!("{mib} MiB is over {} MiB", rules.max_size_mib));
    }
    if let Some(s) = seeders
        && s < rules.min_seeders
    {
        rejections.push(format!("{s} seeders"));
    }
    if !rules.allow_x265 && r.codec == Some("x265") {
        rejections.push("x265 not allowed".into());
    }
    if !rules.languages.is_empty() {
        let langs: &[Language] = if r.languages.is_empty() {
            &[Language::English]
        } else {
            &r.languages
        };
        let ok = langs.iter().any(|l| rules.languages.contains(l))
            || rules.accept_multi
                && langs
                    .iter()
                    .any(|l| matches!(l, Language::Multi | Language::Dual));
        if !ok {
            let names: Vec<_> = langs.iter().map(|l| l.label()).collect();
            rejections.push(format!("audio {} not wanted", names.join("/")));
        }
    }
    for w in &rules.blocked_words {
        let w = w.to_lowercase();
        if r.tokens.contains(&w) {
            rejections.push(format!("blocked word \"{w}\""));
        }
    }

    if !rejections.is_empty() {
        return Verdict {
            score: None,
            rejections,
        };
    }

    let mut score = Quality::of(r).rank();
    if r.proper {
        score += 5;
    }
    for w in &rules.preferred_words {
        if r.tokens.contains(&w.to_lowercase()) {
            score += 15;
        }
    }
    if let Some(s) = seeders {
        // Up to +10 for well-seeded torrents, log scale.
        score += (s.max(1) as f32).log2().min(10.0) as i32;
    }
    Verdict {
        score: Some(score),
        rejections,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release::parse;

    const GIB: u64 = 1024 * 1024 * 1024;

    fn verdict(name: &str, size: u64) -> Verdict {
        let t = Target {
            title: "The Matrix",
            year: Some(1999),
        };
        judge(&PickRules::default(), &t, &parse(name), size, Some(50))
    }

    #[test]
    fn prefers_bluray_over_webrip_same_resolution() {
        let a = verdict("The.Matrix.1999.1080p.BluRay.x264-A", 8 * GIB)
            .score
            .unwrap();
        let b = verdict("The.Matrix.1999.1080p.WEBRip.x264-B", 8 * GIB)
            .score
            .unwrap();
        assert!(a > b);
    }

    #[test]
    fn rejects_out_of_range() {
        assert!(
            verdict("The.Matrix.1999.2160p.BluRay.x265-A", 8 * GIB)
                .score
                .is_none()
        );
        assert!(
            verdict("The.Matrix.1999.480p.DVDRip-A", GIB)
                .score
                .is_none()
        );
        assert!(
            verdict("The.Matrix.1999.1080p.BluRay-A", 40 * GIB)
                .score
                .is_none()
        );
        assert!(
            verdict("The.Matrix.1999.1080p.HDCAM-A", 2 * GIB)
                .score
                .is_none()
        );
    }

    #[test]
    fn rejects_wrong_title_and_year() {
        assert!(
            verdict("The.Matrix.Reloaded.2003.1080p.BluRay-A", 8 * GIB)
                .score
                .is_none()
        );
        assert!(
            verdict("The.Matrix.2021.1080p.BluRay-A", 8 * GIB)
                .score
                .is_none()
        );
    }

    #[test]
    fn audio_language() {
        let rules = PickRules {
            languages: vec![Language::Spanish],
            ..PickRules::default()
        };
        let t = Target {
            title: "The Matrix",
            year: Some(1999),
        };
        let ok = |n: &str| {
            judge(&rules, &t, &parse(n), 8 * GIB, Some(5))
                .score
                .is_some()
        };
        assert!(ok("The.Matrix.1999.1080p.BluRay.CASTELLANO-A"));
        assert!(ok("The.Matrix.1999.MULTi.1080p.BluRay-A"));
        assert!(!ok("The.Matrix.1999.1080p.BluRay-A"));
        assert!(!ok("The.Matrix.1999.1080p.BluRay.LATINO-A"));
    }

    #[test]
    fn upgrades_stop_at_cutoff() {
        let r = PickRules::default();
        let q = |res, source| Quality {
            resolution: res,
            source,
        };
        assert!(is_upgrade(
            &r,
            q(Resolution::R720, Source::WebDl),
            q(Resolution::R1080, Source::WebDl)
        ));
        assert!(!is_upgrade(
            &r,
            q(Resolution::R1080, Source::WebDl),
            q(Resolution::R720, Source::BluRay)
        ));
        assert!(!is_upgrade(
            &r,
            q(Resolution::R1080, Source::BluRay),
            q(Resolution::R1080, Source::Remux)
        ));
    }

    #[test]
    fn preferred_word_bonus() {
        let a = verdict("The.Matrix.1999.1080p.BluRay.PROPER-A", 8 * GIB)
            .score
            .unwrap();
        let b = verdict("The.Matrix.1999.1080p.BluRay-A", 8 * GIB)
            .score
            .unwrap();
        assert_eq!(a - b, 15 + 5); // preferred word + proper
    }
}
