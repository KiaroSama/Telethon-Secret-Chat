use kiaro_secret_chat::{
    crypto::{self, Direction, SharedKey},
    export::{self, Format, Settings, User},
    protocol::Protocol,
    storage::MemoryStorage,
};
fn canonical(
    client: &kiaro_secret_chat::grammers_client::Client,
) -> &kiaro_secret_chat::grammers_client::Client {
    client
}
fn main() {
    let destination = std::env::args_os().nth(1).expect("destination argument");
    let destination = std::path::PathBuf::from(destination);
    let key = SharedKey::from_slice(&[7; 256]).expect("key");
    let frame = crypto::encrypt_frame(&key, b"installed Git consumer", Direction::Initiator)
        .expect("encrypt");
    assert_eq!(
        &*crypto::decrypt_frame(&key, &frame, Direction::Initiator).expect("decrypt"),
        b"installed Git consumer"
    );
    let mut protocol =
        Protocol::open(MemoryStorage::default(), "11".repeat(32)).expect("memory protocol");
    protocol.start().expect("start");
    assert!(protocol.snapshots().is_empty());
    protocol.stop().expect("stop");
    let own = User {
        id: 1,
        first_name: "Self".into(),
        is_self: true,
        ..Default::default()
    };
    let peer = User {
        id: 2,
        first_name: "Peer".into(),
        ..Default::default()
    };
    let options = Settings {
        format: Format::Both,
        ..Default::default()
    };
    let result = export::export_saved_records(&[], &destination, &own, &peer, &options, None)
        .expect("installed export");
    assert!(result.path.join("messages.html").is_file());
    assert!(result.path.join("result.json").is_file());
    let _ = canonical;
    println!("version\t{}", kiaro_secret_chat::VERSION);
    println!("public-api\timplemented");
}
