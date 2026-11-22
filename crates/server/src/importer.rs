//! Moving finished downloads into the library, and reading the library that
//! is already on disk.

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

pub const VIDEO_EXTS: &[&str] = &[
    "mkv", "mp4", "avi", "m4v", "mov", "wmv", "ts", "m2ts", "webm", "mpg", "mpeg",
];

pub fn is_video(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

fn is_sample(p: &Path) -> bool {
    p.file_stem().and_then(|s| s.to_str()).is_some_and(|s| {
        let s = s.to_ascii_lowercase();
        s == "sample"
            || s.ends_with(".sample")
            || s.ends_with("-sample")
            || s.starts_with("sample-")
    }) || p
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case("sample"))
}

/// The main video of a download: the file itself, or the largest video
/// file (not a sample) under the folder.
pub fn find_video(content: &Path) -> Result<PathBuf> {
    if content.is_file() {
        if is_video(content) {
            return Ok(content.to_path_buf());
        }
        bail!("{} is not a video file", content.display());
    }
    let mut best: Option<(u64, PathBuf)> = None;
    let mut stack = vec![content.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = std::fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))?;
        for e in rd.flatten() {
            let p = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                stack.push(p);
            } else if ft.is_file() && is_video(&p) && !is_sample(&p) {
                let len = e.metadata().map(|m| m.len()).unwrap_or(0);
                if best.as_ref().is_none_or(|(l, _)| len > *l) {
                    best = Some((len, p));
                }
            }
        }
    }
    best.map(|(_, p)| p)
        .with_context(|| format!("no video file in {}", content.display()))
}

/// Makes a title safe as a file name on Windows, macOS and Linux.
pub fn safe_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| match c {
            ':' => '-',
            '<' | '>' | '"' | '/' | '\\' | '|' | '?' | '*' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let squashed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let t = squashed.trim_matches(|c: char| c == '.' || c == ' ');
    if t.is_empty() {
        "Untitled".into()
    } else {
        t.to_string()
    }
}

/// `Title (Year)`, the folder and file name of a movie.
pub fn movie_name(title: &str, year: Option<i32>) -> String {
    match year {
        Some(y) => format!("{} ({y})", safe_name(title)),
        None => safe_name(title),
    }
}

/// `<root>/Title (Year)/Title (Year).ext`
pub fn movie_dest(root: &Path, title: &str, year: Option<i32>, ext: &str) -> PathBuf {
    let name = movie_name(title, year);
    root.join(&name)
        .join(format!("{name}.{}", ext.to_ascii_lowercase()))
}

/// Applies the remote → local path mapping for a path reported by the
/// download client.
pub fn map_path(p: &str, remote: &str, local: &str) -> PathBuf {
    let remote = remote.trim_end_matches(['/', '\\']);
    if !remote.is_empty()
        && let Some(rest) = p.strip_prefix(remote)
        && (rest.is_empty() || rest.starts_with(['/', '\\']))
    {
        let rest = rest.trim_start_matches(['/', '\\']).replace('\\', "/");
        return Path::new(local).join(rest);
    }
    PathBuf::from(p)
}

#[derive(Debug, PartialEq, Eq)]
pub enum Placed {
    Hardlinked,
    Copied,
}

/// Puts `src` at `dst`: a hardlink when both are on the same filesystem,
/// otherwise a copy (written next to `dst` first, then renamed, so a
/// half-copied file never shows up in the library). Replaces `dst`.
pub fn place(src: &Path, dst: &Path) -> Result<Placed> {
    let dir = dst.parent().context("destination has no folder")?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let tmp = dir.join(format!(".heistarr-{}.partial", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let how = match std::fs::hard_link(src, &tmp) {
        Ok(()) => Placed::Hardlinked,
        Err(e) => {
            tracing::info!("hardlink {} failed ({e}); copying instead", src.display());
            std::fs::copy(src, &tmp)
                .with_context(|| format!("copying {} to {}", src.display(), dir.display()))?;
            Placed::Copied
        }
    };
    if let Err(e) = std::fs::rename(&tmp, dst) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e).with_context(|| format!("moving into {}", dst.display()));
    }
    Ok(how)
}

/// Removes a previous file of the movie when an upgrade put the new one at a
/// different path (other extension).
pub fn remove_replaced(old: Option<&str>, new: &Path) {
    if let Some(old) = old {
        let old = Path::new(old);
        if old != new && old.is_file() {
            match std::fs::remove_file(old) {
                Ok(()) => tracing::info!("removed replaced file {}", old.display()),
                Err(e) => tracing::warn!("could not remove replaced file {}: {e}", old.display()),
            }
        }
    }
}

