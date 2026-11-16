//! Typed synthetic Telegram peer for the real original SDK transport adapters.
use super::super::{decode, self_user};
use kiaro_secret_chat::grammers_mtsender::{InvocationError, RpcError};
use kiaro_secret_chat::grammers_tl_types::{self as tl, Identifiable, Serializable};
use std::collections::BTreeMap;
#[derive(Default)]
pub struct Server {
    pub calls: Vec<(i32, u32)>,
    pub failure: Option<&'static str>,
    pub binding_mode: u8,
    pub dh_mode: u8,
    pub discard_ok: bool,
    pub download: Vec<u8>,
    pub files: BTreeMap<i64, Vec<u8>>,
    pub sent: Vec<(u32, i64, Vec<u8>)>,
    pub migrate: bool,
    pub wrappers: Vec<i64>,
}
pub fn rpc(name: &str) -> InvocationError {
    InvocationError::Rpc(RpcError::from(tl::types::RpcError {
        error_code: 400,
        error_message: name.into(),
    }))
}
fn boxed<T: Serializable + Identifiable>(value: &T) -> Vec<u8> {
    let mut bytes = T::CONSTRUCTOR_ID.to_le_bytes().to_vec();
    value.serialize(&mut |part| bytes.extend_from_slice(part));
    bytes
}
fn config() -> Vec<u8> {
    let mut value = serde_json::Map::new();
    for name in [
        "default_p2p_contacts",
        "preload_featured_stickers",
        "revoke_pm_inbox",
        "blocked_mode",
        "force_try_ipv6",
        "test_mode",
    ] {
        value.insert(name.into(), false.into());
    }
    for name in [
        "date",
        "expires",
        "chat_size_max",
        "megagroup_size_max",
        "forwarded_count_max",
        "online_update_period_ms",
        "offline_blur_timeout_ms",
        "offline_idle_timeout_ms",
        "online_cloud_timeout_ms",
        "notify_cloud_delay_ms",
        "notify_default_delay_ms",
        "push_chat_period_ms",
        "push_chat_limit",
        "edit_time_limit",
        "revoke_time_limit",
        "revoke_pm_time_limit",
        "rating_e_decay",
        "stickers_recent_limit",
        "channels_read_media_period",
        "call_receive_timeout_ms",
        "call_ring_timeout_ms",
        "call_connect_timeout_ms",
        "call_packet_timeout_ms",
        "caption_length_max",
        "message_length_max",
        "webfile_dc_id",
    ] {
        value.insert(name.into(), 1.into());
    }
    value.insert("this_dc".into(), 2.into());
    value.insert("dc_options".into(), serde_json::json!([]));
    value.insert("dc_txt_domain_name".into(), "fixture".into());
    value.insert("me_url_prefix".into(), "https://fixture.invalid/".into());
    boxed(&serde_json::from_value::<tl::types::Config>(value.into()).unwrap())
}
fn authorizations(mode: u8) -> Vec<u8> {
    let one = tl::types::Authorization {
        current: true,
        official_app: false,
        password_pending: false,
        encrypted_requests_disabled: false,
        call_requests_disabled: false,
        unconfirmed: false,
        hash: 456,
        device_model: "fixture".into(),
        platform: "fixture".into(),
        system_version: "fixture".into(),
        api_id: 123,
        app_name: "fixture".into(),
        app_version: "fixture".into(),
        date_created: 1,
        date_active: 1,
        ip: "127.0.0.1".into(),
        country: "ZZ".into(),
        region: "fixture".into(),
    };
    let items = match mode {
        2 => vec![],
        3 => vec![one.clone().into(), one.into()],
        _ => vec![one.into()],
    };
    boxed(&tl::types::account::Authorizations {
        authorization_ttl_days: 7,
        authorizations: items,
    })
}
impl Server {
    pub fn invoke(&mut self, dc: i32, mut body: Vec<u8>) -> Result<Vec<u8>, InvocationError> {
        let mut id = u32::from_le_bytes(body[..4].try_into().unwrap());
        if id == tl::functions::InvokeWithTakeout::<tl::functions::users::GetUsers>::CONSTRUCTOR_ID
        {
            self.wrappers
                .push(i64::from_le_bytes(body[4..12].try_into().unwrap()));
            body.drain(..12);
            id = u32::from_le_bytes(body[..4].try_into().unwrap());
        }
        if id==tl::functions::InvokeWithMessagesRange::<tl::functions::users::GetUsers>::CONSTRUCTOR_ID{
   assert_eq!(u32::from_le_bytes(body[4..8].try_into().unwrap()),tl::types::MessageRange::CONSTRUCTOR_ID);assert_eq!(i32::from_le_bytes(body[8..12].try_into().unwrap()),1);assert_eq!(i32::from_le_bytes(body[12..16].try_into().unwrap()),20);body.drain(..16);id=u32::from_le_bytes(body[..4].try_into().unwrap());
  }
        self.calls.push((dc, id));
        if let Some(name) = self.failure.take() {
            return Err(rpc(name));
        }
        let args = &body[4..];
        macro_rules! request {
            ($ty:ty) => {
                decode::<$ty>(args)
            };
        }
        Ok(match id {
            id if id == tl::functions::users::GetUsers::CONSTRUCTOR_ID => {
                let r = request!(tl::functions::users::GetUsers);
                assert_eq!(r.id, vec![tl::enums::InputUser::UserSelf]);
                if self.binding_mode == 4 {
                    Vec::<tl::enums::User>::new().to_bytes()
                } else {
                    self_user(self.binding_mode == 1)
                }
            }
            id if id == tl::functions::account::GetAuthorizations::CONSTRUCTOR_ID => {
                authorizations(self.binding_mode)
            }
            id if id == tl::functions::help::GetConfig::CONSTRUCTOR_ID => config(),
            id if id == tl::functions::messages::GetDhConfig::CONSTRUCTOR_ID => {
                let r = request!(tl::functions::messages::GetDhConfig);
                assert_eq!((r.version, r.random_length), (0, 0));
                if self.dh_mode == 1 {
                    boxed(&tl::types::messages::DhConfigNotModified { random: vec![] })
                } else {
                    boxed(&tl::types::messages::DhConfig {
                        g: 2,
                        p: hex_prime(),
                        version: 1,
                        random: vec![],
                    })
                }
            }
            id if id == tl::functions::messages::RequestEncryption::CONSTRUCTOR_ID => {
                let r = request!(tl::functions::messages::RequestEncryption);
                assert_eq!(r.random_id, 77);
                assert_eq!(r.g_a, vec![2; 256]);
                boxed(&tl::types::EncryptedChatWaiting {
                    id: 7,
                    access_hash: 88,
                    date: 1,
                    admin_id: 123,
                    participant_id: 456,
                })
            }
            id if id == tl::functions::messages::AcceptEncryption::CONSTRUCTOR_ID => {
                let r = request!(tl::functions::messages::AcceptEncryption);
                let tl::enums::InputEncryptedChat::Chat(peer) = r.peer;
                assert_eq!((peer.chat_id, peer.access_hash), (7, 88));
                assert_eq!(r.g_b, vec![3; 256]);
                boxed(&tl::types::EncryptedChat {
                    id: 7,
                    access_hash: 88,
                    date: 1,
                    admin_id: 123,
                    participant_id: 456,
                    g_a_or_b: r.g_b,
                    key_fingerprint: r.key_fingerprint,
                })
            }
            id if id == tl::functions::messages::DiscardEncryption::CONSTRUCTOR_ID => {
                let r = request!(tl::functions::messages::DiscardEncryption);
                assert_eq!(r.chat_id, 7);
                self.discard_ok.to_bytes()
            }
            id if id == tl::functions::messages::SendEncrypted::CONSTRUCTOR_ID => {
                let r = request!(tl::functions::messages::SendEncrypted);
                self.sent.push((id, r.random_id, r.data));
                boxed(&tl::types::messages::SentEncryptedMessage { date: 1 })
            }
            id if id == tl::functions::messages::SendEncryptedService::CONSTRUCTOR_ID => {
                let r = request!(tl::functions::messages::SendEncryptedService);
                self.sent.push((id, r.random_id, r.data));
                boxed(&tl::types::messages::SentEncryptedMessage { date: 1 })
            }
            id if id == tl::functions::messages::SendEncryptedFile::CONSTRUCTOR_ID => {
                let r = request!(tl::functions::messages::SendEncryptedFile);
                self.sent.push((id, r.random_id, r.data));
                boxed(&tl::types::messages::SentEncryptedFile {
                    date: 1,
                    file: tl::types::EncryptedFile {
                        id: 9,
                        access_hash: 99,
                        size: 80,
                        dc_id: 4,
                        key_fingerprint: 123,
                    }
                    .into(),
                })
            }
            id if id == tl::functions::upload::SaveFilePart::CONSTRUCTOR_ID => {
                let r = request!(tl::functions::upload::SaveFilePart);
                let file = self.files.entry(r.file_id).or_default();
                assert_eq!(file.len(), r.file_part as usize * 512 * 1024);
                file.extend(r.bytes);
                true.to_bytes()
            }
            id if id == tl::functions::upload::SaveBigFilePart::CONSTRUCTOR_ID => {
                let r = request!(tl::functions::upload::SaveBigFilePart);
                let file = self.files.entry(r.file_id).or_default();
                assert_eq!(file.len(), r.file_part as usize * 512 * 1024);
                file.extend(r.bytes);
                assert!(r.file_total_parts > 20);
                true.to_bytes()
            }
            id if id == tl::functions::upload::GetFile::CONSTRUCTOR_ID => {
                let r = request!(tl::functions::upload::GetFile);
                if self.migrate {
                    self.migrate = false;
                    return Err(rpc("FILE_MIGRATE_4"));
                }
                assert_eq!(dc, 4);
                let start = (r.offset as usize).min(self.download.len());
                let end = (start + r.limit as usize).min(self.download.len());
                boxed(&tl::types::upload::File {
                    r#type: tl::enums::storage::FileType::FileUnknown,
                    mtime: 1,
                    bytes: self.download[start..end].to_vec(),
                })
            }
            id if id == tl::functions::help::GetCountriesList::CONSTRUCTOR_ID => {
                boxed(&tl::types::help::CountriesListNotModified {})
            }
            id if id == tl::functions::account::InitTakeoutSession::CONSTRUCTOR_ID => {
                boxed(&tl::types::account::Takeout { id: 44 })
            }
            id if id == tl::functions::account::FinishTakeoutSession::CONSTRUCTOR_ID => {
                true.to_bytes()
            }
            id if id == tl::functions::messages::GetSplitRanges::CONSTRUCTOR_ID => {
                vec![tl::enums::MessageRange::from(tl::types::MessageRange {
                    min_id: 1,
                    max_id: 20,
                })]
                .to_bytes()
            }
            id if id == tl::functions::messages::GetChats::CONSTRUCTOR_ID => {
                boxed(&tl::types::messages::Chats { chats: vec![] })
            }
            id if id == tl::functions::messages::GetHistory::CONSTRUCTOR_ID
                || id == tl::functions::messages::GetMessages::CONSTRUCTOR_ID
                || id == tl::functions::messages::Search::CONSTRUCTOR_ID =>
            {
                boxed(&tl::types::messages::Messages {
                    messages: vec![],
                    topics: vec![],
                    chats: vec![],
                    users: vec![],
                })
            }
            _ => panic!("An unexpected native adapter RPC was issued"),
        })
    }
}
pub fn hex_prime() -> Vec<u8> {
    let text = include_str!(concat!(
        env!("SDK_FIXTURE_ORIGIN"),
        "/tests/fixtures/dh-prime.hex"
    ));
    text.trim()
        .as_bytes()
        .chunks_exact(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}
