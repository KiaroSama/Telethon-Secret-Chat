// Kiaro supplement to grammers e61cf858. MIT OR Apache-2.0, as the parent crate.
//! Bounded, versioned custody codecs. These contain private update bodies and
//! deliberately have no Debug implementation. They are not session exports.
use super::defs::{Key, MessageBox, State, UpdateAndPeers, UpdatesLike};
use super::{Instant, LiveEntry, MessageBoxes, PossibleGap, PtsInfo};
use grammers_tl_types::{self as tl, Deserializable, Serializable};
use std::{
    collections::BTreeSet,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_ITEMS: usize = 4096;
const MAGIC: &[u8] = b"KGMBOX01";
const MAX_DEADLINE_MILLIS: i64 = i32::MAX as i64 * 1000 + 1000;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CustodyCodecError;
impl std::fmt::Display for CustodyCodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid durable update checkpoint")
    }
}
impl std::error::Error for CustodyCodecError {}
type Result<T> = std::result::Result<T, CustodyCodecError>;

struct Writer(Vec<u8>);
impl Writer {
    fn new() -> Self {
        Self(Vec::new())
    }
    fn int(&mut self, n: i32) {
        self.0.extend_from_slice(&n.to_le_bytes());
    }
    fn long(&mut self, n: i64) {
        self.0.extend_from_slice(&n.to_le_bytes());
    }
    fn count(&mut self, n: usize) -> Result<()> {
        if n > MAX_ITEMS {
            return Err(CustodyCodecError);
        }
        self.int(n as i32);
        Ok(())
    }
    fn bytes(&mut self, b: &[u8]) -> Result<()> {
        if b.len() > MAX_BYTES || self.0.len() + b.len() + 4 > MAX_BYTES {
            return Err(CustodyCodecError);
        }
        self.int(b.len() as i32);
        self.0.extend_from_slice(b);
        Ok(())
    }
    fn tl(&mut self, value: &impl Serializable) -> Result<()> {
        self.bytes(&value.to_bytes())
    }
    fn finish(self) -> Result<Vec<u8>> {
        if self.0.len() > MAX_BYTES {
            Err(CustodyCodecError)
        } else {
            Ok(self.0)
        }
    }
}
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}
impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() > MAX_BYTES {
            return Err(CustodyCodecError);
        }
        Ok(Self { bytes, pos: 0 })
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|end| *end <= self.bytes.len())
            .ok_or(CustodyCodecError)?;
        let b = &self.bytes[self.pos..end];
        self.pos = end;
        Ok(b)
    }
    fn int(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(
            self.take(4)?.try_into().map_err(|_| CustodyCodecError)?,
        ))
    }
    fn long(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(
            self.take(8)?.try_into().map_err(|_| CustodyCodecError)?,
        ))
    }
    fn count(&mut self) -> Result<usize> {
        let n = usize::try_from(self.int()?).map_err(|_| CustodyCodecError)?;
        if n > MAX_ITEMS {
            Err(CustodyCodecError)
        } else {
            Ok(n)
        }
    }
    fn bytes(&mut self) -> Result<&'a [u8]> {
        let n = usize::try_from(self.int()?).map_err(|_| CustodyCodecError)?;
        self.take(n)
    }
    fn tl<T: Deserializable>(&mut self) -> Result<T> {
        let bytes = self.bytes()?;
        let mut cursor = tl::deserialize::Cursor::from_slice(bytes);
        let value = T::deserialize(&mut cursor).map_err(|_| CustodyCodecError)?;
        if cursor.pos() != bytes.len() {
            return Err(CustodyCodecError);
        }
        Ok(value)
    }
    fn finish(self) -> Result<()> {
        if self.pos == self.bytes.len() {
            Ok(())
        } else {
            Err(CustodyCodecError)
        }
    }
}
fn write_key(w: &mut Writer, key: Key) {
    match key {
        Key::Common => w.int(0),
        Key::Secondary => w.int(1),
        Key::Channel(id) => {
            w.int(2);
            w.long(id);
        }
    }
}
fn read_key(r: &mut Reader<'_>) -> Result<Key> {
    match r.int()? {
        0 => Ok(Key::Common),
        1 => Ok(Key::Secondary),
        2 => {
            let id = r.long()?;
            if id <= 0 {
                return Err(CustodyCodecError);
            }
            Ok(Key::Channel(id))
        }
        _ => Err(CustodyCodecError),
    }
}
fn wall_ms() -> Result<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .ok_or(CustodyCodecError)
}
fn save_deadline(deadline: Instant, now: Instant, wall: i64) -> Result<i64> {
    let millis = i64::try_from(deadline.saturating_duration_since(now).as_millis())
        .map_err(|_| CustodyCodecError)?;
    if millis > MAX_DEADLINE_MILLIS {
        return Err(CustodyCodecError);
    }
    wall.checked_add(millis).ok_or(CustodyCodecError)
}
fn load_deadline(deadline: i64, now: Instant, wall: i64) -> Result<Instant> {
    let millis = deadline.saturating_sub(wall).max(0);
    if millis > MAX_DEADLINE_MILLIS {
        return Err(CustodyCodecError);
    }
    // Channel timeouts are signed 32-bit seconds, and may exceed the default
    // polling interval. Preserve every deadline the writer can represent; an
    // unrepresentable Instant is rejected instead of panicking.
    now.checked_add(Duration::from_millis(millis as u64))
        .ok_or(CustodyCodecError)
}
impl MessageBoxes {
    /// Prepare a caller-issued difference against existing durable counters.
    /// Unknown channel counters are not invented from the outgoing request.
    pub fn begin_durable_difference(&mut self, channel: Option<(i64, i32)>) -> Result<()> {
        if let Some((id, pts)) = channel {
            let key = Key::Channel(id);
            if self.entry(key).is_none_or(|entry| entry.pts != pts) {
                return Err(CustodyCodecError);
            }
            self.try_begin_get_diff(key);
        } else {
            if self.entry(Key::Common).is_none() && self.entry(Key::Secondary).is_none() {
                return Err(CustodyCodecError);
            }
            self.try_begin_get_diff(Key::Common);
            self.try_begin_get_diff(Key::Secondary);
        }
        Ok(())
    }
    pub fn durable_checkpoint(&self) -> Result<Vec<u8>> {
        if self.durable_anchor.get().is_none() {
            let anchor = (Instant::now(), wall_ms()?);
            let _ = self.durable_anchor.set(anchor);
        }
        let (now, wall) = *self.durable_anchor.get().ok_or(CustodyCodecError)?;
        let mut w = Writer::new();
        w.0.extend_from_slice(MAGIC);
        w.int(self.date);
        w.int(self.seq);
        w.long(save_deadline(self.next_deadline, now, wall)?);
        w.count(self.entries.len())?;
        for entry in &self.entries {
            write_key(&mut w, entry.key);
            w.int(entry.pts);
            w.long(save_deadline(entry.deadline, now, wall)?);
            match &entry.possible_gap {
                None => w.int(0),
                Some(gap) => {
                    w.int(1);
                    w.long(save_deadline(gap.deadline, now, wall)?);
                    w.count(gap.updates.len())?;
                    for update in &gap.updates {
                        w.tl(update)?;
                    }
                }
            }
        }
        w.count(self.getting_diff_for.len())?;
        for key in &self.getting_diff_for {
            write_key(&mut w, *key);
        }
        w.finish()
    }
    pub fn from_durable_checkpoint(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader::new(bytes)?;
        if r.take(MAGIC.len())? != MAGIC {
            return Err(CustodyCodecError);
        }
        let (now, wall) = (Instant::now(), wall_ms()?);
        let date = r.int()?;
        let seq = r.int()?;
        if date < 0 || seq < 0 {
            return Err(CustodyCodecError);
        }
        let next_deadline = load_deadline(r.long()?, now, wall)?;
        let n = r.count()?;
        let mut entries = Vec::with_capacity(n);
        let mut keys = BTreeSet::new();
        for _ in 0..n {
            let key = read_key(&mut r)?;
            if !keys.insert(key) || entries.last().is_some_and(|e: &LiveEntry| e.key >= key) {
                return Err(CustodyCodecError);
            }
            let pts = r.int()?;
            if pts < 0 {
                return Err(CustodyCodecError);
            }
            let deadline = load_deadline(r.long()?, now, wall)?;
            let possible_gap = match r.int()? {
                0 => None,
                1 => {
                    let deadline = load_deadline(r.long()?, now, wall)?;
                    let n = r.count()?;
                    let mut updates = Vec::with_capacity(n);
                    for _ in 0..n {
                        let update = r.tl()?;
                        let info = PtsInfo::from_gap_update(&update).ok_or(CustodyCodecError)?;
                        if info.key != key || pts.checked_add(info.count).is_none() {
                            return Err(CustodyCodecError);
                        }
                        updates.push(update);
                    }
                    Some(PossibleGap { deadline, updates })
                }
                _ => return Err(CustodyCodecError),
            };
            entries.push(LiveEntry {
                key,
                pts,
                deadline,
                possible_gap,
            });
        }
        let n = r.count()?;
        let mut getting_diff_for = Vec::with_capacity(n);
        let mut seen = BTreeSet::new();
        for _ in 0..n {
            let key = read_key(&mut r)?;
            if !keys.contains(&key) || !seen.insert(key) {
                return Err(CustodyCodecError);
            }
            getting_diff_for.push(key);
        }
        r.finish()?;
        Ok(Self {
            entries,
            date,
            seq,
            getting_diff_for,
            next_deadline,
            durable_anchor: std::sync::OnceLock::from((now, wall)),
        })
    }
}

