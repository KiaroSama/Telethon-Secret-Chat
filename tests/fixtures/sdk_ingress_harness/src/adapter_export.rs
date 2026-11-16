use super::server::Server;
use kiaro_secret_chat::{
    export::{
        fetch_client::{ExportClient, FetchError, RequestScope},
        fetch_files::LazyFile,
        grammers_client::GrammersExportClient,
        model::FileLocation,
    },
    grammers_client::Client,
};
use serde_json::json;
use std::sync::{Arc, Mutex};
pub async fn run(client: Client, model: &Arc<Mutex<Server>>) {
    let export = GrammersExportClient::new(client);
    let _ = export.client();
    let request = json!({"_":"users.GetUsers","id":[{"_":"InputUserSelf"}]});
    let users = export
        .request(Default::default(), request.clone())
        .await
        .unwrap();
    assert_eq!(users[0]["id"], 123);
    let scoped = RequestScope {
        takeout: Some(44),
        range: Some(json!({"_":"MessageRange","min_id":1,"max_id":20})),
    };
    export.request(scoped, request).await.unwrap();
    assert_eq!(model.lock().unwrap().wrappers, vec![44]);
    for request in [
        json!({"_":"help.GetCountriesList","lang_code":"en","hash":0}),
        json!({"_":"account.InitTakeoutSession","contacts":false,"message_users":false,"message_chats":false,"message_megagroups":false,"message_channels":false,"files":false,"file_max_size":null}),
        json!({"_":"account.FinishTakeoutSession","success":true}),
        json!({"_":"messages.GetSplitRanges"}),
        json!({"_":"messages.GetChats","id":[7]}),
        json!({"_":"messages.GetMessages","id":[{"_":"InputMessageID","id":7}]}),
        json!({"_":"messages.GetHistory","peer":{"_":"InputPeerSelf"},"offset_id":0,"offset_date":0,"add_offset":0,"limit":1,"max_id":0,"min_id":0,"hash":0}),
    ] {
        export.request(Default::default(), request).await.unwrap();
    }
    assert!(
        export
            .request(Default::default(), json!({"_":"users.GetUsers","id":false}))
            .await
            .is_err()
    );
    assert!(
        export
            .request(Default::default(), json!({"_":"unsupported.request"}))
            .await
            .is_err()
    );
    model.lock().unwrap().failure = Some("TAKEOUT_INIT_DELAY_7");
    let result = export
        .request(
            Default::default(),
            json!({"_":"users.GetUsers","id":[{"_":"InputUserSelf"}]}),
        )
        .await;
    assert!(matches!(result,Err(FetchError::Rpc{kind,..})if kind=="TakeoutInitDelayError"));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("export/raw.bin");
    let mut target = LazyFile::new(&path);
    let bytes = vec![0x71; 128 * 1024 + 17];
    model.lock().unwrap().download = bytes.clone();
    model.lock().unwrap().migrate = true;
    let location = FileLocation {
        dc_id: 2,
        data: json!({"_":"InputDocumentFileLocation","id":9,"access_hash":99,"file_reference":[],"thumb_size":""}),
    };
    export
        .download(&location, bytes.len() as u64, &mut target)
        .await
        .unwrap();
    target.finish().unwrap();
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}
