//! Validated offline export boundary. Export failures never disclose archived contents or paths.
use super::{
    error::{ExportError, Result},
    files::{normalize_path, prepare_relative_path},
    format::DateContext,
    html_writer::HtmlWriter,
    json_writer::JsonWriter,
    model::{SkipReason, User},
    model_dialogs::{DialogsInfo, dialog_info_from_user, finalize_dialogs_info},
    model_message::{Message, MessagesSlice, skip_message_by_date},
    model_peers::{Peer, PeerUser},
    saved_parse::parse_with_sources,
    settings::{Environment, Format, MediaSettings, Settings},
    writer::{ExportResult, ExportWriter, HtmlAndJsonWriter},
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Component, Path, PathBuf},
};
#[derive(Debug, Clone, Default)]
pub struct SavedExportOptions {
    pub settings: Settings,
}
impl TryFrom<&Value> for SavedExportOptions {
    type Error = ExportError;
    fn try_from(value: &Value) -> Result<Self> {
        let map = value.as_object().ok_or(ExportError::InvalidOptions)?;
        let mut s = Settings::default();
        if let Some(v) = map.get("format") {
            s.format = Format::try_from(v.as_str().ok_or(ExportError::InvalidOptions)?)?;
        }
        if let Some(v) = map.get("size_limit") {
            s.media.size_limit = v.as_u64().ok_or(ExportError::InvalidOptions)?;
        }
        for (key, dest) in [
            ("date_from", &mut s.single_peer_from),
            ("date_till", &mut s.single_peer_till),
        ] {
            if let Some(v) = map.get(key) {
                *dest = v
                    .as_i64()
                    .filter(|n| *n >= 0)
                    .ok_or(ExportError::InvalidOptions)?;
            }
        }
        if let Some(v) = map.get("force_sub_path") {
            s.force_sub_path = v.as_bool().ok_or(ExportError::InvalidOptions)?;
        }
        if let Some(v) = map.get("media_types").filter(|v| !v.is_null()) {
            let names = v
                .as_array()
                .ok_or(ExportError::InvalidOptions)?
                .iter()
                .map(|v| v.as_str().ok_or(ExportError::InvalidOptions))
                .collect::<Result<Vec<_>>>()?;
            s.media = MediaSettings::select(names, s.media.size_limit)?;
        }
        validate(&s)?;
        Ok(Self { settings: s })
    }
}
fn validate(s: &Settings) -> Result<()> {
    if !s.media.validate()
        || s.single_peer_from < 0
        || s.single_peer_till < 0
        || s.single_peer_till != 0 && s.single_peer_till <= s.single_peer_from
    {
        return Err(ExportError::InvalidOptions);
    }
    Ok(())
}
/// Resolve existing symlinks before later `..` components, including destinations that do not yet exist.
pub fn resolve_path(path: &Path) -> Result<PathBuf> {
    let absolute = std::path::absolute(path).map_err(|_| ExportError::InvalidOptions)?;
    let mut resolved = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::CurDir => {}
            other => {
                resolved.push(other.as_os_str());
                match fs::canonicalize(&resolved) {
                    Ok(real) => resolved = real,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Err(ExportError::InvalidOptions),
                }
            }
        }
    }
    Ok(resolved)
}
pub fn export_saved_records(
    records: &[Value],
    destination: &Path,
    own: &User,
    peer: &User,
    options: &Settings,
    source_folder: Option<&Path>,
) -> Result<ExportResult> {
    own.validate()?;
    peer.validate()?;
    validate(options)?;
    if destination.as_os_str().is_empty() || destination.to_string_lossy().trim().is_empty() {
        return Err(ExportError::InvalidOptions);
    }
    let destination = resolve_path(destination)?;
    if let Some(source) = source_folder {
        let source = resolve_path(source)?;
        if destination.starts_with(&source) || source.starts_with(&destination) {
            return Err(ExportError::InvalidOptions);
        }
    }
    let mut settings = options.clone();
    settings.path = destination;
    let mut writer: Box<dyn ExportWriter> = match settings.format {
        Format::Html => Box::new(HtmlWriter::new()),
        Format::Json => Box::new(JsonWriter::new()),
        Format::Both => Box::new(HtmlAndJsonWriter::new(
            Box::new(HtmlWriter::new()),
            Box::new(JsonWriter::new()),
        )),
    };
    export_saved_chat(
        records,
        &settings,
        writer.as_mut(),
        own,
        peer,
        &Environment::default(),
    )
    .map_err(|_| ExportError::Filesystem)
}
pub fn export_saved_chat(
    records: &[Value],
    options: &Settings,
    writer: &mut dyn ExportWriter,
    own: &User,
    peer: &User,
    environment: &Environment,
) -> Result<ExportResult> {
    export_saved_chat_with_copier(
        records,
        options,
        writer,
        own,
        peer,
        environment,
        |source, target| {
            fs::copy(source, target)
                .map(|_| ())
                .map_err(|_| ExportError::Filesystem)
        },
    )
}
/// Copier injection supports applications with encrypted archive file providers; no source is removed.
pub fn export_saved_chat_with_copier(
    records: &[Value],
    options: &Settings,
    writer: &mut dyn ExportWriter,
    own: &User,
    peer: &User,
    environment: &Environment,
    mut copier: impl FnMut(&Path, &Path) -> Result<()>,
) -> Result<ExportResult> {
    let mut settings = options.clone();
    settings.single_peer = Some(peer.id);
    settings = settings.normalized();
    let dates = settings
        .time_zone
        .as_deref()
        .map_or_else(|| Ok(DateContext::system()), DateContext::named)?;
    settings.path = normalize_path(&settings, None, &dates)?;
    let (mut messages, sources) = parse_with_sources(records, own, peer, &dates)?;
    let mut peers = BTreeMap::new();
    peers.insert(own.id, Peer::User(PeerUser::from(own)));
    peers.insert(peer.id, Peer::User(PeerUser::from(peer)));
    let count = messages_count(&messages, &settings);
    let mut info = dialog_info_from_user(&PeerUser::from(peer));
    info.splits = vec![0];
    info.messages_count_per_split = vec![count as i64];
    let mut dialogs = DialogsInfo {
        chats: vec![info],
        ..Default::default()
    };
    finalize_dialogs_info(&mut dialogs, &settings)?;
    let result: Result<ExportResult> = (|| {
        writer.start(&settings, environment)?;
        writer.write_dialogs_start(&dialogs)?;
        writer.write_dialog_start(&dialogs.chats[0])?;
        let mut copied = 0;
        if count != 0 {
            copied = copy_files(&mut messages, &sources, &settings, &mut copier)?;
            for chunk in messages.chunks(100) {
                writer.write_dialog_slice(&MessagesSlice {
                    list: chunk.to_vec(),
                    peers: peers.clone(),
                })?;
            }
        }
        writer.write_dialog_end()?;
        writer.write_dialogs_end()?;
        writer.finish()?;
        Ok(ExportResult {
            path: settings.path,
            messages: if count == 0 { 0 } else { messages.len() },
            files: copied,
            ..Default::default()
        })
    })();
    if result.is_err() {
        writer.abort();
    }
    result
}
fn messages_count(messages: &[Message], s: &Settings) -> usize {
    let latest = messages.iter().map(|message| message.date).max();
    let earliest = messages.iter().map(|message| message.date).min();
    if latest.is_none_or(|date| date <= 0 || date < s.single_peer_from)
        || s.single_peer_till > 0
            && earliest.is_none_or(|date| date <= 0 || date >= s.single_peer_till)
    {
        0
    } else {
        messages.len()
    }
}
fn copy_files(
    messages: &mut [Message],
    sources: &BTreeMap<i64, PathBuf>,
    s: &Settings,
    copier: &mut impl FnMut(&Path, &Path) -> Result<()>,
) -> Result<usize> {
    let mut copied = 0;
    for message in messages {
        if skip_message_by_date(message, s) {
            continue;
        }
        let media_type = match &message.media.content {
            Some(super::model_media::MediaContent::Document(d)) => d.media_type(),
            _ => MediaSettings::PHOTO,
        };
        let Some(file) = message.media.file_mut() else {
            continue;
        };
        if !file.relative_path.is_empty() || file.skip_reason != SkipReason::None {
            continue;
        }
        let source = sources.get(&message.id);
        let Some((source, metadata)) = source.and_then(|path| {
            fs::metadata(path)
                .ok()
                .filter(|m| m.is_file())
                .map(|metadata| (path, metadata))
        }) else {
            file.skip_reason = SkipReason::Unavailable;
            continue;
        };
        file.skip_reason = if s.media.types & media_type != media_type {
            SkipReason::FileType
        } else if file.size.max(metadata.len()) > s.media.size_limit {
            SkipReason::FileSize
        } else {
            SkipReason::None
        };
        if file.skip_reason != SkipReason::None {
            continue;
        }
        let relative = prepare_relative_path(&s.path, &file.suggested_path);
        let target = s.path.join(&relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|_| ExportError::Filesystem)?;
        }
        copier(source, &target)?;
        file.relative_path = relative;
        copied += 1;
    }
    Ok(copied)
}
