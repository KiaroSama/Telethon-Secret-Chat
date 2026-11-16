use kiaro_secret_chat::{
    grammers_client::Client,
    grammers_session::{Session, storages::MemorySession},
};
use std::sync::{Arc, Mutex};
pub async fn run(client: &Client, session: &Arc<MemorySession>, model: &Arc<Mutex<super::Model>>) {
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| client
            .iter_download(&super::File { dc: 2, size: 1 })
            .chunk_size(12288)))
        .is_err(),
        "a configured imprecise chunk limit must divide 1 MiB"
    );
    let mut dc = session.dc_option(4).unwrap().unwrap();
    dc.auth_key = Some([9; 256]);
    session.set_dc_option(&dc).await.unwrap();
    model.lock().unwrap().auth_failure = true;
    let result = client
        .iter_download(&super::File { dc: 4, size: 1 })
        .next()
        .await;
    assert!(result.is_err());
    assert_eq!(
        model.lock().unwrap().imports,
        2,
        "a replaced DC key must receive a new authorization import"
    );
    assert_eq!(model.lock().unwrap().exports, 2);
    model.lock().unwrap().auth_failure = false;
    let before = model.lock().unwrap().download_queries.len();
    model.lock().unwrap().boundary_download = true;
    let mut download = client
        .iter_download(&super::File {
            dc: 2,
            size: 2 * 1024 * 1024,
        })
        .chunk_size(4096)
        .skip_chunks(1)
        .chunk_size(512 * 1024);
    for expected in [524288, 520192, 524288] {
        assert_eq!(download.next().await.unwrap().unwrap().len(), expected);
    }
    assert_eq!(
        &model.lock().unwrap().download_queries[before..],
        &[
            (4096, 524288, false),
            (528384, 520192, true),
            (1048576, 524288, false)
        ],
        "small skips must retain the configured size while bounding each actual request"
    );
    model.lock().unwrap().boundary_download = false;
}
