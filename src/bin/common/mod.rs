use std::fmt::Display;

pub enum Level {
    Info,
    Warning,
    Error,
    Debug,
}

pub fn log(level: Level, message: impl Display) {
    let severity = match level {
        Level::Info => "INFO",
        Level::Warning => "WARNING",
        Level::Error => "ERROR",
        Level::Debug => "DEBUG",
    };
    let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    if matches!(level, Level::Error) {
        eprintln!("{timestamp} [{severity}] {message}");
    } else {
        println!("{timestamp} [{severity}] {message}");
    }
}
