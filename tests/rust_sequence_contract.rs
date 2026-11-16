#![allow(clippy::unwrap_used)]
#[path = "support_manager/mod.rs"]
mod manager_support;
mod support_protocol;
use kiaro_secret_chat::{
    chat::ChatState,
    manager::{Event, Options, SecretChatManager},
    sequence,
    storage::{MemoryStorage, State, Storage},
    types,
};
use support_protocol::*;
#[test]
fn expected_parity_when_originator() {
    let mut c = chat(true, 0);
    sequence::accept(&mut c, from_wrapper(wrap(1, 0, 144))).unwrap();
    assert_eq!(c.in_seq_no, 1);
}
#[test]
fn expected_parity_when_recipient() {
    let mut c = chat(false, 0);
    sequence::accept(&mut c, from_wrapper(wrap(0, 1, 144))).unwrap();
    assert_eq!(c.in_seq_no, 1);
}
fn parity_refusal(outbound: bool, input: i32, output: i32) {
    let mut c = chat(outbound, 0);
    assert!(sequence::accept(&mut c, from_wrapper(wrap(input, output, 144))).is_err());
    assert_eq!(c.in_seq_no, 0);
}
macro_rules! parity {
    ($name:ident,$outbound:expr,$input:expr,$output:expr) => {
        #[test]
        fn $name() {
            parity_refusal($outbound, $input, $output);
        }
    };
}
parity!(wrong_originator_input, true, 0, 0);
parity!(wrong_originator_output, true, 1, 1);
parity!(wrong_both_originator, true, 0, 1);
parity!(wrong_recipient_input, false, 1, 1);
parity!(wrong_recipient_output, false, 0, 0);
#[test]
fn fatal_parity_originator() {
    let mut p = protocol();
    let bad = frame(&from_wrapper(wrap(0, 0, 144)).body);
    assert!(matches!(
        p.receive(7, &bad, None, 1),
        Err(kiaro_secret_chat::error::Error::Sequence)
    ));
}
#[test]
fn fatal_parity_recipient() {
    parity_refusal(false, 1, 1);
}
#[test]
fn reflected_local_message_refused() {
    let c = chat(true, 0);
    let mine = wrap(
        kiaro_secret_chat::framing::transform_in_seq_no(0, c.is_outbound).unwrap(),
        kiaro_secret_chat::framing::transform_out_seq_no(0, c.is_outbound).unwrap(),
        144,
    );
    let mut c = c;
    assert!(sequence::accept(&mut c, from_wrapper(mine)).is_err());
}
#[test]
fn refused_message_does_not_advance_counter() {
    let mut c = chat(true, 0);
    assert!(sequence::accept(&mut c, from_wrapper(wrap(0, 0, 144))).is_err());
    assert_eq!(c.in_seq_no, 0);
    assert!(c.deliveries.is_empty());
}
#[test]
fn decreasing_wrapper_layer_refused() {
    let mut c = chat(true, 0);
    sequence::accept(&mut c, envelope(0, 0, 144, true)).unwrap();
    assert!(sequence::accept(&mut c, envelope(1, 0, 101, true)).is_err());
}
#[test]
fn increasing_wrapper_layer_accepted() {
    let mut c = chat(true, 0);
    sequence::accept(&mut c, envelope(0, 0, 101, true)).unwrap();
    sequence::accept(&mut c, envelope(1, 0, 144, true)).unwrap();
    assert_eq!(c.capability_layer, 144);
}
#[test]
fn notified_capability_does_not_raise_wrapper_floor() {
    let mut p = protocol();
    let action = types::DecryptedMessageService {
        random_id: 77,
        action: types::DecryptedMessageActionNotifyLayer { layer: 143 }.into(),
    };
    let mut wrapper = wrap(1, 0, 73);
    wrapper.message = action.into();
    p.receive(7, &frame(&from_wrapper(wrapper).body), None, 1)
        .unwrap();
    p.apply_service_delivery(7, 0).unwrap();
    assert_eq!(p.chat_record(7).unwrap().capability_layer, 143);
    p.receive(7, &frame(&envelope(1, 0, 73, true).body), None, 1)
        .unwrap();
    assert_eq!(p.snapshot(7).unwrap().state, ChatState::Ready);
}
#[tokio::test]
async fn fatal_echo_closes_through_manager_before_failure_event() {
    let (transport, _) = manager_support::Fake::pair();
    let mut state = State::empty("11".repeat(32)).unwrap();
    state.chats.insert(7, chat(true, 0));
    let mut store = MemoryStorage::default();
    store.commit(&state).unwrap();
    let manager =
        SecretChatManager::new(transport, store, "11".repeat(32), Options::default()).unwrap();
    manager.stop_auto_save_secret_chats().await.unwrap();
    manager.start().await.unwrap();
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    for name in ["ChatClosed", "DecryptFailed"] {
        let sink = std::sync::Arc::clone(&events);
        manager
            .on(name, name, move |event| {
                sink.lock().unwrap().push(match event {
                    Event::ChatClosed { .. } => "ChatClosed",
                    Event::DecryptFailed { .. } => "DecryptFailed",
                    _ => "unexpected",
                });
                Ok(())
            })
            .unwrap();
    }
    let message = grammers_tl_types::types::EncryptedMessageService {
        random_id: 9,
        chat_id: 7,
        date: 1,
        bytes: frame(&envelope(0, 50, 144, true).body),
    };
    manager
        .handle_update(
            &grammers_tl_types::types::UpdateNewEncryptedMessage {
                message: message.into(),
                qts: 1,
            }
            .into(),
        )
        .await;
    assert_eq!(manager.status(7).unwrap().state, ChatState::Closed);
    assert_eq!(*events.lock().unwrap(), ["ChatClosed", "DecryptFailed"]);
    manager.stop().await.unwrap();
}
