//! Desktop media data model. Ported from Telegram Desktop 7.2.10 (GPL-3.0).
use super::{model::*, model_peers::ContactInfo};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SharedContact {
    pub info: ContactInfo,
    pub vcard: File,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GeoPoint {
    pub latitude: f64,
    pub longitude: f64,
    pub valid: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Venue {
    pub point: GeoPoint,
    pub title: String,
    pub address: String,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Game {
    pub id: i64,
    pub short_name: String,
    pub title: String,
    pub description: String,
    pub bot_id: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Invoice {
    pub title: String,
    pub description: String,
    pub currency: String,
    pub amount: i64,
    pub receipt_msg_id: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PaidMedia {
    pub stars: i64,
    pub extended: Vec<Option<Media>>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PollAnswer {
    pub text: Vec<TextPart>,
    pub option: Vec<u8>,
    pub votes: i64,
    pub my: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Poll {
    pub id: i64,
    pub question: Vec<TextPart>,
    pub answers: Vec<PollAnswer>,
    pub total_votes: i64,
    pub closed: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TodoListItem {
    pub text: Vec<TextPart>,
    pub id: i64,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TodoList {
    pub others_can_append: bool,
    pub others_can_complete: bool,
    pub title: Vec<TextPart>,
    pub items: Vec<TodoListItem>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GiveawayStart {
    pub countries: Vec<String>,
    pub channels: Vec<i64>,
    pub additional_prize: String,
    pub until_date: i64,
    pub credits: i64,
    pub quantity: i64,
    pub months: i64,
    pub all: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GiveawayResults {
    pub channel: i64,
    pub winners: Vec<i64>,
    pub additional_prize: String,
    pub until_date: i64,
    pub launch_id: i64,
    pub additional_peers_count: i64,
    pub winners_count: i64,
    pub unclaimed_count: i64,
    pub months: i64,
    pub credits: i64,
    pub refunded: bool,
    pub all: bool,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UnsupportedMedia {}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MediaContent {
    Photo(Photo),
    Document(Document),
    SharedContact(SharedContact),
    GeoPoint(GeoPoint),
    Venue(Venue),
    Game(Game),
    Invoice(Invoice),
    PaidMedia(PaidMedia),
    Poll(Poll),
    TodoList(TodoList),
    GiveawayStart(GiveawayStart),
    GiveawayResults(GiveawayResults),
    UnsupportedMedia(UnsupportedMedia),
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Media {
    pub content: Option<MediaContent>,
    pub ttl: i64,
}
impl Media {
    pub fn file(&self) -> File {
        match &self.content {
            Some(MediaContent::Photo(p)) => p.image.file.clone(),
            Some(MediaContent::Document(d)) => d.file.clone(),
            Some(MediaContent::SharedContact(c)) => c.vcard.clone(),
            _ => File::default(),
        }
    }
    pub fn file_mut(&mut self) -> Option<&mut File> {
        match &mut self.content {
            Some(MediaContent::Photo(p)) => Some(&mut p.image.file),
            Some(MediaContent::Document(d)) => Some(&mut d.file),
            Some(MediaContent::SharedContact(c)) => Some(&mut c.vcard),
            _ => None,
        }
    }
    pub fn thumb(&self) -> Image {
        match &self.content {
            Some(MediaContent::Document(d)) => d.thumb.clone(),
            _ => Image::default(),
        }
    }
    pub fn reset_file(&mut self) {
        if let Some(file) = self.file_mut() {
            *file = File::default();
        }
    }
}
