//! Native exporter writer protocol and both-format fan-out.
use super::{
    error::Result,
    model_dialogs::{DialogInfo, DialogsInfo},
    model_message::MessagesSlice,
    settings::{Environment, Format, Settings},
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportResult {
    pub path: PathBuf,
    pub takeout: bool,
    pub takeout_error: String,
    pub messages: usize,
    pub files: usize,
}
pub trait ExportWriter: Send {
    fn format(&self) -> Format;
    fn start(&mut self, settings: &Settings, environment: &Environment) -> Result<()>;
    fn write_dialogs_start(&mut self, data: &DialogsInfo) -> Result<()>;
    fn write_dialog_start(&mut self, data: &DialogInfo) -> Result<()>;
    fn write_dialog_slice(&mut self, data: &MessagesSlice) -> Result<()>;
    fn write_dialog_end(&mut self) -> Result<()>;
    fn write_dialogs_end(&mut self) -> Result<()>;
    fn finish(&mut self) -> Result<()>;
    /// Abandon partial output and release resources without replacing the original error.
    /// Writers that hold no resources need no cleanup.
    fn abort(&mut self) {}
    fn main_file_path(&self) -> PathBuf;
}
pub struct HtmlAndJsonWriter {
    writers: [Box<dyn ExportWriter>; 2],
}
impl HtmlAndJsonWriter {
    pub fn new(html: Box<dyn ExportWriter>, json: Box<dyn ExportWriter>) -> Self {
        Self {
            writers: [html, json],
        }
    }
    fn invoke(
        &mut self,
        mut method: impl FnMut(&mut dyn ExportWriter) -> Result<()>,
    ) -> Result<()> {
        let mut failure = None;
        for writer in &mut self.writers {
            if let Err(error) = method(writer.as_mut()) {
                failure = Some(error);
            }
        }
        failure.map_or(Ok(()), Err)
    }
}
impl ExportWriter for HtmlAndJsonWriter {
    fn format(&self) -> Format {
        Format::Both
    }
    fn start(&mut self, s: &Settings, e: &Environment) -> Result<()> {
        self.invoke(|w| w.start(s, e))
    }
    fn write_dialogs_start(&mut self, d: &DialogsInfo) -> Result<()> {
        self.invoke(|w| w.write_dialogs_start(d))
    }
    fn write_dialog_start(&mut self, d: &DialogInfo) -> Result<()> {
        self.invoke(|w| w.write_dialog_start(d))
    }
    fn write_dialog_slice(&mut self, d: &MessagesSlice) -> Result<()> {
        self.invoke(|w| w.write_dialog_slice(d))
    }
    fn write_dialog_end(&mut self) -> Result<()> {
        self.invoke(|w| w.write_dialog_end())
    }
    fn write_dialogs_end(&mut self) -> Result<()> {
        self.invoke(|w| w.write_dialogs_end())
    }
    fn finish(&mut self) -> Result<()> {
        self.invoke(|w| w.finish())
    }
    fn abort(&mut self) {
        for writer in &mut self.writers {
            writer.abort();
        }
    }
    fn main_file_path(&self) -> PathBuf {
        self.writers[0].main_file_path()
    }
}
