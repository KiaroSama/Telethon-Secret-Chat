//! Real original SDK Client with private network I/O replaced in a test copy.
mod adapter;
mod review_export;
mod review_manager;
use kiaro_secret_chat::{
    binding::AuthorizationBinding,
    custody::IngressLimits,
    grammers_client::Client,
    grammers_mtsender::{InvocationError, SenderPool, UpdatesConfiguration, UpdatesReceiver},
    grammers_session::{
        SessionData,
        storages::MemorySession,
        updates::{MessageBoxes, UpdatesLike},
    },
    grammers_tl_types::{self as tl, Deserializable, Identifiable, Serializable},
    ingress::{DurableIngress, EncryptedIngressStorage},
};
use sha1::{Digest, Sha1};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
const FIRST: &[u8] = b"first complete original envelope";
const SECOND: &[u8] = b"second complete original envelope";
struct Server {
    owed: BTreeMap<i32, Vec<u8>>,
    requests: Vec<i32>,
    difference: bool,
    bot: bool,
}
fn state(qts: i32) -> tl::types::updates::State {
    tl::types::updates::State {
        pts: 10,
        qts,
        date: 123,
        seq: 1,
        unread_count: 0,
    }
}
fn message(qts: i32) -> tl::enums::EncryptedMessage {
    tl::types::EncryptedMessageService {
        random_id: qts as i64,
        chat_id: 7,
        date: 123,
        bytes: if qts == 41 { FIRST } else { SECOND }.to_vec(),
    }
    .into()
}
fn difference(qts: i32) -> tl::functions::updates::GetDifference {
    tl::functions::updates::GetDifference {
        pts: 10,
        pts_limit: None,
        pts_total_limit: None,
        date: 123,
        qts,
        qts_limit: None,
    }
}
fn self_user(bot: bool) -> Vec<u8> {
    // Independent TL fixture: vector<User> with only flags.self and user id.
    let mut bytes = vec![];
    for value in [
        0x1cb5c415u32,
        1,
        tl::types::User::CONSTRUCTOR_ID,
        (1 << 10) | if bot { 1 << 14 } else { 0 },
        0,
    ] {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.extend_from_slice(&123i64.to_le_bytes());
    if bot {
        bytes.extend_from_slice(&0i32.to_le_bytes());
    }
    bytes
}
fn decode<T: Deserializable>(bytes: &[u8]) -> T {
    let mut cursor = tl::deserialize::Cursor::from_slice(bytes);
    let value = T::deserialize(&mut cursor).unwrap();
    assert_eq!(cursor.pos(), bytes.len());
    value
}
impl Server {
    fn ack(&mut self, qts: i32) {
        self.requests.push(qts);
        self.owed.retain(|held, _| *held > qts);
    }
    fn invoke(&mut self, dc: i32, body: Vec<u8>) -> Result<Vec<u8>, InvocationError> {
        assert_eq!(dc, 2);
        let id = u32::from_le_bytes(body[..4].try_into().unwrap());
        let args = &body[4..];
        match id {
            id if id == tl::functions::Ping::CONSTRUCTOR_ID => {
                let request: tl::functions::Ping = decode(args);
                Ok(tl::enums::Pong::from(tl::types::Pong {
                    msg_id: 0,
                    ping_id: request.ping_id,
                })
                .to_bytes())
            }
            id if id == tl::functions::users::GetUsers::CONSTRUCTOR_ID => {
                let request: tl::functions::users::GetUsers = decode(args);
                assert_eq!(request.id, vec![tl::enums::InputUser::UserSelf]);
                Ok(self_user(self.bot))
            }
            id if id == tl::functions::messages::ReceivedQueue::CONSTRUCTOR_ID => {
                let request: tl::functions::messages::ReceivedQueue = decode(args);
                self.ack(request.max_qts);
                Ok(Vec::<i64>::new().to_bytes())
            }
            id if id == tl::functions::updates::GetDifference::CONSTRUCTOR_ID => {
                let request: tl::functions::updates::GetDifference = decode(args);
                self.ack(request.qts);
                let response: tl::enums::updates::Difference =
                    if self.difference && !self.owed.is_empty() {
                        self.difference = false;
                        tl::types::updates::Difference {
                            new_messages: vec![],
                            new_encrypted_messages: vec![message(41), message(42)],
                            other_updates: vec![],
                            chats: vec![],
                            users: vec![],
                            state: state(42).into(),
                        }
                        .into()
                    } else {
                        tl::types::updates::DifferenceEmpty { date: 123, seq: 1 }.into()
                    };
                Ok(response.to_bytes())
            }
            _ => panic!("unexpected offline SDK RPC constructor"),
        }
    }
}
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mode = std::env::args().nth(1).unwrap();
    if mode == "adapter" {
        tokio::time::timeout(std::time::Duration::from_secs(90), async {
            adapter::run().await;
            review_export::run().await;
            review_manager::run().await;
        })
        .await
        .unwrap();
        return;
    }
    assert!(matches!(
        mode.as_str(),
        "live" | "difference" | "explicit" | "bot" | "capacity"
    ));
    let dir = tempfile::tempdir().unwrap();
    let key: [u8; 256] = std::array::from_fn(|i| i as u8);
    let hash = Sha1::digest(key);
    let id = i64::from_le_bytes(hash[12..20].try_into().unwrap());
    let binding =
        AuthorizationBinding::from_session_parts(123, id, 2, "offline-sdk-generation".into())
            .unwrap();
    let store =
        EncryptedIngressStorage::open(dir.path().join("custody"), [9; 32], binding, 1024 * 1024)
            .unwrap();
    let ingress = DurableIngress::new(
        store,
        IngressLimits::new(if mode == "capacity" { 1 } else { 32 }, 1024 * 1024, 3600.0).unwrap(),
    )
    .unwrap();
    let mut session = SessionData {
        home_dc: 2,
        ..Default::default()
    };
    session.dc_options.get_mut(&2).unwrap().auth_key = Some(key);
    let pool = SenderPool::new(Arc::new(MemorySession::from(session)), 123);
    let client = Client::new(pool.handle.clone());
    let mut machine = MessageBoxes::new();
    machine.set_state(state(40));
    if mode == "difference" {
        machine = MessageBoxes::load(machine.session_state());
    }
    ingress
        .prepare(&pool.handle, Some(machine.durable_checkpoint().unwrap()))
        .await
        .unwrap();
    let model = Arc::new(Mutex::new(Server {
        owed: [(41, FIRST.to_vec()), (42, SECOND.to_vec())].into(),
        requests: vec![],
        difference: mode == "difference",
        bot: mode == "bot",
    }));
    let server_model = Arc::clone(&model);
    let live = pool.runner.offline_fixture_updates();
    let handle = pool.handle.clone();
    let server = tokio::spawn(pool.runner.run_offline_fixture(move |dc, request| {
        let model = Arc::clone(&server_model);
        async move { model.lock().unwrap().invoke(dc, request) }
    }));
    let transport = ingress.transport(client.clone()).await;
    if mode == "bot" {
        assert!(
            transport.is_err(),
            "bot authorization cannot own secret-chat ingress"
        );
        assert!(
            client
                .invoke(&tl::functions::messages::ReceivedQueue { max_qts: 40 })
                .await
                .is_err()
        );
        handle.quit();
        server.await.unwrap();
        println!("SDK receipt and deleting-server frontier verified");
        return;
    }
    let transport = transport.unwrap();
    transport.require_strong_ingress().unwrap();
    assert!(
        client
            .invoke(&tl::functions::messages::ReceivedQueue { max_qts: 41 })
            .await
            .is_err()
    );
    assert!(model.lock().unwrap().requests.is_empty());
    let mut receiver = UpdatesReceiver::create(
        pool.handle.thin,
        pool.handle.session,
        pool.updates,
        UpdatesConfiguration::default(),
    )
    .await
    .unwrap();
    if mode == "capacity" {
        for (qts, seq) in [(41, 2), (42, 3)] {
            live.send(UpdatesLike::Updates(
                tl::types::Updates {
                    updates: vec![
                        tl::types::UpdateNewEncryptedMessage {
                            message: message(qts),
                            qts,
                        }
                        .into(),
                    ],
                    users: vec![],
                    chats: vec![],
                    date: 123,
                    seq,
                }
                .into(),
            ))
            .await
            .unwrap();
            let result = tokio::time::timeout(std::time::Duration::from_secs(2), receiver.next())
                .await
                .unwrap();
            if qts == 41 {
                assert_eq!(result.unwrap().0.len(), 1);
            } else {
                assert!(result.is_err());
            }
        }
        assert_eq!(
            MessageBoxes::from_durable_checkpoint(&ingress.checkpoint().unwrap())
                .unwrap()
                .session_state()
                .qts,
            41
        );
        assert!(
            client
                .invoke(&tl::functions::messages::ReceivedQueue { max_qts: 41 })
                .await
                .is_err()
        );
        assert_eq!(model.lock().unwrap().owed.len(), 2);
        assert!(model.lock().unwrap().requests.is_empty());
        assert_eq!(ingress.pending_batches().unwrap().len(), 1);
        handle.quit();
        server.await.unwrap();
        println!("SDK receipt and deleting-server frontier verified");
        return;
    }
    if mode != "difference" {
        live.send(UpdatesLike::Updates(
            tl::types::Updates {
                updates: vec![
                    tl::types::UpdateNewEncryptedMessage {
                        message: message(41),
                        qts: 41,
                    }
                    .into(),
                    tl::types::UpdateNewEncryptedMessage {
                        message: message(42),
                        qts: 42,
                    }
                    .into(),
                ],
                users: vec![],
                chats: vec![],
                date: 123,
                seq: 2,
            }
            .into(),
        ))
        .await
        .unwrap();
    }
    let output = tokio::time::timeout(std::time::Duration::from_secs(2), receiver.next())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(output.0.len(), 2);
    let pending = ingress.pending_batches().unwrap();
    assert_eq!(pending.len(), 1);
    let held = pending[0].envelope.as_bytes().to_vec();
    assert!(held.windows(FIRST.len()).any(|bytes| bytes == FIRST));
    assert!(held.windows(SECOND.len()).any(|bytes| bytes == SECOND));
    assert_eq!(model.lock().unwrap().owed.len(), 2);
    assert_eq!(
        MessageBoxes::from_durable_checkpoint(&ingress.checkpoint().unwrap())
            .unwrap()
            .session_state()
            .qts,
        42
    );
    let before = model.lock().unwrap().requests.len();
    assert!(client.invoke(&difference(43)).await.is_err());
    assert_eq!(model.lock().unwrap().requests.len(), before);
    if mode == "explicit" {
        client
            .invoke(&tl::functions::messages::ReceivedQueue { max_qts: 42 })
            .await
            .unwrap();
    } else {
        client.invoke(&difference(42)).await.unwrap();
    }
    assert!(model.lock().unwrap().owed.is_empty());
    assert_eq!(
        ingress.pending_batches().unwrap()[0].envelope.as_bytes(),
        held
    );
    ingress.fence().unwrap();
    assert!(
        client
            .invoke(&tl::functions::messages::ReceivedQueue { max_qts: 42 })
            .await
            .is_err()
    );
    assert!(transport.require_strong_ingress().is_err());
    handle.quit();
    server.await.unwrap();
    println!("SDK receipt and deleting-server frontier verified");
}
