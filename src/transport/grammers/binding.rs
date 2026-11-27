use super::{Authorization, Client, SecretChatError, rejected, t, tl};
use sha2::{Digest, Sha256};

pub(super) async fn query_authorization(client: &Client) -> Result<Authorization, SecretChatError> {
    use tl::{
        enums as e,
        functions::{account, help, users},
    };
    let invalid = || {
        rejected(
            None,
            "caller client has no verified Telegram user authorization",
        )
    };
    let before = authorization_key_snapshot(client)?;
    let users = client
        .invoke(&users::GetUsers {
            id: vec![e::InputUser::UserSelf],
        })
        .await
        .map_err(|_| invalid())?;
    let [e::User::User(user)] = users.as_slice() else {
        return Err(invalid());
    };
    if !user.is_self || user.bot || user.id <= 0 {
        return Err(invalid());
    }
    let e::account::Authorizations::Authorizations(auths) = client
        .invoke(&account::GetAuthorizations {})
        .await
        .map_err(|_| invalid())?;
    current_authorization(&auths.authorizations)?;
    let e::Config::Config(config) = client
        .invoke(&help::GetConfig {})
        .await
        .map_err(|_| invalid())?;
    if !(1..=1000).contains(&config.this_dc)
        || config.this_dc != before.0
        || authorization_key_snapshot(client)? != before
    {
        return Err(invalid());
    }
    Ok(Authorization {
        user_id: user.id,
        key_tag: before.1,
        dc_id: config.this_dc,
    })
}

pub(super) fn current_authorization(
    authorizations: &[tl::enums::Authorization],
) -> Result<&t::Authorization, SecretChatError> {
    let invalid = || {
        rejected(
            None,
            "caller client has no verified Telegram user authorization",
        )
    };
    let mut current = authorizations.iter().filter_map(|entry| {
        let tl::enums::Authorization::Authorization(entry) = entry;
        entry.current.then_some(entry)
    });
    let device = current.next().ok_or_else(invalid)?;
    if current.next().is_some()
        || device.password_pending
        || device.unconfirmed
        || device.encrypted_requests_disabled
    {
        return Err(invalid());
    }
    Ok(device)
}

pub(super) fn authorization_key_snapshot(
    client: &Client,
) -> Result<(i32, [u8; 32]), SecretChatError> {
    client
        .authorization_key_tag()
        .map_err(|_| rejected(None, "caller authorization key is unavailable"))?
        .ok_or_else(|| rejected(None, "caller client has no authorization key"))
}

pub(super) fn authorization_identity(authorization: &Authorization, generation: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"kiaro-grammers-binding-v2\0");
    hash.update(authorization.user_id.to_le_bytes());
    hash.update(authorization.key_tag);
    hash.update(authorization.dc_id.to_le_bytes());
    hash.update(generation.as_bytes());
    hex::encode(hash.finalize())
}
