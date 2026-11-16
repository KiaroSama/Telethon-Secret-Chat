//! Executor and completion controls with one deliberately occupied blocking worker.
use kiaro_secret_chat::{
    files::{FileKey, encrypt_file},
    grammers_client::Client,
    grammers_mtsender::SenderPool,
    grammers_session::{SessionData, storages::MemorySession},
    grammers_tl_types::{self as tl, Identifiable, Serializable},
    manager::Transport,
    media::MediaReference,
    transport::GrammersTransport,
    types,
};
use std::{
    sync::{Arc, mpsc},
    time::Duration,
};
use tokio::sync::Notify;
pub fn run() {
    let (plain_completed,ram_completed)=std::thread::spawn(|| {
        tokio::runtime::Builder::new_current_thread().enable_all().max_blocking_threads(1).build().unwrap().block_on(async {
            let mut data=SessionData{home_dc:2,..Default::default()};data.dc_options.get_mut(&2).unwrap().auth_key=Some([7;256]);
            let pool=SenderPool::new(Arc::new(MemorySession::from(data)),123);let client=Client::new(pool.handle.clone());
            let key=FileKey::new(&[4;32],&[8;32]).unwrap();let plaintext=vec![42;2*1024*1024];let mut encrypted=Vec::new();encrypt_file(&mut plaintext.as_slice(),&mut encrypted,plaintext.len() as u64,&key).unwrap();let ciphertext=Arc::new(encrypted);
            let first=Arc::new(Notify::new());let release=Arc::new(Notify::new());let last=Arc::new(Notify::new());
            let initial=first.clone();let resume=release.clone();let final_chunk=last.clone();
            let runner=tokio::spawn(pool.runner.run_offline_fixture(move|_,body| {let initial=initial.clone();let resume=resume.clone();let final_chunk=final_chunk.clone();let ciphertext=ciphertext.clone();async move {
                assert_eq!(u32::from_le_bytes(body[..4].try_into().unwrap()),tl::functions::upload::GetFile::CONSTRUCTOR_ID);let request=super::super::decode::<tl::functions::upload::GetFile>(&body[4..]);
                let is_plain=matches!(&request.location,tl::enums::InputFileLocation::InputDocumentFileLocation(location) if location.id==1);
                let bytes=if is_plain {initial.notify_one();resume.notified().await;vec![7]} else {let from=request.offset as usize;let to=(from+request.limit as usize).min(ciphertext.len());if to==ciphertext.len(){final_chunk.notify_one();}ciphertext[from..to].to_vec()};
                Ok(tl::enums::upload::File::File(tl::types::upload::File{r#type:tl::enums::storage::FileType::FileUnknown,mtime:0,bytes}).to_bytes())
            }}));
            let dir=tempfile::tempdir().unwrap();let path=dir.path().join("flushed.bin");let target=path.clone();let downloader=client.clone();
            let mut plain=tokio::spawn(async move {downloader.download_media(&Plain,target).await});
            tokio::time::timeout(Duration::from_secs(2),first.notified()).await.unwrap();
            let (unlock,locked)=mpsc::channel();let (entered_tx,entered_rx)=tokio::sync::oneshot::channel();let occupied=tokio::task::spawn_blocking(move||{entered_tx.send(()).unwrap();locked.recv().unwrap();});entered_rx.await.unwrap();release.notify_one();
            let plain_completed=tokio::select!{biased;result=&mut plain=>{result.unwrap().unwrap();true},_=tokio::time::sleep(Duration::from_millis(200))=>false};
            unlock.send(()).unwrap();occupied.await.unwrap();if !plain_completed {plain.await.unwrap().unwrap();assert_eq!(std::fs::read(path).unwrap(),[7]);}
            let reference=MediaReference::new(7,&tl::types::EncryptedFile{id:2,access_hash:88,size:plaintext.len() as i64,dc_id:2,key_fingerprint:key.fingerprint()},&types::DecryptedMessageMediaDocument{thumb:vec![],thumb_w:0,thumb_h:0,mime_type:"application/octet-stream".into(),size:plaintext.len() as i64,key:key.key().to_vec(),iv:key.iv().to_vec(),attributes:vec![],caption:String::new()}.into()).unwrap();
            let (unlock,locked)=mpsc::channel();let (entered_tx,entered_rx)=tokio::sync::oneshot::channel();let occupied=tokio::task::spawn_blocking(move||{entered_tx.send(()).unwrap();locked.recv().unwrap();});entered_rx.await.unwrap();
            let native=GrammersTransport::new(client);let expected=plaintext.clone();let mut ram=tokio::spawn(async move{let bytes=native.download_bytes(&reference,expected.len()).await.unwrap();assert_eq!(bytes,expected);});
            tokio::time::timeout(Duration::from_secs(2),last.notified()).await.unwrap();
            let ram_completed=tokio::select!{biased;result=&mut ram=>{result.unwrap();true},_=tokio::time::sleep(Duration::from_millis(200))=>false};
            unlock.send(()).unwrap();occupied.await.unwrap();if !ram_completed{ram.await.unwrap();}
            pool.handle.quit();runner.await.unwrap();drop(pool.updates);(plain_completed,ram_completed)
        })
    }).join().unwrap();
    assert!(
        !plain_completed && !ram_completed,
        "download completion must flush queued writes and RAM decryption must await its blocking worker (plain_completed={plain_completed},ram_completed={ram_completed})"
    );
}
struct Plain;
impl kiaro_secret_chat::grammers_client::media::Downloadable for Plain {
    fn to_raw_input_location(&self) -> Option<tl::enums::InputFileLocation> {
        Some(
            tl::types::InputDocumentFileLocation {
                id: 1,
                access_hash: 88,
                file_reference: vec![],
                thumb_size: String::new(),
            }
            .into(),
        )
    }
    fn size(&self) -> Option<usize> {
        Some(1)
    }
    fn dc_id(&self) -> Option<i32> {
        Some(2)
    }
}
