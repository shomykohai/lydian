use std::io::Write;
use std::str::FromStr;

use colored::Colorize;
use env_logger::fmt::Formatter;
use log::{Level, LevelFilter, Record};
use serde::{Deserialize, Deserializer, Serializer};

pub fn serialize<S>(level: &LevelFilter, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(level.as_str())
}

pub fn deserialize<'de, D>(deserializer: D) -> Result<LevelFilter, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    LevelFilter::from_str(&s).map_err(serde::de::Error::custom)
}

pub fn init_logger(log_level: LevelFilter) {
    let mut builder = env_logger::Builder::new();

    builder.format(|buf: &mut Formatter, record: &Record| {
        let level = match record.level() {
            Level::Trace => "TRACE:".bright_black(),
            Level::Debug => "DEBUG:".blue().bold(),
            Level::Info => "INFO:".green().bold(),
            Level::Warn => "WARN:".yellow().bold(),
            Level::Error => "ERROR:".red().bold(),
        };

        writeln!(buf, "{} {}", level, record.args())
    });

    builder.filter_module("hyper-util", LevelFilter::Info);
    builder.filter_module("hyper", LevelFilter::Info);

    /* Just to make sure :3 */
    if let Ok(rust_log) = std::env::var("RUST_LOG") {
        builder.parse_filters(&rust_log);
    }

    builder.filter_level(log_level).target(env_logger::Target::Stdout).init();
}
