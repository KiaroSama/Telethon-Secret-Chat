//! Opt-in caller application for dedicated, already-authorized test accounts.
//! Merely compiling or collecting the live tests never reads credentials.
mod account;
pub mod cases;
mod pump;
pub use account::{Account, Config};
use kiaro_secret_chat::{
    chat::ChatState,
    manager::{Event, MessageReceived},
    media::FileSource,
};
pub use pump::Pump;
use std::{fmt, path::Path, time::Duration};
pub type Failure = Box<dyn std::error::Error + Send + Sync>;
pub type TestResult<T = ()> = Result<T, Failure>;
pub const HUMAN: Duration = Duration::from_secs(180);
pub const PAIR: Duration = Duration::from_secs(60);
pub(super) static LIVE_RUN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
#[derive(Debug)]
pub struct Refused(pub &'static str);
impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Refused {}
pub fn require(condition: bool, message: &'static str) -> TestResult {
    if condition {
        Ok(())
    } else {
        Err(Refused(message).into())
    }
}
pub fn nonce() -> TestResult<String> {
    if let Ok(code) = std::env::var("TSC_TEST_NONCE") {
        require(
            !code.is_empty()
                && code.len() <= 64
                && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
            "The explicitly configured public test nonce is invalid",
        )?;
        return Ok(code);
    }
    Ok(hex::encode(kiaro_secret_chat::crypto::random_bytes::<6>()?))
}
pub async fn received(account: &Account, id: i32, expected: &str) -> TestResult<MessageReceived> {
    let event=account.recorder.wait_for("MessageReceived",HUMAN,|event|matches!(event,Event::MessageReceived(message) if message.chat_id==id&&message.text.contains(expected))).await?;
    match event {
        Event::MessageReceived(message) => Ok(*message),
        _ => Err(Refused("The expected message event was not received").into()),
    }
}
pub async fn texts(sender: &Account, receiver: &Account, id: i32, label: &str) -> TestResult {
    let tag = nonce()?;
    let expected: Vec<_> = (0..3).map(|n| format!("pair {label} {tag} {n}")).collect();
    for text in &expected {
        sender.manager.send_message(id, text, None, None).await?;
    }
    received(
        receiver,
        id,
        expected
            .last()
            .ok_or(Refused("The ordered message set is empty"))?,
    )
    .await?;
    let got: Vec<_> = receiver
        .recorder
        .of("MessageReceived")
        .into_iter()
        .filter_map(|event| match event {
            Event::MessageReceived(message)
                if message.chat_id == id && expected.contains(&message.text) =>
            {
                Some(message.text)
            }
            _ => None,
        })
        .collect();
    require(
        got == expected,
        "The real server delivered messages in a different order",
    )
}
pub async fn rekey_pair(a: &Account, b: &Account, id: i32) -> TestResult {
    let before = a.manager.status(id)?.key_fingerprint;
    a.manager.rekey(id).await?;
    tokio::time::timeout(PAIR, async {
        loop {
            let aa = a.manager.status(id)?;
            let bb = b.manager.status(id)?;
            if aa.key_fingerprint == bb.key_fingerprint
                && aa.key_fingerprint != before
                && !aa.exchange_in_progress
                && !bb.exchange_in_progress
            {
                return Ok::<(), kiaro_secret_chat::error::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .map_err(|_| Refused("The real peers did not settle on a new shared key"))??;
    Ok(())
}
pub async fn file(
    sender: &Account,
    receiver: &Account,
    id: i32,
    path: &Path,
    size: usize,
) -> TestResult {
    let mut content = vec![0; size];
    getrandom::fill(&mut content).map_err(|_| Refused("File fixture randomness is unavailable"))?;
    let random_id = sender
        .manager
        .send_file(
            id,
            FileSource::bytes(content.clone(), "pair-fixture.bin")?,
            Default::default(),
        )
        .await?;
    let event=receiver.recorder.wait_for("MessageReceived",PAIR,|event|matches!(event,Event::MessageReceived(message) if message.chat_id==id&&message.random_id==random_id)).await?;
    let Event::MessageReceived(message) = event else {
        return Err(Refused("The uploaded file event is missing").into());
    };
    let reference = message
        .media_reference
        .as_ref()
        .ok_or(Refused("The file event has no verified media reference"))?;
    let saved = receiver.manager.save_file(reference, path).await?;
    let actual =
        std::fs::read(saved).map_err(|_| Refused("The received file could not be read"))?;
    require(
        actual == content,
        "The real server file round trip changed its content",
    )
}
pub async fn await_peer_closed(a: &Account, id: i32, deleted: bool) -> TestResult {
    tokio::time::timeout(PAIR, async {
        loop {
            let done = if deleted {
                !a.manager.list()?.iter().any(|chat| chat.id == id)
            } else {
                a.manager.status(id)?.state == ChatState::Closed
            };
            if done {
                return Ok::<(), Failure>(());
            }
            a.force_difference().await?;
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    })
    .await
    .map_err(|_| {
        Refused("The peer did not observe the terminal chat update after synchronization")
    })??;
    Ok(())
}
