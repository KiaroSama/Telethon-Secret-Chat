use super::binding::current_authorization;
use super::*;
use grammers_mtsender::SenderPool;
use grammers_session::{Session, storages::MemorySession};
use std::time::Duration;

fn authorization(hash: i64) -> t::Authorization {
    t::Authorization {
        current: true,
        official_app: false,
        password_pending: false,
        encrypted_requests_disabled: false,
        call_requests_disabled: false,
        unconfirmed: false,
        hash,
        device_model: "synthetic test device".into(),
        platform: "test".into(),
        system_version: "1".into(),
        api_id: 1,
        app_name: "transport audit".into(),
        app_version: "1".into(),
        date_created: 1,
        date_active: 2,
        ip: "127.0.0.1".into(),
        country: "test".into(),
        region: "test".into(),
    }
}

async fn transport_fixture() -> (Arc<MemorySession>, SenderPool, GrammersTransport) {
    let session = Arc::new(MemorySession::default());
    let dc_id = session.home_dc_id().unwrap();
    let mut option = session.dc_option(dc_id).unwrap().unwrap();
    option.auth_key = Some([0x31; 256]);
    session.set_dc_option(&option).await.unwrap();
    // Retain the runner without starting it: any attempted RPC remains pending.
    let pool = SenderPool::new(session.clone(), 1);
    let client = Client::new(pool.handle.clone());
    let (_, key_tag) = client.authorization_key_tag().unwrap().unwrap();
    let transport = GrammersTransport {
        client,
        authorization: Authorization {
            user_id: 10,
            key_tag,
            dc_id,
        },
        identity: "synthetic-client-binding".into(),
        chats: Arc::default(),
        dh: Arc::default(),
    };
    (session, pool, transport)
}

#[test]
fn current_session_with_zero_management_hash_is_valid() {
    let authorizations = [authorization(0).into()];
    assert!(
        current_authorization(&authorizations).is_ok(),
        "the current session may have a zero management hash"
    );
}

#[test]
fn current_session_requires_one_usable_authorization() {
    assert!(current_authorization(&[]).is_err());
    assert!(current_authorization(&[authorization(7).into(), authorization(8).into()]).is_err());
    for field in 0..4 {
        let mut device = authorization(7);
        match field {
            0 => device.current = false,
            1 => device.password_pending = true,
            2 => device.unconfirmed = true,
            3 => device.encrypted_requests_disabled = true,
            _ => unreachable!(),
        }
        assert!(current_authorization(&[device.into()]).is_err());
    }
    assert!(current_authorization(&[authorization(7).into()]).is_ok());
}

#[test]
fn transport_identity_binds_the_user_key_dc_and_generation() {
    let authorization = Authorization {
        user_id: 10,
        key_tag: [0x31; 32],
        dc_id: 2,
    };
    let identity = authorization_identity(&authorization, "generation-one");
    assert_eq!(
        identity,
        authorization_identity(&authorization.clone(), "generation-one")
    );
    for changed in [
        Authorization {
            user_id: 11,
            ..authorization.clone()
        },
        Authorization {
            key_tag: [0x32; 32],
            ..authorization.clone()
        },
        Authorization {
            dc_id: 3,
            ..authorization.clone()
        },
    ] {
        assert_ne!(identity, authorization_identity(&changed, "generation-one"));
    }
    assert_ne!(
        identity,
        authorization_identity(&authorization, "generation-two")
    );
}

#[tokio::test]
async fn replaced_client_authorization_is_rejected_before_any_rpc() {
    let (session, _pool, transport) = transport_fixture().await;
    let dc_id = session.home_dc_id().unwrap();
    let mut option = session.dc_option(dc_id).unwrap().unwrap();
    option.auth_key = Some([0x32; 256]);
    session.set_dc_option(&option).await.unwrap();
    let result = tokio::time::timeout(Duration::from_millis(100), transport.verified_binding())
        .await
        .expect("a changed local authorization must be rejected before an RPC");
    assert!(matches!(
        result,
        Err(SecretChatError::ParameterRejected { .. })
    ));
}

#[tokio::test]
async fn removed_key_or_changed_home_dc_is_rejected_before_any_rpc() {
    for change_dc in [false, true] {
        let (session, _pool, transport) = transport_fixture().await;
        let dc_id = session.home_dc_id().unwrap();
        let mut option = session.dc_option(dc_id).unwrap().unwrap();
        if change_dc {
            option.id = if dc_id == 2 { 3 } else { 2 };
            session.set_dc_option(&option).await.unwrap();
            session.set_home_dc_id(option.id).await.unwrap();
        } else {
            option.auth_key = None;
            session.set_dc_option(&option).await.unwrap();
        }
        let result = tokio::time::timeout(Duration::from_millis(100), transport.verified_binding())
            .await
            .expect("an invalid local binding must be rejected before an RPC");
        assert!(matches!(
            result,
            Err(SecretChatError::ParameterRejected { .. })
        ));
    }
}

#[tokio::test]
async fn unchanged_client_authorization_proceeds_to_remote_verification() {
    let (_session, _pool, transport) = transport_fixture().await;
    assert!(
        tokio::time::timeout(Duration::from_millis(100), transport.verified_binding())
            .await
            .is_err(),
        "an unchanged local key still requires remote authorization verification"
    );
}

#[tokio::test]
async fn changed_key_does_not_poll_the_next_operation() {
    let (session, _pool, transport) = transport_fixture().await;
    let mut option = session
        .dc_option(session.home_dc_id().unwrap())
        .unwrap()
        .unwrap();
    option.auth_key = Some([0x32; 256]);
    session.set_dc_option(&option).await.unwrap();
    let mut polled = false;
    let result = transport
        .with_current_key(
            async {
                polled = true;
                Ok(42)
            },
            "synthetic operation failed",
        )
        .await;
    assert!(
        result.is_err(),
        "a new operation must refuse a changed authorization"
    );
    assert!(
        !polled,
        "authorization refusal must happen before remote side effects"
    );
}

#[tokio::test]
async fn key_change_during_operation_does_not_publish_the_result() {
    let (session, _pool, transport) = transport_fixture().await;
    let result = transport
        .with_current_key(
            async {
                let mut option = session
                    .dc_option(session.home_dc_id().unwrap())
                    .unwrap()
                    .unwrap();
                option.auth_key = Some([0x32; 256]);
                session.set_dc_option(&option).await.unwrap();
                Ok(42)
            },
            "synthetic operation failed",
        )
        .await;
    assert!(
        result.is_err(),
        "a completed operation must refuse a changed authorization"
    );
}

#[tokio::test]
async fn unchanged_key_preserves_operation_results() {
    let (_session, _pool, transport) = transport_fixture().await;
    assert_eq!(
        transport
            .with_current_key(async { Ok(42) }, "synthetic operation failed")
            .await
            .unwrap(),
        42
    );
}
