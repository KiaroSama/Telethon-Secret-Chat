#![allow(clippy::unwrap_used)]
use grammers_tl_types as tl;
use kiaro_secret_chat::{
    dh::DhConfig,
    error::{Error, Result},
    files::{FileKey, save_file},
    manager::{Options, RpcFailure, SecretChatManager, Sent, Transport, transport::BoxFuture},
    media::{EncryptingReader, MediaReference, SourceReader},
    protocol::SendEffect,
    storage::{MemoryStorage, State, Storage},
};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    io::Read,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};
pub type Manager = SecretChatManager<MemoryStorage, Fake>;
#[allow(dead_code)] // Used by lifecycle and removal suites.
#[derive(Clone, Default)]
pub struct TestStorage {
    pub state: Arc<Mutex<Option<State>>>,
    pub failures: Arc<AtomicUsize>,
    pub commits: Arc<AtomicUsize>,
    pub fail_on_commit: Arc<AtomicUsize>,
}
impl Storage for TestStorage {
    fn load(&self) -> Result<Option<State>> {
        Ok(self.state.lock().unwrap().clone())
    }
    fn commit(&mut self, state: &State) -> Result<()> {
        let commit = self.commits.fetch_add(1, Ordering::AcqRel) + 1;
        if self.fail_on_commit.load(Ordering::Acquire) == commit {
            return Err(Error::Storage);
        }
        if self.failures.load(Ordering::Acquire) > 0 {
            self.failures.fetch_sub(1, Ordering::AcqRel);
            return Err(Error::Storage);
        }
        state.validate()?;
        *self.state.lock().unwrap() = Some(state.clone());
        Ok(())
    }
}
pub type DhHook = Box<dyn FnOnce() -> BoxFuture<'static, ()> + Send>;
pub type RequestHook = Box<dyn FnOnce(tl::enums::EncryptedChat) -> BoxFuture<'static, ()> + Send>;
#[derive(Default)]
pub struct World {
    pub next_id: i32,
    pub next_file: i64,
    pub requests: BTreeMap<i32, (i64, i64, Vec<u8>)>,
    pub request_by_identity: BTreeMap<(i64, i32), i32>,
    pub updates: BTreeMap<i64, VecDeque<tl::enums::Update>>,
    pub files: BTreeMap<i64, Vec<u8>>,
    pub fingerprints: BTreeMap<i64, i32>,
    pub sent: Vec<SendEffect>,
    pub request_ids: Vec<i32>,
    pub accepts: Vec<Vec<u8>>,
    pub discards: Vec<(i32, bool)>,
    pub discarded_by: BTreeSet<(i64, i32)>,
    pub uploads: usize,
    pub big_upload: bool,
    pub file_dc: Option<i32>,
    pub download_dcs: Vec<i32>,
    pub downloads: usize,
    pub fail_send: Option<RpcFailure>,
    pub fail_sends: BTreeMap<i64, RpcFailure>,
    pub fail_sends_from: BTreeMap<i64, RpcFailure>,
    pub active_sends: usize,
    pub maximum_sends: usize,
    pub fail_accept: Option<RpcFailure>,
    pub accept_response: Option<tl::enums::EncryptedChat>,
    pub fail_discard: bool,
    pub fail_request: Option<RpcFailure>,
    pub authorization: Option<String>,
    pub dh_prime_override: Option<Vec<u8>>,
    pub dh_gate: Option<Arc<Gate>>,
    pub dh_hook: Option<DhHook>,
    pub request_hook: Option<RequestHook>,
    pub upload_gate: Option<Arc<Gate>>,
    pub download_gate: Option<Arc<Gate>>,
    pub send_gate: Option<Arc<Gate>>,
    pub resume_hook: Option<Box<dyn FnOnce() + Send>>,
    pub fail_store_after_send: Option<TestStorage>,
}
#[derive(Default)]
pub struct Gate {
    pub entered: tokio::sync::Notify,
    pub release: tokio::sync::Notify,
}
struct InFlight(Arc<Mutex<World>>);
impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.lock().unwrap().active_sends -= 1;
    }
}
#[derive(Clone)]
pub struct Fake {
    pub user: i64,
    pub world: Arc<Mutex<World>>,
}
impl Fake {
    pub fn pair() -> (Self, Self) {
        let world = Arc::new(Mutex::new(World {
            next_id: 7,
            next_file: 77,
            ..Default::default()
        }));
        (
            Self {
                user: 1000,
                world: Arc::clone(&world),
            },
            Self { user: 2000, world },
        )
    }
}
pub fn prime() -> Vec<u8> {
    hex::decode(concat!(
        "FFFFFFFFFFFFFFFFC90FDAA22168C234C4C6628B80DC1CD129024E088A67CC74",
        "020BBEA63B139B22514A08798E3404DDEF9519B3CD3A431B302B0A6DF25F1437",
        "4FE1356D6D51C245E485B576625E7EC6F44C42E9A637ED6B0BFF5CB6F406B7ED",
        "EE386BFB5A899FA5AE9F24117C4B1FE649286651ECE45B3DC2007CB8A163BF05",
        "98DA48361C55D39A69163FA8FD24CF5F83655D23DCA3AD961C62F356208552BB",
        "9ED529077096966D670C354E4ABC9804F1746C08CA18217C32905E462E36CE3B",
        "E39E772C180E86039B2783A2EC07A28FB5C55DF06F4C52C9DE2BCBF695581718",
        "3995497CEA956AE515D2261898FA051015728E5A8AACAA68FFFFFFFFFFFFFFFF"
    ))
    .unwrap()
}
fn ready(
    id: i32,
    a: i64,
    b: i64,
    value: Vec<u8>,
    key_fingerprint: i64,
) -> tl::enums::EncryptedChat {
    tl::types::EncryptedChat {
        id,
        access_hash: 88,
        date: 1,
        admin_id: a,
        participant_id: b,
        g_a_or_b: value,
        key_fingerprint,
    }
    .into()
}
impl Transport for Fake {
    fn resume_io(&self) -> Result<()> {
        let hook = self.world.lock().unwrap().resume_hook.take();
        if let Some(hook) = hook {
            hook();
        }
        Ok(())
    }

