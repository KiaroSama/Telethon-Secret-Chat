use kiaro_secret_chat::export::{Format, Settings, User, export_saved_records};
use std::path::Path;

pub fn example(records: &[serde_json::Value]) -> Result<(), Box<dyn std::error::Error>> {
    let me = User {
        id: 1,
        is_self: true,
        first_name: "Me".into(),
        ..Default::default()
    };
    let peer = User {
        id: 2,
        first_name: "Peer".into(),
        ..Default::default()
    };
    let settings = Settings {
        format: Format::Both,
        time_zone: Some("UTC".into()),
        ..Default::default()
    };
    let result = export_saved_records(
        records,
        Path::new("export"),
        &me,
        &peer,
        &settings,
        Some(Path::new("saved source")),
    )?;
    assert!(!result.takeout);
    Ok(())
}
