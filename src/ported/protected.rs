//! Synthetic protected storage validation boundary. This is not the production store.

use serde_json::{Map, Value, json};

type Outcome<T> = Result<T, String>;

fn refused(chat: Option<i64>) -> String {
    let prefix = chat.map_or_else(String::new, |id| format!("[chat {id}] "));
    format!("ValueError: TransientRefused: {prefix}transient processing refused")
}

pub fn validate_binding(binding: Value) -> Outcome<()> {
    let fields = binding.as_object().ok_or_else(|| refused(None))?;
    let user = binding["user_id"].as_i64().is_some_and(|id| id > 0);
    let auth = binding["auth_key_id"].as_i64().is_some_and(|id| id != 0)
        || binding["auth_key_id"].as_u64().is_some_and(|id| id != 0);
    let dc = binding["dc_id"]
        .as_i64()
        .is_some_and(|id| (1..=1000).contains(&id));
    if fields.len() == 3 && user && auth && dc {
        Ok(())
    } else {
        Err(refused(None))
    }
}

fn fixture_chat() -> Map<String, Value> {
    let mut chat = Map::new();
    for key in "admin_id closed_reason dh_g dh_prime exchange_id exchange_secret gap_end initial_key_hash key key_fingerprint participant_id pending_key previous_key rekey_role resend_due".split_whitespace() {
        chat.insert(key.into(),Value::Null);
    }
    for key in "created_at in_seq_no messages_since_rekey out_seq_no peer_in_seq_no rekeyed_at ttl wrapper_layer".split_whitespace() {
        chat.insert(key.into(),json!(0));
    }
    for key in ["gap_requested", "new_key_confirmed"] {
        chat.insert(key.into(), json!(false));
    }
    let fields = json!({"access_hash":8,"handshake":{},"id":7,"is_outbound":true,"layer":46,"peer_user_id":9,"state":"requested"});
    if let Value::Object(fields) = fields {
        chat.extend(fields);
    }
    chat.sort_keys();
    chat
}

pub fn protected_fixture(mut binding: Value, change: String) -> Outcome<Value> {
    let mode = if change == "ordinary" {
        None
    } else {
        Some("protected-v1")
    };
    if mode != Some("protected-v1") {
        return Err(refused(None));
    }
    let mut chat = fixture_chat();
    let id = chat["id"].as_i64().ok_or_else(|| refused(None))?;
    if change == "queue" {
        return Err(refused(Some(id)));
    }
    validate_binding(binding.clone())?;
    let expected_keys = chat.len();
    if change == "extra" {
        chat.insert("unknown".into(), json!("synthetic"));
    }
    if change == "bad_counter" {
        chat.insert("in_seq_no".into(), json!(-1));
    }
    let stored_id = if change == "wrong_id" { id + 1 } else { id };
    if id != stored_id
        || chat.len() != expected_keys
        || !chat["in_seq_no"].as_i64().is_some_and(|value| value >= 0)
    {
        return Err(refused(Some(stored_id)));
    }
    if let Value::Object(fields) = &mut binding {
        fields.sort_keys();
    }
    Ok(json!({"binding":binding,"chat":chat,"format":1,"suspended":false}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn refuses_boolean_id_and_out_of_range_dc() {
        assert!(validate_binding(json!({"user_id":true,"auth_key_id":1,"dc_id":1})).is_err());
        assert!(validate_binding(json!({"user_id":1,"auth_key_id":1,"dc_id":1001})).is_err());
    }
}