    fn authorization_digest(&self) -> BoxFuture<'_, Result<Option<String>>> {
        Box::pin(async { Ok(self.world.lock().unwrap().authorization.clone()) })
    }
    fn dh_config(&self) -> BoxFuture<'_, Result<DhConfig>> {
        Box::pin(async move {
            let (gate, hook, prime) = {
                let mut world = self.world.lock().unwrap();
                (
                    world.dh_gate.clone(),
                    world.dh_hook.take(),
                    world.dh_prime_override.clone().unwrap_or_else(prime),
                )
            };
            if let Some(hook) = hook {
                hook().await;
            }
            if let Some(gate) = gate {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
            DhConfig::new(2, &prime)
        })
    }

    fn request(
        &self,
        user: tl::enums::InputUser,
        random_id: i32,
        g_a: Vec<u8>,
    ) -> BoxFuture<'_, std::result::Result<tl::enums::EncryptedChat, RpcFailure>> {
        Box::pin(async move {
            let tl::enums::InputUser::User(peer) = user else {
                return Err(RpcFailure::Permanent);
            };
            let (answer, hook, failure) = {
                let mut world = self.world.lock().unwrap();
                world.request_ids.push(random_id);
                let identity = (self.user, random_id);
                let id = if let Some(id) = world.request_by_identity.get(&identity) {
                    *id
                } else {
                    let id = world.next_id;
                    world.next_id += 1;
                    world.request_by_identity.insert(identity, id);
                    world
                        .requests
                        .insert(id, (self.user, peer.user_id, g_a.clone()));
                    world.updates.entry(peer.user_id).or_default().push_back(
                        tl::types::UpdateEncryption {
                            chat: tl::types::EncryptedChatRequested {
                                folder_id: None,
                                id,
                                access_hash: 88,
                                date: 1,
                                admin_id: self.user,
                                participant_id: peer.user_id,
                                g_a,
                            }
                            .into(),
                            date: 1,
                        }
                        .into(),
                    );
                    id
                };
                let answer: tl::enums::EncryptedChat = tl::types::EncryptedChatWaiting {
                    id,
                    access_hash: 88,
                    date: 1,
                    admin_id: self.user,
                    participant_id: peer.user_id,
                }
                .into();
                (answer, world.request_hook.take(), world.fail_request.take())
            };
            if let Some(hook) = hook {
                hook(answer.clone()).await;
            }
            if let Some(failure) = failure {
                return Err(failure);
            }
            Ok(answer)
        })
    }
    fn accept(
        &self,
        id: i32,
        _access: i64,
        g_b: Vec<u8>,
        fingerprint: i64,
    ) -> BoxFuture<'_, std::result::Result<tl::enums::EncryptedChat, RpcFailure>> {
        Box::pin(async move {
            let mut world = self.world.lock().unwrap();
            world.accepts.push(g_b.clone());
            if let Some(failure) = world.fail_accept {
                return Err(failure);
            }
            if let Some(response) = world.accept_response.take() {
                return Ok(response);
            }
            let (a, b, g_a) = world
                .requests
                .get(&id)
                .cloned()
                .ok_or(RpcFailure::ChatEnded)?;
            world.updates.entry(a).or_default().push_back(
                tl::types::UpdateEncryption {
                    chat: ready(id, a, b, g_b, fingerprint),
                    date: 1,
                }
                .into(),
            );
            Ok(ready(id, a, b, g_a, fingerprint))
        })
    }
    fn discard(&self, id: i32, delete_history: bool) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let mut world = self.world.lock().unwrap();
            world.discards.push((id, delete_history));
            if world.fail_discard {
                return Err(Error::Transport);
            }
            if !world.discarded_by.insert((self.user, id)) {
                return Err(Error::Transport);
            }
            if let Some((a, b, _)) = world.requests.get(&id) {
                let target = if self.user == *a { *b } else { *a };
                world.updates.entry(target).or_default().push_back(
                    tl::types::UpdateEncryption {
                        chat: tl::types::EncryptedChatDiscarded {
                            history_deleted: delete_history,
                            id,
                        }
                        .into(),
                        date: 1,
                    }
                    .into(),
                );
            }
            Ok(())
        })
    }
    fn send<'a>(
        &'a self,
        effect: &'a SendEffect,
    ) -> BoxFuture<'a, std::result::Result<Sent, RpcFailure>> {
        Box::pin(async move {
            let (gate, failure) = {
                let mut world = self.world.lock().unwrap();
                world.sent.push(effect.clone());
                world.active_sends += 1;
                world.maximum_sends = world.maximum_sends.max(world.active_sends);
                (
                    world.send_gate.clone(),
                    world
                        .fail_send
                        .take()
                        .or_else(|| world.fail_sends.get(&effect.outgoing.random_id).copied())
                        .or_else(|| world.fail_sends_from.get(&self.user).copied()),
                )
            };
            let _inflight = InFlight(Arc::clone(&self.world));
            tokio::task::yield_now().await;
            if let Some(gate) = gate {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
            if let Some(failure) = failure {
                return Err(failure);
            }
            let mut world = self.world.lock().unwrap();
            let (a, b, _) = world
                .requests
                .get(&effect.chat_id)
                .cloned()
                .ok_or(RpcFailure::ChatEnded)?;
            let target = if a == self.user { b } else { a };
            let file = if let Some(file) = &effect.outgoing.file {
                use tl::Deserializable;
                let input = tl::enums::InputEncryptedFile::from_bytes(file).unwrap();
                let (id, fingerprint) = match input {
                    tl::enums::InputEncryptedFile::Uploaded(f) => (f.id, f.key_fingerprint),
                    tl::enums::InputEncryptedFile::BigUploaded(f) => (f.id, f.key_fingerprint),
                    tl::enums::InputEncryptedFile::File(f) => {
                        (f.id, *world.fingerprints.get(&f.id).unwrap())
                    }
                    _ => return Err(RpcFailure::Permanent),
                };
                Some(tl::types::EncryptedFile {
                    id,
                    access_hash: 99,
                    size: world.files.get(&id).map_or(0, |bytes| bytes.len()) as i64,
                    dc_id: world.file_dc.unwrap_or(1),
                    key_fingerprint: fingerprint,
                })
            } else {
                None
            };
            let message = if effect.outgoing.service {
                tl::types::EncryptedMessageService {
                    random_id: effect.outgoing.random_id,
                    chat_id: effect.chat_id,
                    date: 1,
                    bytes: effect.outgoing.frame.clone(),
                }
                .into()
            } else {
                tl::types::EncryptedMessage {
                    random_id: effect.outgoing.random_id,
                    chat_id: effect.chat_id,
                    date: 1,
                    bytes: effect.outgoing.frame.clone(),
                    file: file
                        .clone()
                        .map(tl::enums::EncryptedFile::from)
                        .unwrap_or(tl::enums::EncryptedFile::Empty),
                }
                .into()
            };
            world
                .updates
                .entry(target)
                .or_default()
                .push_back(tl::types::UpdateNewEncryptedMessage { message, qts: 1 }.into());
            if let Some(store) = world.fail_store_after_send.take() {
                store.failures.store(1, Ordering::Release);
            }
            Ok(Sent { file })
        })
    }
    fn upload<'a>(
        &'a self,
        source: &'a mut SourceReader,
        key: &'a FileKey,
    ) -> BoxFuture<'a, Result<tl::enums::InputEncryptedFile>> {
        Box::pin(async move {
            let gate = self.world.lock().unwrap().upload_gate.clone();
            if let Some(gate) = gate {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
            let size = source.size;
            let mut reader = EncryptingReader::new(source.reader(), size, key.clone())?;
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).unwrap();
            let mut world = self.world.lock().unwrap();
            world.uploads += 1;
            let id = world.next_file;
            world.next_file += 1;
            world.files.insert(id, bytes);
            world.fingerprints.insert(id, key.fingerprint());
            if world.big_upload {
                return Ok(tl::types::InputEncryptedFileBigUploaded {
                    id,
                    parts: 22,
                    key_fingerprint: key.fingerprint(),
                }
                .into());
            }
            Ok(tl::types::InputEncryptedFileUploaded {
                id,
                parts: 1,
                md5_checksum: String::new(),
                key_fingerprint: key.fingerprint(),
            }
            .into())
        })
    }
    fn download_bytes<'a>(
        &'a self,
        reference: &'a MediaReference,
        maximum: usize,
    ) -> BoxFuture<'a, Result<Vec<u8>>> {
        Box::pin(async move {
            let gate = self.world.lock().unwrap().download_gate.clone();
            if let Some(gate) = gate {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
            let parts = reference.validate()?;
            if parts.size > maximum as u64 {
                return Err(Error::Capacity);
            }
            let bytes = {
                let mut world = self.world.lock().unwrap();
                world.downloads += 1;
                world.download_dcs.push(reference.dc_id);
                world
                    .files
                    .get(&reference.file_id)
                    .cloned()
                    .ok_or(Error::Media)?
            };
            let mut result = Vec::new();
            kiaro_secret_chat::files::decrypt_file(
                &mut bytes.as_slice(),
                &mut result,
                parts.size,
                &parts.key,
            )?;
            Ok(result)
        })
    }
    fn download<'a>(
        &'a self,
        reference: &'a MediaReference,
        path: &'a Path,
    ) -> BoxFuture<'a, Result<PathBuf>> {
        Box::pin(async move {
            let gate = self.world.lock().unwrap().download_gate.clone();
            if let Some(gate) = gate {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
            let parts = reference.validate()?;
            let bytes = {
                let mut world = self.world.lock().unwrap();
                world.downloads += 1;
                world.download_dcs.push(reference.dc_id);
                world
                    .files
                    .get(&reference.file_id)
                    .cloned()
                    .ok_or(Error::Media)?
            };
            let parent = path.parent().unwrap();
            std::fs::create_dir_all(parent).unwrap();
            save_file(
                &mut bytes.as_slice(),
                path,
                parts.size,
                &parts.key,
                reference.key_fingerprint,
            )?;
            Ok(path.to_owned())
        })
    }
}
#[allow(dead_code)] // Shared support is also used by protocol-only Strong fixtures.
pub fn peer(id: i64) -> tl::enums::InputUser {
    tl::types::InputUser {
        user_id: id,
        access_hash: 0,
    }
    .into()
}
#[allow(dead_code)] // Shared support is also compiled by file-backed restart suites.
pub fn managers() -> (Manager, Manager, Arc<Mutex<World>>) {
    let (a, b) = Fake::pair();
    let world = Arc::clone(&a.world);
    (
        Manager::new(
            a,
            MemoryStorage::default(),
            "11".repeat(32),
            Options::default(),
        )
        .unwrap(),
        Manager::new(
            b,
            MemoryStorage::default(),
            "22".repeat(32),
            Options::default(),
        )
        .unwrap(),
        world,
    )
}
pub async fn pump<A: Storage + 'static, B: Storage + 'static>(
    a: &SecretChatManager<A, Fake>,
    b: &SecretChatManager<B, Fake>,
    world: &Arc<Mutex<World>>,
) {
    for _ in 0..1000 {
        let next = {
            let mut world = world.lock().unwrap();
            world
                .updates
                .get_mut(&1000)
                .and_then(VecDeque::pop_front)
                .map(|update| (true, update))
                .or_else(|| {
                    world
                        .updates
                        .get_mut(&2000)
                        .and_then(VecDeque::pop_front)
                        .map(|update| (false, update))
                })
        };
        match next {
            None => return,
            Some((true, update)) => a.handle_update(&update).await,
            Some((false, update)) => b.handle_update(&update).await,
        }
    }
    panic!("Synthetic update queue did not converge")
}
#[allow(dead_code)] // Shared support is also compiled by file-backed restart suites.
pub async fn established() -> (Manager, Manager, Arc<Mutex<World>>, i32) {
    let (a, b, world) = managers();
    a.start().await.unwrap();
    b.start().await.unwrap();
    let id = a.create(peer(2000)).await.unwrap().id;
    pump(&a, &b, &world).await;
    b.accept(id).await.unwrap();
    pump(&a, &b, &world).await;
    (a, b, world, id)
}
