//! Runs the parser over `release_names.tsv` and reports every mismatch at
//! once, so a regression in one name is easy to spot.

use heistarr_core::release::{Language, Resolution, Source, parse};

fn resolution(s: &str) -> Resolution {
    match s {
        "480p" => Resolution::R480,
        "576p" => Resolution::R576,
        "720p" => Resolution::R720,
        "1080p" => Resolution::R1080,
        "2160p" => Resolution::R2160,
        "?" => Resolution::Unknown,
        o => panic!("bad resolution {o:?} in the corpus"),
    }
}

fn source(s: &str) -> Source {
    match s {
        "CAM" => Source::Cam,
        "TS" => Source::Telesync,
        "DVD" => Source::Dvd,
        "HDTV" => Source::Hdtv,
        "WEBRip" => Source::WebRip,
        "WEB-DL" => Source::WebDl,
        "BluRay" => Source::BluRay,
        "Remux" => Source::Remux,
        "?" => Source::Unknown,
        o => panic!("bad source {o:?} in the corpus"),
    }
}

fn languages(s: &str) -> Vec<Language> {
    if s == "-" {
        return vec![];
    }
    let mut v: Vec<Language> = s
        .split(',')
        .map(|l| match l.trim() {
            "english" => Language::English,
            "spanish" => Language::Spanish,
            "latino" => Language::Latino,
            "french" => Language::French,
            "german" => Language::German,
            "italian" => Language::Italian,
            "multi" => Language::Multi,
            "dual" => Language::Dual,
            o => panic!("bad language {o:?} in the corpus"),
        })
        .collect();
    v.sort();
    v
}

#[test]
fn release_name_corpus() {
    let data = include_str!("release_names.tsv");
    let mut failures = Vec::new();
    let mut count = 0;
    for (n, line) in data.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            cols.len(),
            6,
            "line {}: expected 6 tab-separated columns",
            n + 1
        );
        count += 1;
        let p = parse(cols[0]);
        let want_year = (cols[2] != "-").then(|| cols[2].parse::<i32>().expect("year"));
        let mut problems = Vec::new();
        if p.title != cols[1] {
            problems.push(format!("title {:?}, want {:?}", p.title, cols[1]));
        }
        if p.year != want_year {
            problems.push(format!("year {:?}, want {want_year:?}", p.year));
        }
        if p.resolution != resolution(cols[3]) {
            problems.push(format!("resolution {:?}, want {}", p.resolution, cols[3]));
        }
        if p.source != source(cols[4]) {
            problems.push(format!("source {:?}, want {}", p.source, cols[4]));
        }
        if p.languages != languages(cols[5]) {
            problems.push(format!("languages {:?}, want {}", p.languages, cols[5]));
        }
        if !problems.is_empty() {
            failures.push(format!(
                "line {}: {}\n    {}",
                n + 1,
                cols[0],
                problems.join("\n    ")
            ));
        }
    }
    assert!(count >= 40, "the corpus should not shrink");
    assert!(
        failures.is_empty(),
        "{} of {count} names parsed wrong:\n{}",
        failures.len(),
        failures.join("\n")
    );
}
