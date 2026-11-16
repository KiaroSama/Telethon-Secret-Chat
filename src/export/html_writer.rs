//! Streaming single-chat HTML pages, static assets and cross-page reply links.
use super::{
    assets::write_assets,
    error::{ExportError, Result},
    format::DateContext,
    html_message::MessageInfo,
    html_text::{PeersMap, WrapBase, attrs, display_date, format_date_text, serialize_string},
    model_dialogs::{DialogInfo, DialogsInfo},
    model_message::{MessagesSlice, skip_message_by_date},
    output_file::OutputFile,
    settings::{Environment, Format, Settings},
    writer::ExportWriter,
};
use std::{
    ops::{Deref, DerefMut},
    path::{Component, Path, PathBuf},
};
pub const MESSAGES_IN_FILE: usize = 1000;
pub struct HtmlPage {
    pub wrap: WrapBase,
    file: OutputFile,
    closed: bool,
    composed_start: Vec<u8>,
}
impl Deref for HtmlPage {
    type Target = WrapBase;
    fn deref(&self) -> &WrapBase {
        &self.wrap
    }
}
impl DerefMut for HtmlPage {
    fn deref_mut(&mut self) -> &mut WrapBase {
        &mut self.wrap
    }
}
impl HtmlPage {
    pub fn new(path: impl Into<PathBuf>, base: &Path) -> Result<Self> {
        let path = path.into();
        let relative = path
            .strip_prefix(base)
            .map_err(|_| ExportError::InvalidOptions)?;
        if relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(ExportError::InvalidOptions);
        }
        let depth = relative.components().count().saturating_sub(1);
        let mut result = Self {
            wrap: WrapBase::new("../".repeat(depth)),
            file: OutputFile::new(path),
            closed: false,
            composed_start: vec![],
        };
        result.composed_start = result.compose_start()?.into_bytes();
        Ok(result)
    }
    pub fn empty(&self) -> bool {
        self.file.empty()
    }
    pub fn write_block(&mut self, block: &[u8]) -> Result<()> {
        if self.closed {
            return Err(ExportError::WriterState);
        }
        let result = if !block.is_empty() && self.file.empty() {
            let mut full = self.composed_start.clone();
            full.extend_from_slice(block);
            self.file.write_block(&full)
        } else {
            self.file.write_block(block)
        };
        if result.is_err() {
            self.closed = true;
        }
        result
    }
    pub fn close(&mut self) -> Result<()> {
        let result = (|| {
            if !self.closed {
                self.closed = true;
                if !self.file.empty() {
                    let mut block = Vec::new();
                    while !self.wrap.context.empty() {
                        block.extend(self.wrap.pop_tag()?.into_bytes());
                    }
                    self.file.write_block(&block)?;
                }
            }
            Ok(())
        })();
        self.file.close();
        result
    }
    pub fn push_header(&mut self, header: &str, path: &str) -> Result<String> {
        let mut result = self.push_div("page_header", "");
        if path.is_empty() {
            result += &self.push_div("content", "");
        } else {
            let href = self.relative_path(path);
            result += &self.push_tag(
                "a",
                &attrs([
                    ("class", "content block_link"),
                    ("href", &href),
                    ("onclick", "return GoBack(this)"),
                ]),
            );
        }
        result += &self.push_div("text bold", "");
        result += &serialize_string(header);
        result += &self.pop_tag()?;
        result += &self.pop_tag()?;
        Ok(result + &self.pop_tag()?)
    }
    fn compose_start(&mut self) -> Result<String> {
        let mut result = "<!DOCTYPE html>".to_owned() + &self.push_tag("html", &attrs([]));
        result += &self.push_tag("head", &attrs([]));
        result += &self.push_tag("meta", &attrs([("charset", "utf-8"), ("empty", "")]));
        result += &self.push_tag("title", &attrs([("inline", "")]));
        result += "Exported Data";
        result += &self.pop_tag()?;
        result += &self.push_tag(
            "meta",
            &attrs([
                ("name", "viewport"),
                ("content", "width=device-width, initial-scale=1.0"),
                ("empty", ""),
            ]),
        );
        let css = format!("{}css/style.css", self.base);
        let js = format!("{}js/script.js", self.base);
        result += &self.push_tag(
            "link",
            &attrs([("href", &css), ("rel", "stylesheet"), ("empty", "")]),
        );
        result += &self.push_tag(
            "script",
            &attrs([("src", &js), ("type", "text/javascript")]),
        );
        result += &self.pop_tag()?;
        result += &self.pop_tag()?;
        result += &self.push_tag("body", &attrs([("onload", "CheckLocation();")]));
        Ok(result + &self.push_div("page_wrap", ""))
    }
}
pub fn messages_file(index: usize) -> String {
    if index == 0 {
        "messages.html".into()
    } else {
        format!("messages{}.html", index + 1)
    }
}
#[derive(Default)]
pub struct HtmlWriter {
    settings: Settings,
    environment: Environment,
    dialog: DialogInfo,
    messages_count: usize,
    last_message_info: Option<MessageInfo>,
    date_message_id: i64,
    chat: Option<HtmlPage>,
    last_message_ids_per_file: Vec<i64>,
    chat_file_empty: bool,
    dates: DateContext,
    started: bool,
}
impl HtmlWriter {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn wrap_message_link(&self, id: i64, text: &str) -> String {
        for (index, max) in self.last_message_ids_per_file.iter().enumerate() {
            if id <= *max {
                return format!(
                    "<a href=\"{}#go_to_message{id}\">{text}</a>",
                    messages_file(index)
                );
            }
        }
        format!("<a href=\"#go_to_message{id}\" onclick=\"return GoToMessage({id})\">{text}</a>")
    }
    fn page(&self, index: usize) -> Result<HtmlPage> {
        let mut page = HtmlPage::new(
            self.settings.path.join(format!(
                "{}{}",
                self.dialog.relative_path,
                messages_file(index)
            )),
            &self.settings.path,
        )?;
        page.context.countries = self.environment.countries.clone();
        Ok(page)
    }
    fn write_opening(&self, chat: &mut HtmlPage, index: usize) -> Result<()> {
        let name = if self.dialog.name.is_empty() && self.dialog.last_name.is_empty() {
            "Deleted Account".into()
        } else {
            format!("{} {}", self.dialog.name, self.dialog.last_name)
        };
        let mut block = chat.push_header(&name, "")?;
        block += &chat.push_div("page_body chat_page", "");
        block += &chat.push_div("history", "");
        if index > 0 {
            block += &chat.push_tag(
                "a",
                &attrs([
                    ("class", "pagination block_link"),
                    ("href", &messages_file(index - 1)),
                ]),
            );
            block += "Previous messages";
            block += &chat.pop_tag()?;
        }
        chat.write_block(block.as_bytes())
    }
    fn switch_page(&mut self, chat: &mut HtmlPage, index: usize) -> Result<()> {
        let mut link = chat.push_tag(
            "a",
            &attrs([
                ("class", "pagination block_link"),
                ("href", &messages_file(index)),
            ]),
        );
        link += "Next messages";
        link += &chat.pop_tag()?;
        chat.write_block(link.as_bytes())?;
        chat.close()?;
        *chat = self.page(index)?;
        self.chat_file_empty = true;
        Ok(())
    }
    fn write_slice(&mut self, chat: &mut HtmlPage, data: &MessagesSlice) -> Result<()> {
        let peers = PeersMap {
            data: data.peers.clone(),
        };
        let mut old_index = if self.messages_count == 0 {
            0
        } else {
            (self.messages_count - 1) / MESSAGES_IN_FILE
        };
        let mut previous = self.last_message_info.clone();
        let mut saved: Option<MessageInfo> = None;
        let mut block = Vec::new();
        for message in &data.list {
            if skip_message_by_date(message, &self.settings) {
                continue;
            }
            let new_index = self.messages_count / MESSAGES_IN_FILE;
            if old_index != new_index {
                chat.write_block(&block)?;
                self.switch_page(chat, new_index)?;
                let last = saved
                    .as_ref()
                    .or(self.last_message_info.as_ref())
                    .ok_or(ExportError::WriterState)?;
                self.last_message_ids_per_file.push(last.id);
                block.clear();
                self.last_message_info = None;
                previous = None;
                old_index = new_index;
            }
            if self.chat_file_empty {
                self.write_opening(chat, old_index)?;
                self.chat_file_empty = false;
            }
            if display_date(
                message.date,
                previous.as_ref().map_or(0, |p| p.date),
                &self.dates,
            )? {
                self.date_message_id -= 1;
                let date = format_date_text(message.date, &self.dates)?;
                block.extend(chat.push_service_message(
                    self.date_message_id,
                    &self.dialog,
                    &self.settings.path,
                    date.as_bytes(),
                    None,
                )?);
            }
            let (info, content) = chat.push_message(
                message,
                previous.as_ref(),
                &self.dialog,
                &self.settings.path,
                &peers,
                &self.environment.internal_links_domain,
                |id, text| self.wrap_message_link(id, text),
                &self.dates,
            )?;
            block.extend(content);
            self.messages_count += 1;
            saved = Some(info.clone());
            previous = Some(info);
        }
        if let Some(saved) = saved {
            self.last_message_info = Some(saved);
        }
        if !block.is_empty() {
            chat.write_block(&block)?;
        }
        Ok(())
    }
}
impl ExportWriter for HtmlWriter {
    fn format(&self) -> Format {
        Format::Html
    }
    fn start(&mut self, settings: &Settings, environment: &Environment) -> Result<()> {
        if !settings.only_single_peer() {
            return Err(ExportError::Unsupported);
        }
        if self.started || settings.path.as_os_str().is_empty() {
            return Err(ExportError::WriterState);
        }
        self.settings = settings.clone();
        self.environment = environment.clone();
        self.dates = settings
            .time_zone
            .as_deref()
            .map_or_else(|| Ok(DateContext::system()), DateContext::named)?;
        write_assets(&settings.path)?;
        self.started = true;
        Ok(())
    }
    fn write_dialogs_start(&mut self, _: &DialogsInfo) -> Result<()> {
        Ok(())
    }
    fn write_dialog_start(&mut self, data: &DialogInfo) -> Result<()> {
        if self.chat.is_some() || !self.started {
            return Err(ExportError::WriterState);
        }
        self.dialog = data.clone();
        self.chat = Some(self.page(0)?);
        self.chat_file_empty = true;
        self.messages_count = 0;
        self.date_message_id = 0;
        self.last_message_info = None;
        self.last_message_ids_per_file.clear();
        Ok(())
    }
    fn write_dialog_slice(&mut self, data: &MessagesSlice) -> Result<()> {
        let mut chat = self.chat.take().ok_or(ExportError::WriterState)?;
        let result = self.write_slice(&mut chat, data);
        self.chat = Some(chat);
        result
    }
    fn write_dialog_end(&mut self) -> Result<()> {
        let mut chat = self.chat.take().ok_or(ExportError::WriterState)?;
        if self.messages_count == 0 {
            self.write_opening(&mut chat, 0)?;
            self.date_message_id -= 1;
            let block = chat.push_service_message(
                self.date_message_id,
                &self.dialog,
                &self.settings.path,
                b"No exported messages",
                None,
            )?;
            chat.write_block(&block)?;
        }
        chat.close()
    }
    fn write_dialogs_end(&mut self) -> Result<()> {
        Ok(())
    }
    fn finish(&mut self) -> Result<()> {
        Ok(())
    }
    fn main_file_path(&self) -> PathBuf {
        self.settings.path.join(messages_file(0))
    }
}