/// One entry found while scanning the movies folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// What to search the catalog for.
    pub title: String,
    pub year: Option<i32>,
    /// The video file.
    pub file: PathBuf,
    /// Folder or file name, for the report.
    pub label: String,
}

/// Reads `Title (Year)` (or a release-style name) into title and year.
pub fn parse_library_name(name: &str) -> (String, Option<i32>) {
    // "Title (1999)" and "Title (1999) {tmdb-603}"
    if let Some(open) = name.rfind('(') {
        let after = &name[open + 1..];
        if let Some(close) = after.find(')') {
            let inner = &after[..close];
            if inner.len() == 4
                && let Ok(y) = inner.parse::<i32>()
                && (1880..=2100).contains(&y)
            {
                return (name[..open].trim().to_string(), Some(y));
            }
        }
    }
    let p = heistarr_core::release::parse(name);
    (p.title, p.year)
}

/// Lists the movies in the library folder: each subfolder with a video in
/// it, and loose video files at the top.
pub fn scan(root: &Path) -> Result<Vec<Found>> {
    let mut out = Vec::new();
    let rd = std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))?;
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if let Ok(file) = find_video(&p) {
                let (title, year) = parse_library_name(&name);
                out.push(Found {
                    title,
                    year,
                    file,
                    label: name,
                });
            }
        } else if ft.is_file() && is_video(&p) && !is_sample(&p) {
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let (title, year) = parse_library_name(&stem);
            out.push(Found {
                title,
                year,
                file: p,
                label: name,
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(
            movie_name("Mission: Impossible", Some(1996)),
            "Mission- Impossible (1996)"
        );
        assert_eq!(safe_name("AC/DC? *Live*"), "AC DC Live");
        assert_eq!(safe_name("..."), "Untitled");
        assert_eq!(
            movie_dest(Path::new("/m"), "Dune", Some(2021), "MKV"),
            PathBuf::from("/m/Dune (2021)/Dune (2021).mkv")
        );
    }

    #[test]
    fn library_names() {
        assert_eq!(
            parse_library_name("The Matrix (1999)"),
            ("The Matrix".into(), Some(1999))
        );
        assert_eq!(
            parse_library_name("Blade Runner 2049 (2017) {tmdb-335984}"),
            ("Blade Runner 2049".into(), Some(2017))
        );
        assert_eq!(
            parse_library_name("Heat.1995.1080p.BluRay.x264-X"),
            ("Heat".into(), Some(1995))
        );
    }

    #[test]
    fn mapping() {
        assert_eq!(
            map_path("/downloads/a/b.mkv", "/downloads", "/data/torrents"),
            PathBuf::from("/data/torrents/a/b.mkv")
        );
        assert_eq!(
            map_path("/downloads2/x", "/downloads", "/data"),
            PathBuf::from("/downloads2/x")
        );
        assert_eq!(
            map_path("D:\\dl\\x\\y.mkv", "D:\\dl", "/mnt/dl"),
            PathBuf::from("/mnt/dl/x/y.mkv")
        );
        assert_eq!(map_path("/same", "", ""), PathBuf::from("/same"));
    }

    #[test]
    fn finds_largest_video_and_places_it() {
        let d = tempfile::tempdir().unwrap();
        let dl = d.path().join("dl/Some.Movie.2020.1080p");
        std::fs::create_dir_all(dl.join("Sample")).unwrap();
        std::fs::write(dl.join("movie.mkv"), vec![0u8; 1000]).unwrap();
        std::fs::write(dl.join("Sample/sample.mkv"), vec![0u8; 5000]).unwrap();
        std::fs::write(dl.join("extra.mkv"), vec![0u8; 10]).unwrap();
        std::fs::write(dl.join("info.nfo"), vec![0u8; 9000]).unwrap();
        let v = find_video(&dl).unwrap();
        assert_eq!(v, dl.join("movie.mkv"));

        let dst = movie_dest(&d.path().join("lib"), "Some Movie", Some(2020), "mkv");
        assert_eq!(place(&v, &dst).unwrap(), Placed::Hardlinked);
        assert_eq!(std::fs::metadata(&dst).unwrap().len(), 1000);
        // Placing again replaces.
        place(&v, &dst).unwrap();

        let found = scan(&d.path().join("lib")).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "Some Movie");
        assert_eq!(found[0].year, Some(2020));
    }
}
