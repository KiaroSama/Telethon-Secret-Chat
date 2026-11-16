//! Desktop's portable file naming, destination and media admission rules.
use super::{
    error::{ExportError, Result},
    format::DateContext,
    model::{File, SkipReason},
    settings::{MediaSettings, Settings},
};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};
pub fn file_name_from_user_string(name: &str) -> String {
    let mut result = name
        .chars()
        .map(|c| {
            if c < ' '
                || "\u{200e}\u{200f}\u{202a}\u{202b}\u{202d}\u{202e}\u{2066}\u{2067}/\\<>:\"|?*"
                    .contains(c)
            {
                '_'
            } else {
                c
            }
        })
        .collect::<String>();
    if result.is_empty() || result.ends_with([' ', '.']) {
        result.push('_');
    }
    let lower = result.to_ascii_lowercase();
    if lower.ends_with(".lnk") || lower.ends_with(".scf") {
        result.push_str(".download");
    }
    let upper = result.to_ascii_uppercase();
    let reserved = matches!(
        upper.split('.').next().unwrap_or(""),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );
    if reserved {
        result.insert(0, '_');
    }
    result
}
pub fn prepare_relative_path(folder: &Path, suggested: &str) -> String {
    if !folder.join(suggested).exists() {
        return suggested.into();
    }
    let (base, extension) = suggested.find('.').map_or((suggested, ""), |position| {
        (&suggested[..position], &suggested[position..])
    });
    let mut attempt = 1u64;
    loop {
        let result = format!("{base} ({attempt}){extension}");
        if !folder.join(&result).exists() {
            return result;
        }
        attempt += 1;
    }
}
pub fn safe_relative_path(value: &str) -> bool {
    !value.is_empty()
        && !value.contains(['\\', ':', '\0'])
        && Path::new(value)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}
pub fn normalize_path(
    settings: &Settings,
    today: Option<&str>,
    context: &DateContext,
) -> Result<PathBuf> {
    let folder = std::path::absolute(&settings.path).map_err(|_| ExportError::Filesystem)?;
    let populated = folder.is_dir()
        && fs::read_dir(&folder)
            .map_err(|_| ExportError::Filesystem)?
            .next()
            .is_some();
    if !settings.force_sub_path && !populated {
        return Ok(folder);
    }
    let stamp = match today {
        Some(today) => today.into(),
        None => {
            let now = context.datetime(jiff::Timestamp::now().as_second())?;
            format!("{:04}-{:02}-{:02}", now.year(), now.month(), now.day())
        }
    };
    let base = format!(
        "{}Export_{stamp}",
        if settings.only_single_peer() {
            "Chat"
        } else {
            "Data"
        }
    );
    let mut index = 0;
    loop {
        let name = if index == 0 {
            base.clone()
        } else {
            format!("{base} ({index})")
        };
        let path = folder.join(name);
        if !path.is_dir() {
            return Ok(path);
        }
        index += 1;
    }
}
pub fn apply_file_policy(
    file: &mut File,
    skip_by_date: bool,
    media_type: u32,
    controlling_size: u64,
    media: &MediaSettings,
) -> bool {
    if !file.relative_path.is_empty() || file.skip_reason != SkipReason::None {
        return true;
    }
    file.skip_reason = if skip_by_date {
        SkipReason::DateLimits
    } else if !file.location.available() && file.content.is_empty() {
        SkipReason::Unavailable
    } else if media.types & media_type != media_type {
        SkipReason::FileType
    } else if controlling_size > media.size_limit {
        SkipReason::FileSize
    } else {
        return false;
    };
    true
}
pub fn mime_first_glob(mime: &str) -> &'static str {
    match mime {
        "image/webp" => return "*.webp",
        "application/x-tgsticker" => return "*.tgs",
        "application/x-tgwallpattern" => return "*.tgv",
        "application/x-tdesktop-theme" | "application/x-tgtheme-tdesktop" => {
            return "*.tdesktop-theme";
        }
        "application/x-tdesktop-palette" => return "*.tdesktop-palette",
        _ => {}
    }
    let mime = if mime == "audio/mpeg3" {
        "audio/mp3"
    } else {
        mime
    };
    super::mime_globs::first_glob(super::mime_globs::alias(mime))
}
