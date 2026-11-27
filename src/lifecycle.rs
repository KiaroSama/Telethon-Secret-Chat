use serde_json::Value;

use crate::errors::SecretChatError;

pub fn build_service_action(
    kind: &str,
    first: i64,
    last: i64,
    ids: Vec<i64>,
) -> Result<Option<Value>, SecretChatError> {
    crate::ported::actions::build(kind.to_owned(), first, last, ids).map_err(|err| {
        SecretChatError::ParameterRejected {
            chat_id: None,
            reason: err,
        }
    })
}

pub fn check_lock_guard(
    generation: i64,
    current_generation: i64,
    stopping: bool,
    installed: bool,
) -> Result<Value, SecretChatError> {
    crate::ported::locking::check(generation, current_generation, stopping, installed).map_err(
        |err| SecretChatError::ParameterRejected {
            chat_id: None,
            reason: err,
        },
    )
}

pub fn remember_early_update(
    initial: i64,
    present: bool,
    discarded: bool,
    incoming_discarded: bool,
) -> Result<Value, SecretChatError> {
    crate::ported::establishment::remember(initial, present, discarded, incoming_discarded).map_err(
        |err| SecretChatError::ParameterRejected {
            chat_id: None,
            reason: err,
        },
    )
}

pub fn retained_outbox_content_ids(
    ids: Vec<i64>,
    services: Vec<bool>,
) -> Result<Vec<i64>, SecretChatError> {
    crate::ported::outbox::retained(ids, services).map_err(|err| {
        SecretChatError::ParameterRejected {
            chat_id: None,
            reason: err,
        }
    })
}

pub fn receive_history_projection(
    ids: Vec<i64>,
    remove: Vec<i64>,
    limit: i64,
    forget: bool,
) -> Result<Value, SecretChatError> {
    crate::ported::receive::history(ids, remove, limit, forget).map_err(|err| {
        SecretChatError::ParameterRejected {
            chat_id: None,
            reason: err,
        }
    })
}

pub fn ogg_looks_like_voice(header: &[u8]) -> Result<Value, SecretChatError> {
    crate::ported::ogg_tags::looks_like_voice(header.to_vec()).map_err(|err| {
        SecretChatError::ParameterRejected {
            chat_id: None,
            reason: err,
        }
    })
}

pub fn temporary_owned_roundtrip(prefix: &str) -> Result<Vec<bool>, SecretChatError> {
    crate::ported::temporary::owned_roundtrip(prefix.to_owned()).map_err(|err| {
        SecretChatError::ParameterRejected {
            chat_id: None,
            reason: err,
        }
    })
}
