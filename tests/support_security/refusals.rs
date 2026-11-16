#![allow(clippy::unwrap_used)]
use super::manager_support;
use kiaro_secret_chat::{
    Serializable,
    chat::{Chat, ChatState, DhRecord, Envelope},
    crypto::{self, Direction, SharedKey},
    dh::DhConfig,
    enums,
    error::{Error, Result},
    files::FileKey,
    framing,
    manager::{Options, RpcFailure, SecretChatManager},
    protocol::Protocol,
    sequence,
    storage::{MemoryStorage, State, Storage},
    types,
};
pub const PLAIN: &str = "the quick brown fox jumps over the lazy dog";
pub const PRIVATE_TEXT: &str = "the text of a message nobody but two people should ever read";
pub const PEER_PUBLIC: u128 = 0x00C0_FFEE_1234_5678_9ABC_DEF0;
pub fn secret_key() -> [u8; 256] {
    std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(3))
}
pub fn catch<T>(result: Result<T>) -> Error {
    match result {
        Err(error) => error.with_chat(42),
        Ok(_) => panic!("refusal site unexpectedly accepted the input"),
    }
}
fn key() -> SharedKey {
    SharedKey::from_slice(&secret_key()).unwrap()
}
fn chat() -> Chat {
    let mut c = Chat::new(42, 1, 2, true, ChatState::Ready);
    c.adopt_key(key()).unwrap();
    c.capability_layer = 144;
    c.dh = Some(DhRecord {
        g: 2,
        prime: manager_support::prime(),
    });
    c
}
pub fn message() -> enums::DecryptedMessage {
    types::DecryptedMessage {
        no_webpage: false,
        silent: false,
        random_id: 1,
        ttl: 0,
        message: PLAIN.into(),
        media: None,
        entities: None,
        via_bot_name: None,
        reply_to_random_id: None,
        grouped_id: None,
    }
    .into()
}
pub fn wrapper(input: i32, output: i32, layer: i32) -> types::DecryptedMessageLayer {
    types::DecryptedMessageLayer {
        random_bytes: vec![0; 31],
        layer,
        in_seq_no: input,
        out_seq_no: output,
        message: message(),
    }
}
fn body(wrapper: types::DecryptedMessageLayer) -> Vec<u8> {
    let e: enums::DecryptedMessageLayer = wrapper.into();
    e.to_bytes()
}
fn frame() -> Vec<u8> {
    crypto::encrypt_frame(&key(), &body(wrapper(1, 0, 144)), Direction::Initiator).unwrap()
}
fn order(mut chat: Chat, wrapper: types::DecryptedMessageLayer) -> Error {
    catch(sequence::accept(
        &mut chat,
        Envelope {
            body: body(wrapper),
            file: None,
            date: 0,
        },
    ))
}
fn manager_refusal(state: Option<ChatState>, start: bool, uncertain_send: bool) -> Error {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let (transport, _) = manager_support::Fake::pair();
        let mut stored = State::empty("11".repeat(32)).unwrap();
        if let Some(state) = state.filter(|state| *state != ChatState::Requested) {
            let c = if state == ChatState::Ready {
                chat()
            } else {
                Chat::new(42, 1, 2, true, state)
            };
            stored.chats.insert(42, c);
        }
        if state == Some(ChatState::Requested) {
            transport.world.lock().unwrap().next_id = 42;
        }
        if uncertain_send {
            transport.world.lock().unwrap().fail_send = Some(RpcFailure::Transient);
        }
        let world = transport.world.clone();
        let mut storage = MemoryStorage::default();
        storage.commit(&stored).unwrap();
        let manager =
            SecretChatManager::new(transport, storage, "11".repeat(32), Options::default())
                .unwrap();
        if start {
            manager.start().await.unwrap();
            if state == Some(ChatState::Requested) {
                let requested = manager.create(manager_support::peer(2000)).await.unwrap();
                assert_eq!(requested.id, 42);
                assert_eq!(requested.state, ChatState::Requested);
            }
        } else {
            manager.stop().await.unwrap();
        }
        let error = catch(manager.send_message(42, PRIVATE_TEXT, None, None).await);
        if uncertain_send {
            assert_eq!(
                world.lock().unwrap().sent.len(),
                1,
                "uncertain-send refusal must actually reach transport after committing"
            );
        }
        manager.stop().await.unwrap();
        error
    })
}
struct FailedStorage;
impl Storage for FailedStorage {
    fn load(&self) -> Result<Option<State>> {
        Err(Error::Storage)
    }
    fn commit(&mut self, _: &State) -> Result<()> {
        Err(Error::Storage)
    }
}
pub fn family(name: &str) -> Error {
    match name {
        "ParameterRejected" => refusal("dh: bad residue"),
        "MessageRejected" => refusal("crypto: tampered"),
        "ChatNotReady" => manager_refusal(Some(ChatState::Requested), true, false),
        "ChatClosed" => manager_refusal(Some(ChatState::Closed), true, false),
        "StorageRequired" => catch(Protocol::open(FailedStorage, "11".repeat(32))),
        "LayerUnsupported" => refusal("framing: layer too low"),
        "ResendUnsatisfiable" => refusal("sequence: resend unsatisfiable"),
        "StoreCorrupt" => {
            let mut stored = State::empty("11".repeat(32)).unwrap();
            let mut c = chat();
            c.initial_key_hash = Some(secret_key().to_vec());
            stored.chats.insert(42, c);
            catch(stored.validate())
        }
        "UnknownChat" => manager_refusal(None, true, false),
        "ManagerStopping" => manager_refusal(None, false, false),
        "SendPending" => manager_refusal(Some(ChatState::Ready), true, true),
        _ => panic!("unknown native error family"),
    }
}
pub fn refusal(name: &str) -> Error {
    match name {
        "dh: bad residue" | "dh: composite prime" => {
            let mut prime = vec![0; 256];
            prime[0] = 128;
            prime[255] = if name == "dh: bad residue" { 1 } else { 7 };
            catch(DhConfig::new(2, &prime))
        }
        "dh: peer value" => {
            let config = DhConfig::new(2, &manager_support::prime()).unwrap();
            catch(config.check_peer(&[1]))
        }
        "crypto: wrong fingerprint" => {
            let wrong = SharedKey::from_slice(&[0; 256]).unwrap();
            catch(crypto::decrypt_frame(
                &wrong,
                &frame(),
                Direction::Initiator,
            ))
        }
        "crypto: tampered" => {
            let mut frame = frame();
            let last = frame.len() - 1;
            frame[last] ^= 1;
            catch(crypto::decrypt_frame(&key(), &frame, Direction::Initiator))
        }
        "crypto: truncated" => catch(crypto::decrypt_frame(
            &key(),
            &frame()[..30],
            Direction::Initiator,
        )),
        "framing: not a wrapper" => catch(framing::unwrap(&message().to_bytes())),
        "framing: unparseable" => catch(framing::unwrap(&[0xde, 0xad, 0xbe, 0xef])),
        "framing: layer too low" => catch(framing::unwrap(&body(wrapper(1, 0, 8)))),
        "sequence: parity" => order(chat(), wrapper(0, 0, 144)),
        "sequence: echo backwards" => {
            let mut c = chat();
            c.peer_in_seq_no = 5;
            c.out_seq_no = 9;
            order(c, wrapper(1, 0, 144))
        }
        "sequence: echo too far" => order(chat(), wrapper(9, 0, 144)),
        "sequence: layer backwards" => {
            let mut c = chat();
            c.wrapper_layer = 144;
            order(c, wrapper(1, 0, 101))
        }
        "sequence: resend unsatisfiable" => catch(sequence::answer_resend(&chat(), 1, 9)),
        "sequence: resend too wide" => catch(sequence::answer_resend(
            &chat(),
            1,
            1 + 2 * sequence::MAX_RESEND_COUNT + 2,
        )),
        "files: fingerprint" => {
            let bytes = secret_key();
            let file = FileKey::new(&bytes[..32], &bytes[224..]).unwrap();
            catch(file.verify(file.fingerprint().wrapping_add(1)))
        }
        "chat: not ready" => manager_refusal(Some(ChatState::Requested), true, false),
        "chat: closed" => manager_refusal(Some(ChatState::Closed), true, false),
        _ => panic!("unknown refusal site"),
    }
}
pub fn render(error: &Error) -> [String; 2] {
    [error.to_string(), format!("{error:?}")]
}
pub fn assert_private(error: &Error) {
    let sequential = (0..=255).collect::<Vec<u8>>();
    let ciphertext = [0xde, 0xad, 0xbe, 0xef].repeat(64);
    let material = [
        hex::encode(&sequential),
        format!("{sequential:?}"),
        hex::encode(&ciphertext),
        format!("{ciphertext:?}"),
        hex::encode(secret_key())[..32].into(),
        format!("{:?}", secret_key()),
        PEER_PUBLIC.to_string(),
        PRIVATE_TEXT.into(),
        PLAIN.into(),
        "DecryptedMessage".into(),
        "KeyExchange(".into(),
    ];
    for text in render(error) {
        for value in &material {
            assert!(
                !text.to_lowercase().contains(&value.to_lowercase()),
                "error leaked a public burned secret marker"
            );
        }
    }
}
pub fn assert_useful(error: &Error, chat: bool) {
    let text = error.to_string();
    assert!(
        text.len() > 20,
        "operator needs a concrete refusal description"
    );
    if chat {
        assert!(
            text.contains("42"),
            "a contextual refusal must identify its conversation"
        );
        assert!(text.len() + format!("{error:?}").len() > 40);
    }
    assert!(
        std::error::Error::source(error).is_some(),
        "context must retain a typed source"
    );
}
fn maximum_run(text: &str, allowed: impl Fn(u8) -> bool) -> usize {
    let mut longest = 0;
    let mut run = 0;
    for byte in text.bytes() {
        if allowed(byte) {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    longest
}
pub fn assert_no_opaque(error: &Error, site: bool) {
    for text in render(error) {
        assert!(
            maximum_run(&text, |b| b.is_ascii_hexdigit()) < if site { 24 } else { 32 },
            "opaque hex material reached an error"
        );
        if !site {
            assert!(
                maximum_run(&text, |b| b.is_ascii_alphanumeric()
                    || b == b'+'
                    || b == b'/'
                    || b == b'=')
                    < 40,
                "opaque base64 material reached an error"
            );
        }
    }
}
pub fn assert_site_shape(name: &str, error: &Error) {
    let Error::Context(context) = error else {
        panic!("missing native contextual error")
    };
    assert_eq!(context.chat_id(), 42);
    let source = context.refusal();
    let correct = match name {
        n if n.starts_with("dh: bad") || n == "dh: composite prime" => {
            matches!(source, Error::DhFailure(_))
        }
        "dh: peer value" => matches!(source, Error::PeerValue),
        n if n.starts_with("crypto:") => matches!(source, Error::FrameRejected),
        n if n.starts_with("framing:") => matches!(source, Error::WrapperRejected),
        n if n.starts_with("sequence: resend") => matches!(source, Error::ResendUnavailable),
        n if n.starts_with("sequence:") => matches!(source, Error::Sequence),
        "files: fingerprint" => matches!(source, Error::Fingerprint),
        "chat: not ready" => matches!(source, Error::ChatNotReady { chat_id: 42 }),
        "chat: closed" => matches!(source, Error::ChatClosed { chat_id: 42 }),
        _ => false,
    };
    assert!(correct, "wrong refusal family for {name}: {error:?}");
}
