//! File and folder names for saved clips, built from the user's templates.

use std::path::{Path, PathBuf};

use crate::settings::StoragePrefs;

#[derive(Clone, Copy, Debug)]
pub struct DateTime {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
}

/// Replaces the placeholders in a template. Unknown `{words}` are left as typed.
pub fn render(template: &str, game: &str, now: DateTime) -> String {
    template
        .replace("{game}", game)
        .replace(
            "{date}",
            &format!("{:04}-{:02}-{:02}", now.year, now.month, now.day),
        )
        .replace(
            "{time}",
            &format!("{:02}-{:02}-{:02}", now.hour, now.minute, now.second),
        )
        .replace("{year}", &format!("{:04}", now.year))
        .replace("{month}", &format!("{:02}", now.month))
        .replace("{day}", &format!("{:02}", now.day))
}

/// Makes text safe to use as one path segment on Windows.
pub fn sanitize_segment(text: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    // Windows drops trailing dots and spaces from names, which can make two names collide.
    let trimmed = cleaned.trim().trim_end_matches('.').trim();
    let name = if trimmed.is_empty() || trimmed == ".." {
        "Clip"
    } else {
        trimmed
    };
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let stem = name.split('.').next().unwrap_or(name);
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        format!("_{name}")
    } else {
        name.chars().take(120).collect()
    }
}

/// The path a new clip should be saved to. A number is added if the name is taken.
pub fn clip_path(storage: &StoragePrefs, game: &str, now: DateTime) -> PathBuf {
    media_path(storage, game, now, "mp4")
}

/// Like [`clip_path`], with another extension (screenshots are `png` or `jpg`).
pub fn media_path(storage: &StoragePrefs, game: &str, now: DateTime, extension: &str) -> PathBuf {
    let mut dir = PathBuf::from(&storage.folder);
    if !storage.folder_template.trim().is_empty() {
        let rendered = render(&storage.folder_template, game, now);
        // The template may contain `/` or `\` to nest folders; each part is cleaned separately.
        for part in rendered.split(['/', '\\']) {
            if part.trim().is_empty() {
                continue;
            }
            dir.push(sanitize_segment(part));
        }
    }
    let name = sanitize_segment(&render(&storage.name_template, game, now));
    unique(&dir, &name, extension)
}

/// Where a recording goes: like a clip, with `label` (such as `session`) after the name.
pub fn recording_path(storage: &StoragePrefs, game: &str, now: DateTime, label: &str) -> PathBuf {
    suffixed_path(storage, game, now, &format!(" ({label})"))
}

/// Where an auto-clip goes: like a clip, named after what happened (`… - Triple kill`).
pub fn event_clip_path(storage: &StoragePrefs, game: &str, now: DateTime, label: &str) -> PathBuf {
    suffixed_path(
        storage,
        game,
        now,
        &format!(" - {}", sanitize_segment(label)),
    )
}

fn suffixed_path(storage: &StoragePrefs, game: &str, now: DateTime, suffix: &str) -> PathBuf {
    let clip = media_path(storage, game, now, "mp4");
    let dir = clip.parent().map(Path::to_path_buf).unwrap_or_default();
    let stem = clip
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    unique(&dir, &format!("{stem}{suffix}"), "mp4")
}

fn unique(dir: &Path, name: &str, extension: &str) -> PathBuf {
    let first = dir.join(format!("{name}.{extension}"));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| dir.join(format!("{name} ({n}).{extension}")))
        .find(|p| !p.exists())
        .unwrap()
}

/// A readable name for a program: `eldenring.exe` becomes `eldenring`.
pub fn game_name_from_exe(exe: &str) -> String {
    // A detected game's proper name ("World of Warships" for WorldOfWarships64.exe).
    if let Some(name) = crate::gamedb::known_name(exe.rsplit(['/', '\\']).next().unwrap_or(exe)) {
        return name;
    }
    let stem = exe.rsplit(['/', '\\']).next().unwrap_or(exe);
    let stem = stem
        .strip_suffix(".exe")
        .or_else(|| stem.strip_suffix(".EXE"))
        .unwrap_or(stem);
    stem.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: DateTime = DateTime {
        year: 2026,
        month: 9,
        day: 30,
        hour: 18,
        minute: 5,
        second: 42,
    };

    #[test]
    fn placeholders_are_replaced() {
        assert_eq!(
            render("{game} {date} {time}", "Apex", NOW),
            "Apex 2026-09-30 18-05-42"
        );
        assert_eq!(render("{year}/{month}/{day}", "x", NOW), "2026/09/30");
        assert_eq!(render("{unknown}", "x", NOW), "{unknown}");
    }

    #[test]
    fn unsafe_characters_are_removed() {
        assert_eq!(sanitize_segment("a:b*c?"), "a-b-c-");
        assert_eq!(sanitize_segment(".."), "Clip");
        assert_eq!(sanitize_segment("  "), "Clip");
        assert_eq!(sanitize_segment("CON"), "_CON");
        assert_eq!(sanitize_segment("name..."), "name");
    }

    #[test]
    fn a_template_cannot_escape_the_clip_folder() {
        let storage = StoragePrefs {
            folder: "C:\\Clips".into(),
            folder_template: "..\\..\\{game}".into(),
            name_template: "{date}".into(),
            ..StoragePrefs::default()
        };
        let path = clip_path(&storage, "Apex", NOW);
        assert!(path.starts_with("C:\\Clips"), "{path:?}");
        assert!(!path.to_string_lossy().contains(".."), "{path:?}");
    }

    #[test]
    fn game_names_drop_the_extension() {
        assert_eq!(game_name_from_exe("eldenring.exe"), "eldenring");
        assert_eq!(game_name_from_exe("C:\\Games\\Apex.EXE"), "Apex");
    }
}
