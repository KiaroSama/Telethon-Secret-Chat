//! Export saved history as a plaintext Telegram Desktop-style archive.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{self, Read},
    path::{Component, Path, PathBuf},
};

use chrono::{DateTime, Utc};
use serde_json::{Map, Value, json};

pub use crate::export::DecodedImageBudget;

use crate::{
    autosave::{SavedMediaRecord, SavedSecretMessage},
    errors::SecretChatError,
    export::{escape_html_text, safe_http_href},
    media::{SecretMediaKind, parameter, safe_filename, write_media_atomic},
    ported::tdexport_qt_jpeg::desktop_thumb,
};

const FALLBACK_CSS: &str = "body{margin:1rem auto;padding:0 1rem;max-width:50rem;color:#191919;background:#fff;font:1rem/1.5 system-ui,sans-serif}a{color:#145587}a:focus-visible{outline:2px solid #145587;outline-offset:2px}.pagination{display:flex;flex-wrap:wrap;gap:1rem}.pagination a{min-height:44px;display:inline-flex;align-items:center}.message,.service{border-bottom:1px solid #aaa;padding:.75rem 0}.text{overflow-wrap:anywhere}img,video,audio{max-width:100%;height:auto}";
const FALLBACK_JS: &str = "/* Page and reply links use standard HTML navigation. */\n";
const THUMB_SOURCE_LIMIT: u64 = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportFormat {
    Html,
    Json,
    HtmlAndJson,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportMediaFilter {
    pub photos: bool,
    pub videos: bool,
    pub voice_messages: bool,
    pub video_messages: bool,
    pub stickers: bool,
    pub gifs: bool,
    pub files: bool,
    pub size_limit_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportOptions {
    pub chat_id: i32,
    pub chat_title: String,
    pub peer_id: i64,
    pub format: ExportFormat,
    pub output_dir: PathBuf,
    pub assets_dir: Option<PathBuf>,
    pub media_root: Option<PathBuf>,
    pub media_filter: ExportMediaFilter,
    pub date_from_unix: Option<i64>,
    pub date_to_unix: Option<i64>,
    pub messages_per_page: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExportReport {
    pub output_dir: PathBuf,
    pub html_pages: Vec<PathBuf>,
    pub json_path: Option<PathBuf>,
    pub copied_media: usize,
    pub skipped_media: usize,
    pub exported_messages: usize,
}

struct ExportedMessage<'a> {
    message: &'a SavedSecretMessage,
    date: DateTime<Utc>,
    attachment: Attachment,
}

struct ArchiveStage {
    path: PathBuf,
    published: bool,
}

impl ArchiveStage {
    fn publish(mut self, destination: &Path) -> Result<(), SecretChatError> {
        // A trailing separator or '.' denotes the same directory, but rename
        // can reject that spelling when the destination does not yet exist.
        let destination: PathBuf = destination.components().collect();
        let destination = destination.as_path();
        let exists = verify_empty_output(destination)?;
        // Windows cannot rename a directory over an existing empty directory.
        // Remove only that verified-empty destination immediately before publish.
        #[cfg(windows)]
        if exists {
            fs::remove_dir(destination)
                .map_err(|_| parameter("could not prepare empty archive destination"))?;
        }
        #[cfg(not(windows))]
        let _ = exists;
        if fs::rename(&self.path, destination).is_err() {
            #[cfg(windows)]
            if exists {
                let _ = fs::create_dir(destination);
            }
            return Err(parameter("could not publish completed archive"));
        }
        self.published = true;
        Ok(())
    }
}

impl Drop for ArchiveStage {
    fn drop(&mut self) {
        if !self.published {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

#[derive(Default)]
enum Attachment {
    #[default]
    None,
    Skipped(&'static str),
    Copied {
        path: String,
        thumb: Option<String>,
    },
}

/// Writes plaintext to a new or empty output directory. Build failures discard
/// a private staging directory before any completed archive is published. Keep
/// the destination's parent private and supply only trusted assets. Publication
/// is not a crash-durability guarantee; newly created parent directories can remain.
pub fn export_saved_chat(
    messages: &[SavedSecretMessage],
    options: &ExportOptions,
) -> Result<ExportReport, SecretChatError> {
    export_saved_chat_with(messages, options, |_| Ok(()))
}

/// Export an archive with an opt-in JPEG thumbnail budget.
/// An image outside the budget is copied as usual but gets no thumbnail.
pub fn export_saved_chat_with_image_budget(
    messages: &[SavedSecretMessage],
    options: &ExportOptions,
    budget: DecodedImageBudget,
) -> Result<ExportReport, SecretChatError> {
    export_saved_chat_inner(messages, options, |_| Ok(()), Some(budget))
}

pub(crate) fn export_saved_chat_with(
    messages: &[SavedSecretMessage],
    options: &ExportOptions,
    write_extra: impl FnOnce(&Path) -> Result<(), SecretChatError>,
) -> Result<ExportReport, SecretChatError> {
    export_saved_chat_inner(messages, options, write_extra, None)
}

fn export_saved_chat_inner(
    messages: &[SavedSecretMessage],
    options: &ExportOptions,
    write_extra: impl FnOnce(&Path) -> Result<(), SecretChatError>,
    budget: Option<DecodedImageBudget>,
) -> Result<ExportReport, SecretChatError> {
    let html = options.format != ExportFormat::Json;
    if html && options.messages_per_page == 0 {
        return Err(parameter("messages per page must be greater than zero"));
    }
    if options
        .date_from_unix
        .zip(options.date_to_unix)
        .is_some_and(|(from, to)| from > to)
    {
        return Err(parameter("invalid archive date range"));
    }
    verify_output_path(&options.output_dir)?;
    let mut selected: Vec<_> = messages
        .iter()
        .filter(|msg| {
            msg.chat_id == options.chat_id
                && options
                    .date_from_unix
                    .is_none_or(|start| msg.date_unix >= start)
                && options.date_to_unix.is_none_or(|end| msg.date_unix <= end)
        })
        .collect();
    selected.sort_unstable_by_key(|msg| (msg.date_unix, msg.id));
    let mut ids = BTreeSet::new();
    let mut entries = Vec::with_capacity(selected.len());
    for message in selected {
        if !ids.insert(message.id) {
            return Err(parameter("archive contains duplicate message IDs"));
        }
        if let Some(relative) = message
            .media
            .as_ref()
            .and_then(|m| m.local_rel_path.as_deref())
        {
            verify_media_relative(relative)?;
        }
        let date = DateTime::from_timestamp(message.date_unix, 0)
            .ok_or_else(|| parameter("archive message date is out of range"))?;
        entries.push(ExportedMessage {
            message,
            date,
            attachment: Attachment::None,
        });
    }
    let stage = prepare_output(&options.output_dir)?;
    let mut staged_options = options.clone();
    staged_options.output_dir = stage.path.clone();
    let mut report = ExportReport {
        output_dir: options.output_dir.clone(),
        html_pages: Vec::new(),
        json_path: None,
        copied_media: 0,
        skipped_media: 0,
        exported_messages: entries.len(),
    };
    if html {
        write_assets(&staged_options)?;
    }
    for entry in &mut entries {
        if let Some(media) = &entry.message.media {
            entry.attachment = copy_media(media, &staged_options, entry.message.id, html, budget)?;
            match entry.attachment {
                Attachment::Copied { .. } => report.copied_media += 1,
                Attachment::Skipped(_) => report.skipped_media += 1,
                Attachment::None => {}
            }
        }
    }
    if html {
        report.html_pages = write_pages(&staged_options, &entries)?
            .into_iter()
            .map(|path| {
                path.strip_prefix(&stage.path)
                    .map(|relative| options.output_dir.join(relative))
                    .map_err(|_| parameter("invalid archive page destination"))
            })
            .collect::<Result<_, _>>()?;
    }
    if options.format != ExportFormat::Html {
        let path = stage.path.join("result.json");
        let value = json!({
            "name": options.chat_title,
            "type": "personal_chat",
            "id": options.peer_id,
            "messages": entries.iter().map(json_message).collect::<Vec<_>>(),
        });
        let text = serde_json::to_string_pretty(&value)
            .map_err(|_| parameter("could not serialize archive JSON"))?;
        fs::write(&path, text).map_err(|_| parameter("could not write archive JSON"))?;
        report.json_path = Some(options.output_dir.join("result.json"));
    }
    write_extra(&stage.path)?;
    stage.publish(&options.output_dir)?;
    Ok(report)
}

fn verify_output_path(path: &Path) -> Result<(), SecretChatError> {
    if !path
        .components()
        .any(|part| matches!(part, Component::Normal(_)))
        || path.components().any(|part| part == Component::ParentDir)
        || (!path.is_absolute()
            && matches!(
                path.components().next(),
                Some(Component::Prefix(_) | Component::RootDir)
            ))
    {
        return Err(parameter("unsafe archive output directory"));
    }
    Ok(())
}

fn verify_empty_output(path: &Path) -> Result<bool, SecretChatError> {
    if path
        .ancestors()
        .any(|part| fs::symlink_metadata(part).is_ok_and(|info| info.is_symlink()))
    {
        return Err(parameter("archive output path must not follow symlinks"));
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if !metadata.is_dir() => {
            return Err(parameter("archive output path is not a directory"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(parameter("could not inspect archive output directory")),
    }
    let mut entries =
        fs::read_dir(path).map_err(|_| parameter("could not inspect archive output directory"))?;
    if entries.next().is_some() {
        return Err(parameter("archive output directory is not empty"));
    }
    Ok(true)
}

fn prepare_output(path: &Path) -> Result<ArchiveStage, SecretChatError> {
    verify_empty_output(path)?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut dirs = fs::DirBuilder::new();
    dirs.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        dirs.mode(0o700);
    }
    dirs.create(parent)
        .map_err(|_| parameter("could not create archive parent directory"))?;
    dirs.recursive(false);
    for _ in 0..10 {
        let mut random = [0; 16];
        getrandom::fill(&mut random).map_err(|_| SecretChatError::EntropyUnavailable {
            reason: "could not generate archive staging name".into(),
        })?;
        let path = parent.join(format!(".archive-{}.tmp", hex::encode(random)));
        match dirs.create(&path) {
            Ok(()) => {
                return Ok(ArchiveStage {
                    path,
                    published: false,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) => return Err(parameter("could not create archive staging directory")),
        }
    }
    Err(parameter("could not allocate archive staging directory"))
}

fn verify_media_relative(path: &str) -> Result<(), SecretChatError> {
    if !path.split('/').all(safe_filename) || Path::new(path).is_absolute() {
        return Err(parameter("unsafe saved media relative path"));
    }
    Ok(())
}

fn write_assets(options: &ExportOptions) -> Result<(), SecretChatError> {
    let root = &options.output_dir;
    for (relative, fallback) in [
        ("css/style.css", FALLBACK_CSS),
        ("js/script.js", FALLBACK_JS),
    ] {
        let relative = Path::new(relative);
        let parent = relative
            .parent()
            .ok_or_else(|| parameter("invalid archive asset path"))?;
        fs::create_dir(root.join(parent))
            .map_err(|_| parameter("could not create archive asset directory"))?;
        let dest = root.join(relative);
        let result = if let Some(source) = &options.assets_dir {
            let source = source.join(relative);
            if !fs::symlink_metadata(&source).is_ok_and(|info| info.is_file()) {
                return Err(parameter("missing or invalid archive CSS or script asset"));
            }
            copy_asset(&source, &dest)
        } else {
            fs::write(dest, fallback)
        };
        result.map_err(|_| parameter("could not write archive stylesheet or script"))?;
    }
    if let Some(source) = &options.assets_dir {
        let source = source.join("images");
        let items = match fs::read_dir(&source) {
            Ok(items) => items,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(parameter("could not read archive image assets")),
        };
        let dest = root.join("images");
        fs::create_dir(&dest).map_err(|_| parameter("could not create archive image directory"))?;
        for item in items {
            let item = item.map_err(|_| parameter("could not read archive image asset"))?;
            if item.file_type().is_ok_and(|kind| kind.is_file()) {
                copy_asset(&item.path(), &dest.join(item.file_name()))
                    .map_err(|_| parameter("could not copy archive image asset"))?;
            }
        }
    }
    Ok(())
}

fn copy_asset(source: &Path, destination: &Path) -> io::Result<()> {
    // Copy bytes without inheriting read-only source permissions, so a failed
    // export can remove every staged file on Windows as well as Unix.
    let mut input = File::open(source)?;
    let mut output = File::create_new(destination)?;
    io::copy(&mut input, &mut output).map(|_| ())
}

fn media_folder(kind: SecretMediaKind, filter: &ExportMediaFilter) -> (&'static str, bool) {
    use SecretMediaKind::*;
    match kind {
        Photo => ("photos", filter.photos),
        Video => ("videos", filter.videos),
        VoiceNote => ("voice_messages", filter.voice_messages),
        VideoNote => ("video_messages", filter.video_messages),
        Sticker => ("stickers", filter.stickers),
        Gif => ("video_files", filter.gifs),
        Audio | Document => ("files", filter.files),
    }
}

fn copy_media(
    media: &SavedMediaRecord,
    options: &ExportOptions,
    id: i64,
    html: bool,
    budget: Option<DecodedImageBudget>,
) -> Result<Attachment, SecretChatError> {
    const FILE_UNAVAILABLE: &str = "(File unavailable, please try again later)";
    const OVER_LIMIT: &str =
        "(File exceeds maximum size. Change data exporting settings to download.)";
    let (folder, allowed) = media_folder(media.kind, &options.media_filter);
    if !allowed {
        return Ok(Attachment::Skipped(
            "(File not included. Change data exporting settings to download.)",
        ));
    }
    let limit = options.media_filter.size_limit_bytes;
    if media.size_bytes > limit {
        return Ok(Attachment::Skipped(OVER_LIMIT));
    }
    let (Some(root), Some(rel)) = (&options.media_root, &media.local_rel_path) else {
        return Ok(Attachment::Skipped(FILE_UNAVAILABLE));
    };
    let root = root
        .canonicalize()
        .map_err(|_| parameter("could not open saved media root"))?;
    let source = match root.join(rel).canonicalize() {
        Ok(path) if path.starts_with(&root) => path,
        Ok(_) => return Err(parameter("saved media path escapes its root")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(Attachment::Skipped(FILE_UNAVAILABLE));
        }
        Err(_) => return Err(parameter("could not resolve saved media path")),
    };
    let metadata = source
        .metadata()
        .map_err(|_| parameter("could not inspect saved media file"))?;
    if !metadata.is_file() {
        return Err(parameter("saved media path is not a regular file"));
    }
    if metadata.len() > limit {
        return Ok(Attachment::Skipped(OVER_LIMIT));
    }
    let input = File::open(source).map_err(|_| parameter("could not open saved media file"))?;
    let dir = options.output_dir.join(folder);
    fs::create_dir_all(&dir).map_err(|_| parameter("could not create archive media directory"))?;
    let default = format!("media_{id}.bin");
    let name = if safe_filename(&media.file_name) {
        &media.file_name
    } else {
        &default
    };
    let mut reader = input.take(limit);
    let path = write_media_atomic(&dir, name, |staged| {
        let bytes = io::copy(&mut reader, staged)?;
        if bytes != metadata.len() || reader.get_mut().read(&mut [0])? != 0 {
            return Err(io::Error::from(io::ErrorKind::InvalidData));
        }
        Ok(())
    })
    .map_err(|_| parameter("could not copy saved media atomically"))?;
    let thumb = if html {
        attach_thumbnail(&path, media, folder, id, budget)?
    } else {
        None
    };
    Ok(Attachment::Copied {
        path: relative_file(folder, &path)?,
        thumb,
    })
}

fn relative_file(folder: &str, path: &Path) -> Result<String, SecretChatError> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| parameter("invalid archive media filename"))?;
    Ok(format!("{folder}/{name}"))
}

fn attach_thumbnail(
    path: &Path,
    media: &SavedMediaRecord,
    folder: &str,
    id: i64,
    budget: Option<DecodedImageBudget>,
) -> Result<Option<String>, SecretChatError> {
    let (Some(width), Some(height)) = (media.width, media.height) else {
        return Ok(None);
    };
    if media.kind != SecretMediaKind::Photo
        || !media.mime_type.eq_ignore_ascii_case("image/jpeg")
        || width == 0
        || height == 0
    {
        return Ok(None);
    }
    let mut source = Vec::new();
    File::open(path)
        .and_then(|file| file.take(THUMB_SOURCE_LIMIT + 1).read_to_end(&mut source))
        .map_err(|_| parameter("could not read archive thumbnail source"))?;
    if source.len() as u64 > THUMB_SOURCE_LIMIT {
        return Ok(None);
    }
    let longest = u64::from(width.max(height)).max(320);
    let target = |size: u32| ((u64::from(size) * 320 / longest).max(1)) as u32;
    let (width, height) = (target(width), target(height));
    if budget.is_some_and(|budget| !budget.allows_jpeg(&source, (width, height))) {
        return Ok(None);
    }
    let Some(bytes) = desktop_thumb(source, i64::from(width), i64::from(height), None)
        .ok()
        .flatten()
    else {
        return Ok(None);
    };
    let dir = path
        .parent()
        .ok_or_else(|| parameter("invalid archive thumbnail path"))?;
    let thumb = write_media_atomic(dir, &format!("photo_{id}_thumb.jpg"), |file| {
        std::io::Write::write_all(file, &bytes)
    })
    .map_err(|_| parameter("could not write archive thumbnail"))?;
    Ok(Some(relative_file(folder, &thumb)?))
}

fn json_message(entry: &ExportedMessage<'_>) -> Value {
    let msg = entry.message;
    let mut data = Map::new();
    data.insert("id".into(), json!(msg.id));
    data.insert("type".into(), json!("message"));
    data.insert(
        "date".into(),
        json!(entry.date.format("%Y-%m-%dT%H:%M:%S").to_string()),
    );
    data.insert("date_unixtime".into(), json!(msg.date_unix.to_string()));
    data.insert("from".into(), json!(msg.sender_name));
    data.insert("from_id".into(), json!(format!("user{}", msg.sender_id)));
    if let Some(reply) = msg.reply_to_msg_id {
        data.insert("reply_to_message_id".into(), json!(reply));
    }
    if let Some(seconds) = msg.ttl_seconds {
        data.insert("self_destruct_period_seconds".into(), json!(seconds));
    }
    if let Some(media) = &msg.media {
        use SecretMediaKind::*;
        let (key, media_type) = match media.kind {
            Photo => ("photo", None),
            Document => ("file", None),
            Video => ("file", Some("video_file")),
            Audio => ("file", Some("audio_file")),
            VoiceNote => ("file", Some("voice_message")),
            VideoNote => ("file", Some("video_message")),
            Sticker => ("file", Some("sticker")),
            Gif => ("file", Some("animation")),
        };
        let path: &str = match &entry.attachment {
            Attachment::Copied { path, .. } => path,
            Attachment::Skipped(reason) => reason,
            Attachment::None => "(File unavailable, please try again later)",
        };
        data.insert(key.into(), json!(path));
        if media.kind == Photo {
            data.insert("photo_file_size".into(), json!(media.size_bytes));
        } else {
            data.insert("file_name".into(), json!(media.file_name));
            data.insert("file_size".into(), json!(media.size_bytes));
            data.insert("mime_type".into(), json!(media.mime_type));
        }
        if let Some(kind) = media_type {
            data.insert("media_type".into(), json!(kind));
        }
        for (name, value) in [
            ("width", media.width),
            ("height", media.height),
            ("duration_seconds", media.duration_secs),
        ] {
            if let Some(value) = value {
                data.insert(name.into(), json!(value));
            }
        }
    }
    data.insert("text".into(), json!(msg.text));
    let entities = if msg.text.is_empty() {
        json!([])
    } else {
        json!([{"type": "plain", "text": msg.text}])
    };
    data.insert("text_entities".into(), entities);
    Value::Object(data)
}

fn page_name(index: usize) -> String {
    if index == 0 {
        "messages.html".into()
    } else {
        format!("messages{}.html", index + 1)
    }
}

fn write_pages(
    options: &ExportOptions,
    entries: &[ExportedMessage<'_>],
) -> Result<Vec<PathBuf>, SecretChatError> {
    let page_size = options.messages_per_page;
    let count = entries.len().div_ceil(page_size).max(1);
    let links: BTreeMap<_, _> = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.message.id, index / page_size))
        .collect();
    let title = escape_html_text(&options.chat_title)?;
    let mut paths = Vec::with_capacity(count);
    for page in 0..count {
        let mut html = format!(
            "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{title}</title><link rel=\"stylesheet\" href=\"css/style.css\"><script src=\"js/script.js\" defer></script></head><body><div class=\"page_wrap\"><header class=\"page_header\"><div class=\"content\"><h1 class=\"text\" dir=\"auto\">{title}</h1></div></header><main class=\"page_body chat_page\"><div class=\"history\">\n"
        );
        let start = page * page_size;
        let end = start.saturating_add(page_size).min(entries.len());
        let mut date = None;
        for entry in &entries[start..end] {
            if date != Some(entry.date.date_naive()) {
                date = Some(entry.date.date_naive());
                html.push_str(&format!(
                    "<div class=\"message service\"><h2 class=\"body details\">{}</h2></div>\n",
                    entry.date.format("%-d %B %Y")
                ));
            }
            write_message(&mut html, entry, page, &links)?;
        }
        if entries.is_empty() {
            html.push_str("<p>No saved messages in the selected range.</p>\n");
        }
        if count > 1 {
            html.push_str("<nav class=\"pagination\" aria-label=\"Archive pages\">");
            if page > 0 {
                html.push_str(&format!(
                    "<a rel=\"prev\" href=\"{}\">Previous messages</a>",
                    page_name(page - 1)
                ));
            }
            if page + 1 < count {
                html.push_str(&format!(
                    "<a rel=\"next\" href=\"{}\">Next messages</a>",
                    page_name(page + 1)
                ));
            }
            html.push_str("</nav>\n");
        }
        html.push_str("</div></main></div></body></html>\n");
        let path = options.output_dir.join(page_name(page));
        fs::write(&path, html).map_err(|_| parameter("could not write archive HTML page"))?;
        paths.push(path);
    }
    Ok(paths)
}

fn write_message(
    html: &mut String,
    entry: &ExportedMessage<'_>,
    page: usize,
    links: &BTreeMap<i64, usize>,
) -> Result<(), SecretChatError> {
    let msg = entry.message;
    let sender = escape_html_text(&msg.sender_name)?;
    html.push_str(&format!("<article class=\"message default clearfix\" id=\"message{}\"><div class=\"body\"><time class=\"pull_right date details\" datetime=\"{}\">{} UTC</time><div class=\"from_name\" dir=\"auto\">{sender}</div>", msg.id, entry.date.to_rfc3339(), entry.date.format("%H:%M")));
    if let Some(reply) = msg.reply_to_msg_id {
        if let Some(&target) = links.get(&reply) {
            let prefix = if target == page {
                String::new()
            } else {
                page_name(target)
            };
            html.push_str(&format!("<div class=\"reply_to details\"><a href=\"{prefix}#message{reply}\">In reply to message</a></div>"));
        } else {
            html.push_str(
                "<div class=\"reply_to details\">Reply to a message outside this archive</div>",
            );
        }
    }
    if let Some(media) = &msg.media {
        write_attachment(html, media, &entry.attachment)?;
    }
    if !msg.text.is_empty() {
        html.push_str("<div class=\"text\" dir=\"auto\">");
        write_text(html, &msg.text)?;
        html.push_str("</div>");
    }
    html.push_str("</div></article>\n");
    Ok(())
}

fn write_text(html: &mut String, text: &str) -> Result<(), SecretChatError> {
    for part in text.split_inclusive(char::is_whitespace) {
        let token = part.trim_end();
        let tail = &part[token.len()..];
        let visible = escape_html_text(token)?;
        if (token.starts_with("https://") || token.starts_with("http://"))
            && let Ok(Some(href)) = safe_http_href(token)
        {
            html.push_str(&format!(
                "<a href=\"{}\" rel=\"noopener noreferrer\">{visible}</a>",
                escape_html_text(&href)?
            ));
        } else {
            html.push_str(&visible);
        }
        html.push_str(&escape_html_text(tail)?.replace('\n', "<br>\n"));
    }
    Ok(())
}

fn write_attachment(
    html: &mut String,
    media: &SavedMediaRecord,
    attachment: &Attachment,
) -> Result<(), SecretChatError> {
    let name = escape_html_text(&media.file_name)?;
    match attachment {
        Attachment::Copied { path, thumb } => {
            let url = relative_url(path);
            let extension = path
                .rsplit('.')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase();
            html.push_str("<div class=\"media_wrap clearfix\">");
            if matches!(
                media.kind,
                SecretMediaKind::Photo | SecretMediaKind::Sticker | SecretMediaKind::Gif
            ) && matches!(
                extension.as_str(),
                "jpg" | "jpeg" | "png" | "webp" | "gif" | "avif"
            ) {
                let source = thumb
                    .as_ref()
                    .map_or_else(|| url.clone(), |path| relative_url(path));
                html.push_str(&format!("<a href=\"{url}\" download><img src=\"{source}\" alt=\"{name}\" loading=\"lazy\"></a> "));
            } else if matches!(
                media.kind,
                SecretMediaKind::Video | SecretMediaKind::VideoNote | SecretMediaKind::Gif
            ) && matches!(extension.as_str(), "mp4" | "webm")
            {
                html.push_str(&format!(
                    "<video controls preload=\"none\" src=\"{url}\" aria-label=\"{name}\"></video> "
                ));
            } else if matches!(
                media.kind,
                SecretMediaKind::VoiceNote | SecretMediaKind::Audio
            ) && matches!(
                extension.as_str(),
                "ogg" | "opus" | "mp3" | "m4a" | "wav" | "flac"
            ) {
                html.push_str(&format!(
                    "<audio controls preload=\"none\" src=\"{url}\" aria-label=\"{name}\"></audio> "
                ));
            }
            html.push_str(&format!(
                "<a href=\"{url}\" download dir=\"auto\">{name}</a></div>"
            ));
        }
        Attachment::Skipped(reason) => {
            html.push_str(&format!(
                "<div class=\"media_wrap details\"><span dir=\"auto\">{name}</span> {reason}</div>"
            ));
        }
        Attachment::None => {}
    }
    Ok(())
}

fn relative_url(path: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut output = String::new();
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~') {
            output.push(char::from(byte));
        } else {
            output.push('%');
            output.push(char::from(HEX[(byte / 16) as usize]));
            output.push(char::from(HEX[(byte % 16) as usize]));
        }
    }
    output
}
