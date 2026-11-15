//! Four-message rekey protocol. The CommitKey ciphertext uses the previous key;
//! adoption and retention of that ciphertext commit as one state transition.
use crate::{
    chat::{Chat, ChatState, Exchange},
    crypto::random_id,
    dh::{DhConfig, SecretExponent, verify_fingerprint},
    enums::DecryptedMessageAction as Action,
    error::{Error, Result},
    protocol::{Effects, prepare_action},
    types,
};
pub const MESSAGE_TRIGGER: u64 = 100;
pub const AGE_TRIGGER: u64 = 7 * 24 * 60 * 60;
#[derive(Debug, PartialEq, Eq)]
pub enum Collision {
    AbandonTheirs,
    JoinTheirs,
    AbortBoth,
}
pub fn resolve_collision(mine: i64, theirs: i64) -> Collision {
    match mine.cmp(&theirs) {
        std::cmp::Ordering::Greater => Collision::AbandonTheirs,
        std::cmp::Ordering::Less => Collision::JoinTheirs,
        std::cmp::Ordering::Equal => Collision::AbortBoth,
    }
}
pub fn should_rekey(chat: &Chat, now: u64) -> bool {
    chat.exchange.is_none()
        && chat.previous_key.is_none()
        && chat.messages_since_rekey > 0
        && (chat.messages_since_rekey > MESSAGE_TRIGGER
            || now.saturating_sub(chat.rekeyed_at) > AGE_TRIGGER)
}
fn config(chat: &Chat) -> Result<DhConfig> {
    let dh = chat.dh.as_ref().ok_or(Error::DhParameters)?;
    DhConfig::new(dh.g, &dh.prime)
}
pub fn start(chat: &mut Chat) -> Result<Option<crate::protocol::SendEffect>> {
    chat.require_ready()?;
    if chat.exchange.is_some() || chat.previous_key.is_some() {
        return Ok(None);
    }
    let dh = config(chat)?;
    let secret = SecretExponent::generate()?;
    let id = random_id()?;
    let g_a = dh.public_value(&secret);
    let effect = prepare_action(
        chat,
        types::DecryptedMessageActionRequestKey {
            exchange_id: id,
            g_a,
        }
        .into(),
    )?;
    chat.exchange = Some(Exchange::Requested { id, secret });
    chat.state = ChatState::Rekeying;
    Ok(Some(effect))
}
fn clear(chat: &mut Chat) {
    chat.exchange = None;
    if chat.state == ChatState::Rekeying {
        chat.state = ChatState::Ready;
    }
}
fn adopt(chat: &mut Chat, key: crate::crypto::SharedKey, now: u64) {
    chat.previous_key = chat.key.take();
    chat.adopt_key(key);
    clear(chat);
    chat.messages_since_rekey = 0;
    chat.rekeyed_at = now;
    chat.new_key_confirmed = false;
}
pub fn observe_key(chat: &mut Chat, fingerprint: i64, now: u64) {
    let pending = match &chat.exchange {
        Some(Exchange::Accepted { key, .. }) if key.fingerprint() == fingerprint => {
            Some(key.clone())
        }
        _ => None,
    };
    if let Some(key) = pending {
        adopt(chat, key, now);
        chat.new_key_confirmed = true;
    } else if chat.previous_key.is_some()
        && chat
            .key
            .as_ref()
            .is_some_and(|k| k.fingerprint() == fingerprint)
    {
        chat.new_key_confirmed = true;
    }
}
pub fn retire_if_settled(chat: &mut Chat) {
    if chat.new_key_confirmed && chat.gaps.is_empty() && chat.gap_end.is_none() {
        chat.previous_key = None;
    }
}
pub fn handle(chat: &mut Chat, action: &Action, effects: &mut Effects, now: u64) -> Result<bool> {
    match action {
        Action::RequestKey(request) => {
            if chat.previous_key.is_some() {
                effects.sends.push(prepare_action(
                    chat,
                    types::DecryptedMessageActionAbortKey {
                        exchange_id: request.exchange_id,
                    }
                    .into(),
                )?);
                return Ok(false);
            }
            if let Some(exchange) = &chat.exchange {
                let Exchange::Requested { id, .. } = exchange else {
                    return Ok(false);
                };
                match resolve_collision(*id, request.exchange_id) {
                    Collision::AbandonTheirs => return Ok(false),
                    Collision::AbortBoth => {
                        clear(chat);
                        return Ok(true);
                    }
                    Collision::JoinTheirs => {}
                }
            }
            let dh = config(chat)?;
            let secret = SecretExponent::generate()?;
            let key = dh.shared_key(&request.g_a, &secret)?;
            let action = types::DecryptedMessageActionAcceptKey {
                exchange_id: request.exchange_id,
                g_b: dh.public_value(&secret),
                key_fingerprint: key.fingerprint(),
            };
            effects.sends.push(prepare_action(chat, action.into())?);
            chat.exchange = Some(Exchange::Accepted {
                id: request.exchange_id,
                key,
            });
            chat.state = ChatState::Rekeying;
            Ok(true)
        }
        Action::AcceptKey(accept) => {
            let Some(Exchange::Requested { id, secret }) = &chat.exchange else {
                return Ok(false);
            };
            if *id != accept.exchange_id {
                return Ok(false);
            }
            let key = config(chat)?.shared_key(&accept.g_b, secret);
            let key = match key.and_then(|key| {
                verify_fingerprint(&key, accept.key_fingerprint)?;
                Ok(key)
            }) {
                Ok(key) => key,
                Err(_) => {
                    effects.sends.push(prepare_action(
                        chat,
                        types::DecryptedMessageActionAbortKey {
                            exchange_id: accept.exchange_id,
                        }
                        .into(),
                    )?);
                    clear(chat);
                    return Ok(false);
                }
            };
            effects.sends.push(prepare_action(
                chat,
                types::DecryptedMessageActionCommitKey {
                    exchange_id: accept.exchange_id,
                    key_fingerprint: key.fingerprint(),
                }
                .into(),
            )?);
            adopt(chat, key, now);
            Ok(true)
        }
        Action::CommitKey(commit) => {
            let Some(Exchange::Accepted { id, key }) = &chat.exchange else {
                return Ok(false);
            };
            if *id != commit.exchange_id {
                return Ok(false);
            }
            verify_fingerprint(key, commit.key_fingerprint)?;
            let key = key.clone();
            adopt(chat, key, now);
            chat.new_key_confirmed = true;
            effects.sends.push(prepare_action(chat, Action::Noop)?);
            Ok(true)
        }
        Action::AbortKey(abort) => {
            let id = match &chat.exchange {
                Some(Exchange::Requested { id, .. }) | Some(Exchange::Accepted { id, .. }) => *id,
                None => return Ok(false),
            };
            if id != abort.exchange_id {
                return Ok(false);
            }
            clear(chat);
            Ok(true)
        }
        _ => Ok(false),
    }
}
