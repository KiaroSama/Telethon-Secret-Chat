//! Desktop file work ordering, caching, downloads, reference refresh and custom emoji resolution.
use super::{
    error::{ExportError, Result},
    fetch_client::*,
    files::{apply_file_policy, prepare_relative_path},
    format::DateContext,
    model::*,
    model_actions::Action,
    model_document::{parse_document, refresh_file_reference},
    model_media::MediaContent,
    model_message::{
        FileOrigin, Message, MessagesSlice, ReactionType, parse_messages_slice,
        skip_message_by_date,
    },
    model_peers::type_is,
    model_rich::{RichText, RichTextType},
    parse_rich::{
        RichVisitor, extract_full_rich_message, parse_rich_message,
        refresh_rich_message_file_reference, visit_rich_message,
    },
    settings::{MediaSettings, Settings},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    io::Write,
    path::PathBuf,
};
pub const FILE_CHUNK_KB: usize = 128;
pub fn compute_location_key(location: &FileLocation) -> (i64, i64) {
    let letter = string(&location.data, "thumb_size")
        .chars()
        .next()
        .map_or(0, |c| i64::from(c as u32));
    let t = if type_is(&location.data, "InputDocumentFileLocation") {
        2
    } else if type_is(&location.data, "InputPhotoFileLocation") {
        6
    } else {
        5
    };
    (
        i64::from(location.dc_id) | (t << 24) | if t == 5 { 0 } else { letter << 16 },
        if t == 5 {
            0
        } else {
            integer(&location.data, "id")
        },
    )
}
pub struct LoadedFileCache {
    limit: usize,
    map: BTreeMap<(i64, i64), String>,
    list: VecDeque<(i64, i64)>,
}
impl Default for LoadedFileCache {
    fn default() -> Self {
        Self::new(100_000)
    }
}
impl LoadedFileCache {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            map: BTreeMap::new(),
            list: VecDeque::new(),
        }
    }
    pub fn save(&mut self, location: &FileLocation, path: String) {
        if !location.available() {
            return;
        }
        let key = compute_location_key(location);
        self.map.insert(key, path);
        self.list.push_back(key);
        if self.list.len() > self.limit
            && let Some(old) = self.list.pop_front()
        {
            self.map.remove(&old);
        }
    }
    pub fn find(&self, location: &FileLocation) -> Option<&str> {
        if location.available() {
            self.map
                .get(&compute_location_key(location))
                .map(String::as_str)
        } else {
            None
        }
    }
}
pub struct LazyFile {
    path: PathBuf,
    handle: Option<fs::File>,
}
impl LazyFile {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            handle: None,
        }
    }
    pub fn write(&mut self, data: &[u8]) -> Result<usize> {
        if self.handle.is_none() {
            if let Some(parent) = self.path.parent() {
                fs::create_dir_all(parent).map_err(|_| ExportError::Filesystem)?;
            }
            self.handle = Some(fs::File::create(&self.path).map_err(|_| ExportError::Filesystem)?);
        }
        self.handle
            .as_mut()
            .ok_or(ExportError::Filesystem)?
            .write_all(data)
            .map_err(|_| ExportError::Filesystem)?;
        Ok(data.len())
    }
    pub fn finish(&mut self) -> Result<()> {
        if self.handle.is_none() {
            self.write(&[])?;
        }
        self.close();
        Ok(())
    }
    pub fn close(&mut self) {
        self.handle = None;
    }
}
pub fn to_ulonglong(value: &str) -> i64 {
    let s = value.trim();
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        0
    } else {
        s.parse::<u64>()
            .ok()
            .and_then(|v| i64::try_from(v).ok())
            .unwrap_or(0)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum FileSlot {
    Main,
    Thumb,
    RichPhoto(i64),
    RichDocument(i64),
    RichThumb(i64),
}
#[derive(Debug, Clone, PartialEq)]
pub struct FileWork {
    pub file: File,
    pub media_type: u32,
    pub controlling_size: u64,
    pub rich: bool,
    pub slot: FileSlot,
}
impl FileWork {
    pub fn apply(self, message: &mut Message) {
        let target = match self.slot {
            FileSlot::Main => message.file_mut(),
            FileSlot::Thumb => match &mut message.media.content {
                Some(MediaContent::Document(d)) => Some(&mut d.thumb.file),
                _ => None,
            },
            FileSlot::RichPhoto(id) => message
                .rich_message
                .as_mut()
                .and_then(|r| r.photos.get_mut(&id))
                .map(|p| &mut p.image.file),
            FileSlot::RichDocument(id) => message
                .rich_message
                .as_mut()
                .and_then(|r| r.documents.get_mut(&id))
                .map(|d| &mut d.file),
            FileSlot::RichThumb(id) => message
                .rich_message
                .as_mut()
                .and_then(|r| r.documents.get_mut(&id))
                .map(|d| &mut d.thumb.file),
        };
        if let Some(target) = target {
            *target = self.file;
        }
    }
}
pub fn message_file_work(message: &Message) -> Vec<FileWork> {
    let mut work = Vec::new();
    let action_photo = matches!(
        message.action.content,
        Some(Action::ChatEditPhoto(_) | Action::SuggestProfilePhoto(_))
    );
    let mut main = if action_photo {
        Some(message.file())
    } else {
        None
    };
    let mut main_type = MediaSettings::PHOTO;
    let mut doc = None;
    match &message.media.content {
        Some(MediaContent::Photo(p)) => {
            main.get_or_insert_with(|| p.image.file.clone());
        }
        Some(MediaContent::Document(d)) => {
            doc = Some(d);
            if main.is_none() {
                main = Some(d.file.clone());
                main_type = d.media_type();
            }
        }
        Some(MediaContent::SharedContact(c)) => {
            main.get_or_insert_with(|| c.vcard.clone());
        }
        _ => {}
    }
    let size = main.as_ref().map_or(0, |f| f.size);
    if let Some(file) = main {
        work.push(FileWork {
            file,
            media_type: main_type,
            controlling_size: size,
            rich: false,
            slot: FileSlot::Main,
        });
    }
    if let Some(d) = doc.filter(|d| d.thumb.width > 0) {
        work.push(FileWork {
            file: d.thumb.file.clone(),
            media_type: d.media_type(),
            controlling_size: size,
            rich: false,
            slot: FileSlot::Thumb,
        });
    }
    struct Visitor {
        work: Vec<FileWork>,
        seen: BTreeSet<FileSlot>,
    }
    impl Visitor {
        fn add(&mut self, file: &File, media_type: u32, size: u64, slot: FileSlot) {
            if self.seen.insert(slot.clone()) {
                self.work.push(FileWork {
                    file: file.clone(),
                    media_type,
                    controlling_size: size,
                    rich: true,
                    slot,
                });
            }
        }
    }
    impl RichVisitor for Visitor {
        fn photo(&mut self, p: &mut Photo) {
            self.add(
                &p.image.file,
                MediaSettings::PHOTO,
                p.image.file.size,
                FileSlot::RichPhoto(p.id),
            );
        }
        fn document(&mut self, d: &mut Document) {
            self.add(
                &d.file,
                d.media_type(),
                d.file.size,
                FileSlot::RichDocument(d.id),
            );
            if d.thumb.width > 0 {
                self.add(
                    &d.thumb.file,
                    d.media_type(),
                    d.file.size,
                    FileSlot::RichThumb(d.id),
                );
            }
        }
    }
    if let Some(mut rich) = message.rich_message.clone() {
        let mut visitor = Visitor {
            work: vec![],
            seen: BTreeSet::new(),
        };
        visit_rich_message(&mut rich, &mut visitor);
        work.extend(visitor.work);
    }
    work
}
pub struct MessageFiles<'a, C: ExportClient> {
    pub client: &'a C,
    pub settings: Settings,
    pub self_id: i64,
    pub scope: RequestScope,
    pub splits: Vec<Value>,
    pub file_cache: LoadedFileCache,
    pub resolved_emoji: BTreeMap<i64, Document>,
    pub unresolved_emoji: BTreeSet<i64>,
    pub files: usize,
    pub messages: usize,
    pub emoji_chunk: usize,
    pub warnings: Vec<&'static str>,
}
impl<'a, C: ExportClient> MessageFiles<'a, C> {
    pub fn new(client: &'a C, settings: Settings, self_id: i64) -> Self {
        Self {
            client,
            settings,
            self_id,
            scope: RequestScope::default(),
            splits: vec![],
            file_cache: LoadedFileCache::default(),
            resolved_emoji: BTreeMap::new(),
            unresolved_emoji: BTreeSet::new(),
            files: 0,
            messages: 0,
            emoji_chunk: 100,
            warnings: vec![],
        }
    }
    pub async fn main(&self, request: Value) -> FetchResult<Value> {
        self.client.request(self.scope.clone(), request).await
    }
    pub async fn split(&self, index: i64, request: Value) -> FetchResult<Value> {
        let mut scope = self.scope.clone();
        if scope.takeout.is_some() {
            scope.range = self.splits.get(index as usize).cloned();
        }
        self.client.request(scope, request).await
    }
    pub fn collect_custom_emoji(&mut self, slice: &MessagesSlice) {
        struct Collector(Vec<i64>);
        impl RichVisitor for Collector {
            fn text(&mut self, t: &mut RichText) {
                if t.r#type == RichTextType::CustomEmoji {
                    self.0.push(t.id);
                }
            }
        }
        let mut values = Vec::new();
        for message in &slice.list {
            if skip_message_by_date(message, &self.settings) {
                continue;
            }
            values.extend(
                message
                    .text
                    .iter()
                    .filter(|p| p.kind == TextType::CustomEmoji)
                    .map(|p| to_ulonglong(&p.additional)),
            );
            values.extend(
                message
                    .reactions
                    .iter()
                    .filter(|r| r.r#type == ReactionType::CustomEmoji)
                    .map(|r| to_ulonglong(&r.document_id)),
            );
            if let Some(mut r) = message.rich_message.clone() {
                let mut c = Collector(vec![]);
                visit_rich_message(&mut r, &mut c);
                values.extend(c.0);
            }
        }
        for id in values {
            if id != 0 && !self.resolved_emoji.contains_key(&id) {
                self.unresolved_emoji.insert(id);
            }
        }
    }
    pub async fn resolve_custom_emoji(
        &mut self,
        context: &mut ParseMediaContext,
        folder: &str,
        dates: &DateContext,
    ) -> FetchResult<()> {
        while !self.unresolved_emoji.is_empty() {
            let chunk = self
                .unresolved_emoji
                .iter()
                .rev()
                .take(self.emoji_chunk.max(1))
                .copied()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            for id in &chunk {
                self.unresolved_emoji.remove(id);
            }
            let documents = match self
                .main(json!({"_":"messages.GetCustomEmojiDocuments","document_id":chunk}))
                .await
            {
                Ok(v) => v,
                Err(FetchError::Rpc { .. }) => {
                    self.warnings
                        .push("Export Error: Failed to get documents for emoji.");
                    json!([])
                }
                Err(e) => return Err(e),
            };
            for entry in documents.as_array().into_iter().flatten() {
                let d = parse_document(context, entry, folder, 0, dates)?;
                self.resolved_emoji.entry(d.id).or_insert(d);
            }
            for id in chunk {
                self.resolved_emoji.entry(id).or_insert_with(|| {
                    let mut d = Document::default();
                    d.file.skip_reason = SkipReason::Unavailable;
                    d
                });
            }
        }
        Ok(())
    }
    pub async fn custom_emoji_value(
        &mut self,
        folder: &str,
        message: &Message,
        data: &str,
        dates: &DateContext,
    ) -> FetchResult<String> {
        let id = to_ulonglong(data);
        if id == 0 {
            return Ok(data.into());
        }
        let Some(mut d) = self.resolved_emoji.get(&id).cloned() else {
            return Ok(TextPart::UNAVAILABLE_EMOJI.into());
        };
        let media_type = d.media_type();
        let size = d.file.size;
        let result = self
            .process_file_load(
                folder,
                &mut d.file,
                &FileOrigin {
                    custom_emoji_id: id,
                    ..Default::default()
                },
                message,
                media_type,
                size,
                dates,
            )
            .await;
        self.resolved_emoji.insert(id, d.clone());
        result?;
        Ok(match d.file.skip_reason {
            SkipReason::Unavailable => TextPart::UNAVAILABLE_EMOJI.into(),
            SkipReason::FileType | SkipReason::FileSize => String::new(),
            _ => d.file.relative_path,
        })
    }
    pub async fn resolve_message_custom_emoji(
        &mut self,
        folder: &str,
        message: &mut Message,
        dates: &DateContext,
    ) -> FetchResult<()> {
        let snapshot = message.clone();
        for part in &mut message.text {
            if part.kind == TextType::CustomEmoji {
                part.additional = self
                    .custom_emoji_value(folder, &snapshot, &part.additional, dates)
                    .await?;
            }
        }
        for reaction in &mut message.reactions {
            if reaction.r#type == ReactionType::CustomEmoji {
                reaction.document_id = self
                    .custom_emoji_value(folder, &snapshot, &reaction.document_id, dates)
                    .await?;
            }
        }
        struct Texts(Vec<String>);
        impl RichVisitor for Texts {
            fn text(&mut self, t: &mut RichText) {
                if t.r#type == RichTextType::CustomEmoji {
                    self.0.push(t.custom_emoji_data.clone());
                }
            }
        }
        if let Some(r) = &mut message.rich_message {
            let mut texts = Texts(vec![]);
            visit_rich_message(r, &mut texts);
            let mut replaced = VecDeque::new();
            for data in texts.0 {
                replaced.push_back(
                    self.custom_emoji_value(folder, &snapshot, &data, dates)
                        .await?,
                );
            }
            struct Replace(VecDeque<String>);
            impl RichVisitor for Replace {
                fn text(&mut self, t: &mut RichText) {
                    if t.r#type == RichTextType::CustomEmoji {
                        t.custom_emoji_data = self.0.pop_front().unwrap_or_default();
                    }
                }
            }
            visit_rich_message(r, &mut Replace(replaced));
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn process_file_load(
        &mut self,
        folder: &str,
        file: &mut File,
        origin: &FileOrigin,
        message: &Message,
        media_type: u32,
        size: u64,
        dates: &DateContext,
    ) -> FetchResult<()> {
        if apply_file_policy(
            file,
            skip_message_by_date(message, &self.settings),
            media_type,
            size,
            &self.settings.media,
        ) {
            return Ok(());
        }
        if let Some(cached) = self.file_cache.find(&file.location) {
            file.relative_path = cached.into();
            return Ok(());
        }
        let relative = prepare_relative_path(&self.settings.path, &file.suggested_path);
        if !file.content.is_empty() {
            let mut target = LazyFile::new(self.settings.path.join(&relative));
            target.write(&file.content)?;
            target.close();
            file.relative_path = relative.clone();
            self.file_cache.save(&file.location, relative);
            return Ok(());
        }
        file.relative_path = self
            .load_file(folder, file, origin, relative, dates)
            .await?;
        if file.relative_path.is_empty() {
            file.skip_reason = SkipReason::Unavailable;
        }
        Ok(())
    }
    async fn load_file(
        &mut self,
        folder: &str,
        file: &File,
        origin: &FileOrigin,
        relative: String,
        dates: &DateContext,
    ) -> FetchResult<String> {
        let mut location = file.location.clone();
        let path = self.settings.path.join(&relative);
        let mut target = LazyFile::new(&path);
        let mut refreshes = 0;
        loop {
            match self
                .client
                .download(&location, file.size, &mut target)
                .await
            {
                Ok(()) => break,
                Err(e) => {
                    let name = rpc_message(&e);
                    if matches!(
                        name,
                        "LOCATION_INVALID" | "VERSION_INVALID" | "LOCATION_NOT_AVAILABLE"
                    ) {
                        self.warnings.push("Export Error: File unavailable.");
                        return Ok(String::new());
                    }
                    if !matches!(&e, FetchError::Rpc { code: 400, .. })
                        || !name.starts_with("FILE_REFERENCE_")
                    {
                        return Err(e);
                    }
                    refreshes += 1;
                    let fresh = if refreshes <= 3 {
                        self.refresh_reference(folder, origin, &location, dates)
                            .await?
                    } else {
                        None
                    };
                    let Some(fresh) = fresh else {
                        self.warnings.push("Export Error: File unavailable.");
                        return Ok(String::new());
                    };
                    location = fresh;
                    target.close();
                    target = LazyFile::new(&path);
                }
            }
        }
        target.finish()?;
        self.files += 1;
        self.file_cache.save(&location, relative.clone());
        Ok(relative)
    }
    pub async fn refresh_reference(
        &mut self,
        folder: &str,
        origin: &FileOrigin,
        location: &FileLocation,
        dates: &DateContext,
    ) -> FetchResult<Option<FileLocation>> {
        let mut context = ParseMediaContext {
            self_peer_id: peer_from_user(self.self_id),
            ..Default::default()
        };
        let answer = if origin.custom_emoji_id != 0 {
            self.main(json!({"_":"messages.GetCustomEmojiDocuments","document_id":[origin.custom_emoji_id]})).await
        } else if origin.rich_message {
            if origin.message_id == 0 {
                return Ok(None);
            }
            self.main(
                json!({"_":"messages.GetRichMessage","peer":origin.peer,"id":origin.message_id}),
            )
            .await
        } else {
            if origin.message_id == 0 {
                return Err(ExportError::InvalidRecord.into());
            }
            let ids = json!([{"_":"InputMessageID","id":origin.message_id}]);
            if type_is(&origin.peer, "InputPeerChannel")
                || type_is(&origin.peer, "InputPeerChannelFromMessage")
            {
                let channel = if type_is(&origin.peer, "InputPeerChannel") {
                    json!({"_":"InputChannel","channel_id":origin.peer["channel_id"],"access_hash":origin.peer["access_hash"]})
                } else {
                    json!({"_":"InputChannelFromMessage","peer":origin.peer["peer"],"msg_id":origin.peer["msg_id"],"channel_id":origin.peer["channel_id"]})
                };
                self.main(json!({"_":"channels.GetMessages","channel":channel,"id":ids}))
                    .await
            } else {
                self.split(origin.split, json!({"_":"messages.GetMessages","id":ids}))
                    .await
            }
        };
        let result = match answer {
            Ok(v) => v,
            Err(FetchError::Rpc { .. }) => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut refreshed = location.clone();
        if origin.custom_emoji_id != 0 {
            let matches = result
                .as_array()
                .into_iter()
                .flatten()
                .filter(|d| integer(d, "id") == origin.custom_emoji_id)
                .collect::<Vec<_>>();
            if matches.len() != 1 {
                return Ok(None);
            }
            let d = parse_document(&mut context, matches[0], folder, 0, dates)?;
            return Ok(
                if refresh_file_reference(&mut refreshed, &d.file.location)
                    || d.thumb.width > 0
                        && refresh_file_reference(&mut refreshed, &d.thumb.file.location)
                {
                    Some(refreshed)
                } else {
                    None
                },
            );
        }
        if origin.rich_message {
            let Some(r) = extract_full_rich_message(array(&result, "messages"), origin.message_id)
            else {
                return Ok(None);
            };
            let rich = parse_rich_message(&mut context, &r, folder, 0, dates)?;
            return Ok(if rich.part {
                None
            } else {
                refresh_rich_message_file_reference(location, &rich)
            });
        }
        let found = parse_messages_slice(
            &mut context,
            array(&result, "messages"),
            array(&result, "users"),
            array(&result, "chats"),
            folder,
            dates,
        )?;
        for message in found.list {
            if message.id == origin.message_id
                && (refresh_file_reference(&mut refreshed, &message.file().location)
                    || refresh_file_reference(&mut refreshed, &message.thumb().file.location))
            {
                return Ok(Some(refreshed));
            }
        }
        Ok(None)
    }
}
