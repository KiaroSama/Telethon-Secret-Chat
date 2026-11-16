//! Streaming result.json framing and account-wide chat sections.
use super::{
    error::{ExportError, Result},
    format::DateContext,
    json_message::serialize_message,
    json_serialize::*,
    model::peer_to_bare_id,
    model_dialogs::{DialogInfo, DialogType, DialogsInfo},
    model_message::{MessagesSlice, skip_message_by_date},
    output_file::OutputFile,
    settings::{Environment, Format, Settings},
    writer::ExportWriter,
};
use std::path::PathBuf;
#[derive(Default)]
pub struct JsonWriter {
    settings: Settings,
    environment: Environment,
    context: JsonContext,
    had_item: bool,
    dialogs_mode: Option<bool>,
    output: Option<OutputFile>,
    dialog_open: bool,
    dates: DateContext,
    finished: bool,
}
impl JsonWriter {
    pub fn new() -> Self {
        Self::default()
    }
    fn write(&mut self, block: &[u8]) -> Result<()> {
        if self.finished {
            return Err(ExportError::WriterState);
        }
        self.output
            .as_mut()
            .ok_or(ExportError::WriterState)?
            .write_block(block)
    }
    fn push_nesting(&mut self, kind: bool) -> Vec<u8> {
        self.context.nesting.push(kind);
        self.had_item = false;
        vec![if kind { b'{' } else { b'[' }]
    }
    fn object_item(&mut self, key: &str) -> Vec<u8> {
        let mut result = if self.had_item {
            b",\n".to_vec()
        } else {
            b"\n".to_vec()
        };
        self.had_item = true;
        result.extend(indentation(&self.context));
        result.extend(serialize_string(key));
        result.extend_from_slice(b": ");
        result
    }
    fn array_item(&mut self) -> Vec<u8> {
        let mut result = if self.had_item {
            b",\n".to_vec()
        } else {
            b"\n".to_vec()
        };
        self.had_item = true;
        result.extend(indentation(&self.context));
        result
    }
    fn pop_nesting(&mut self) -> Result<Vec<u8>> {
        let kind = self.context.nesting.pop().ok_or(ExportError::WriterState)?;
        self.had_item = true;
        let mut result = b"\n".to_vec();
        result.extend(indentation(&self.context));
        result.push(if kind { b'}' } else { b']' });
        Ok(result)
    }
    fn chats_end(&mut self) -> Result<()> {
        let mut block = self.pop_nesting()?;
        block.extend(self.pop_nesting()?);
        self.write(&block)
    }
    fn validate_dialogs_mode(&mut self, left: bool) -> Result<()> {
        if self.dialogs_mode == Some(left) {
            return Ok(());
        }
        if self.dialogs_mode.is_some() {
            self.chats_end()?;
        }
        self.dialogs_mode = Some(left);
        let mut block = self.object_item(if left { "left_chats" } else { "chats" });
        block.extend(self.push_nesting(K_OBJECT));
        block.extend(self.object_item("about"));
        block.extend(serialize_string(if left {
            &self.environment.about_left_chats
        } else {
            &self.environment.about_chats
        }));
        block.extend(self.object_item("list"));
        block.extend(self.push_nesting(K_ARRAY));
        self.write(&block)
    }
}
fn dialog_type_name(kind: DialogType) -> &'static str {
    match kind {
        DialogType::Unknown => "",
        DialogType::Self_ => "saved_messages",
        DialogType::Replies => "replies",
        DialogType::VerifyCodes => "verification_codes",
        DialogType::Personal => "personal_chat",
        DialogType::Bot => "bot_chat",
        DialogType::PrivateGroup => "private_group",
        DialogType::PrivateSupergroup => "private_supergroup",
        DialogType::PublicSupergroup => "public_supergroup",
        DialogType::PrivateChannel => "private_channel",
        DialogType::PublicChannel => "public_channel",
    }
}
impl ExportWriter for JsonWriter {
    fn format(&self) -> Format {
        Format::Json
    }
    fn start(&mut self, settings: &Settings, environment: &Environment) -> Result<()> {
        if self.output.is_some() || settings.path.as_os_str().is_empty() {
            return Err(ExportError::WriterState);
        }
        self.settings = settings.clone();
        self.environment = environment.clone();
        self.context.countries = environment.countries.clone();
        self.dates = settings
            .time_zone
            .as_deref()
            .map_or_else(|| Ok(DateContext::system()), DateContext::named)?;
        self.output = Some(OutputFile::new(self.main_file_path()));
        if !self.settings.only_single_peer() {
            let mut block = self.push_nesting(K_OBJECT);
            block.extend(self.object_item("about"));
            block.extend(serialize_string(&self.environment.about_telegram));
            self.write(&block)?;
        }
        Ok(())
    }
    fn write_dialogs_start(&mut self, _: &DialogsInfo) -> Result<()> {
        Ok(())
    }
    fn write_dialog_start(&mut self, data: &DialogInfo) -> Result<()> {
        if self.dialog_open || self.output.is_none() {
            return Err(ExportError::WriterState);
        }
        if !self.settings.only_single_peer() {
            self.validate_dialogs_mode(data.is_left_channel)?;
        }
        let mut block = if self.settings.only_single_peer() {
            vec![]
        } else {
            self.array_item()
        };
        block.extend(self.push_nesting(K_OBJECT));
        if !matches!(
            data.r#type,
            DialogType::Self_ | DialogType::Replies | DialogType::VerifyCodes
        ) {
            block.extend(self.object_item("name"));
            block.extend(string_allow_null(&data.name));
        }
        block.extend(self.object_item("type"));
        block.extend(string_allow_null(dialog_type_name(data.r#type)));
        block.extend(self.object_item("id"));
        block.extend(number(peer_to_bare_id(data.peer_id)));
        block.extend(self.object_item("messages"));
        block.extend(self.push_nesting(K_ARRAY));
        self.dialog_open = true;
        self.write(&block)
    }
    fn write_dialog_slice(&mut self, data: &MessagesSlice) -> Result<()> {
        if !self.dialog_open {
            return Err(ExportError::WriterState);
        }
        let mut block = Vec::new();
        for message in &data.list {
            if skip_message_by_date(message, &self.settings) {
                continue;
            }
            block.extend(self.array_item());
            block.extend(serialize_message(
                &mut self.context,
                message,
                &data.peers,
                &self.environment.internal_links_domain,
                &self.dates,
            )?);
        }
        if !block.is_empty() {
            self.write(&block)?;
        }
        Ok(())
    }
    fn write_dialog_end(&mut self) -> Result<()> {
        if !self.dialog_open {
            return Err(ExportError::WriterState);
        }
        let mut block = self.pop_nesting()?;
        block.extend(self.pop_nesting()?);
        self.dialog_open = false;
        self.write(&block)
    }
    fn write_dialogs_end(&mut self) -> Result<()> {
        if !self.settings.only_single_peer() {
            self.chats_end()?;
        }
        Ok(())
    }
    fn finish(&mut self) -> Result<()> {
        if self.dialog_open || self.finished {
            return Err(ExportError::WriterState);
        }
        if !self.settings.only_single_peer() {
            let block = self.pop_nesting()?;
            self.write(&block)?;
        }
        if !self.context.nesting.is_empty() {
            return Err(ExportError::WriterState);
        }
        self.output
            .as_mut()
            .ok_or(ExportError::WriterState)?
            .close();
        self.finished = true;
        Ok(())
    }
    fn main_file_path(&self) -> PathBuf {
        self.settings.path.join("result.json")
    }
}
