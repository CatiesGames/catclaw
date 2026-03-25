use thiserror::Error;

#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum CatClawError {
    #[error("config error: {0}")]
    Config(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("toml parse error: {0}")]
    TomlParse(#[from] toml::de::Error),

    #[error("toml serialize error: {0}")]
    TomlSerialize(#[from] toml::ser::Error),

    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("session error: {0}")]
    Session(String),

    #[error("agent error: {0}")]
    Agent(String),

    #[error("channel error: {0}")]
    Channel(String),

    #[error("claude cli error: {0}")]
    Claude(String),

    #[error("discord error: {0}")]
    Discord(String),

    #[error("telegram error: {0}")]
    Telegram(String),

    #[error("slack error: {0}")]
    Slack(String),

    #[error("update error: {0}")]
    Update(String),

    #[error("service error: {0}")]
    Service(String),

    #[error("social error: {0}")]
    Social(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, CatClawError>;