/// Encode every UpdatesLike variant, including the request needed to reconstruct
/// short-sent updates and the affected-channel identity.
pub fn encode_live(value: &UpdatesLike) -> Result<Vec<u8>> {
    let mut w = Writer::new();
    match value {
        UpdatesLike::Captured(captured) => {
            w.int(8);
            w.bytes(&captured.envelope)?;
            match &captured.request {
                Some(request) => {
                    w.int(1);
                    w.bytes(request)?;
                }
                None => w.int(0),
            }
        }
        UpdatesLike::Updates(u) => {
            w.int(0);
            w.tl(u)?;
        }
        UpdatesLike::ShortSentMessage { request, update } => {
            w.int(1);
            w.tl(request)?;
            w.tl(update)?;
        }
        UpdatesLike::AffectedMessages(u) => {
            w.int(2);
            w.tl(u)?;
        }
        UpdatesLike::AffectedChannelMessages {
            affected,
            channel_id,
            message_ids,
        } => {
            w.int(3);
            w.tl(affected)?;
            w.long(*channel_id);
            w.count(message_ids.len())?;
            for id in message_ids {
                w.int(*id);
            }
        }
        UpdatesLike::InvitedUsers(u) => {
            w.int(4);
            w.tl(u)?;
        }
        UpdatesLike::ChatInviteJoinResult(u) => {
            w.int(5);
            w.tl(u)?;
        }
        UpdatesLike::ConnectionClosed => w.int(6),
        UpdatesLike::MalformedUpdates => w.int(7),
    }
    w.finish()
}
pub fn decode_live(bytes: &[u8]) -> Result<UpdatesLike> {
    let mut r = Reader::new(bytes)?;
    let value = match r.int()? {
        0 => UpdatesLike::Updates(r.tl()?),
        1 => {
            let bytes = r.bytes()?;
            let mut cursor = tl::deserialize::Cursor::from_slice(bytes);
            let id = u32::deserialize(&mut cursor).map_err(|_| CustodyCodecError)?;
            if id != <tl::functions::messages::SendMessage as tl::Identifiable>::CONSTRUCTOR_ID {
                return Err(CustodyCodecError);
            }
            let request = tl::functions::messages::SendMessage::deserialize(&mut cursor)
                .map_err(|_| CustodyCodecError)?;
            if cursor.pos() != bytes.len() {
                return Err(CustodyCodecError);
            }
            UpdatesLike::ShortSentMessage {
                request,
                update: r.tl()?,
            }
        }
        2 => UpdatesLike::AffectedMessages(r.tl()?),
        3 => {
            let affected = r.tl()?;
            let channel_id = r.long()?;
            let n = r.count()?;
            let mut message_ids = Vec::with_capacity(n);
            for _ in 0..n {
                message_ids.push(r.int()?);
            }
            UpdatesLike::AffectedChannelMessages {
                affected,
                channel_id,
                message_ids,
            }
        }
        4 => UpdatesLike::InvitedUsers(r.tl()?),
        5 => UpdatesLike::ChatInviteJoinResult(r.tl()?),
        6 => UpdatesLike::ConnectionClosed,
        7 => UpdatesLike::MalformedUpdates,
        8 => {
            let envelope = r.bytes()?.to_vec();
            let request = match r.int()? {
                0 => None,
                1 => Some(r.bytes()?.to_vec()),
                _ => return Err(CustodyCodecError),
            };
            UpdatesLike::captured(envelope, request)
        }
        _ => return Err(CustodyCodecError),
    };
    r.finish()?;
    Ok(value)
}
pub fn encode_output(value: &UpdateAndPeers) -> Result<Vec<u8>> {
    let mut w = Writer::new();
    w.count(value.0.len())?;
    for (update, state) in &value.0 {
        w.tl(update)?;
        w.int(state.date);
        w.int(state.seq);
        match state.message_box {
            None => w.int(0),
            Some(MessageBox::Common { pts }) => {
                w.int(1);
                w.int(pts);
            }
            Some(MessageBox::Secondary { qts }) => {
                w.int(2);
                w.int(qts);
            }
            Some(MessageBox::Channel { channel_id, pts }) => {
                w.int(3);
                w.long(channel_id);
                w.int(pts);
            }
        }
    }
    w.count(value.1.len())?;
    for user in &value.1 {
        w.tl(user)?;
    }
    w.count(value.2.len())?;
    for chat in &value.2 {
        w.tl(chat)?;
    }
    w.finish()
}
pub fn decode_output(bytes: &[u8]) -> Result<UpdateAndPeers> {
    let mut r = Reader::new(bytes)?;
    let n = r.count()?;
    let mut updates = Vec::with_capacity(n);
    for _ in 0..n {
        let update = r.tl()?;
        let date = r.int()?;
        let seq = r.int()?;
        let message_box = match r.int()? {
            0 => None,
            1 => Some(MessageBox::Common { pts: r.int()? }),
            2 => Some(MessageBox::Secondary { qts: r.int()? }),
            3 => Some(MessageBox::Channel {
                channel_id: r.long()?,
                pts: r.int()?,
            }),
            _ => return Err(CustodyCodecError),
        };
        updates.push((
            update,
            State {
                date,
                seq,
                message_box,
            },
        ));
    }
    let n = r.count()?;
    let mut users = Vec::with_capacity(n);
    for _ in 0..n {
        users.push(r.tl()?);
    }
    let n = r.count()?;
    let mut chats = Vec::with_capacity(n);
    for _ in 0..n {
        chats.push(r.tl()?);
    }
    r.finish()?;
    Ok((updates, users, chats))
}
