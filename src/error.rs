use thiserror::Error;

#[derive(Debug, Error)]
pub enum BotError {
    #[error("Telegram API error {code}: {description}")]
    TelegramApi { code: i64, description: String },

    #[error("Telegram request timed out (method: {method})")]
    TelegramTimeout { method: String },

    #[error("Telegram network error: {0}")]
    TelegramNetwork(#[from] reqwest::Error),

    #[error("Zabbix API error: {message}")]
    ZabbixApi { message: String },

    #[error("Zabbix graph error: {message}")]
    ZabbixGraph { message: String },

    #[error("Command timed out after {secs}s")]
    CommandTimeout { secs: u64 },

    #[error("Command unavailable: sudo not permitted for '{cmd}'. Add to sudoers.")]
    CommandUnavailable { cmd: String },

    #[error("Invalid argument '{input}': only alphanumeric and ._/:- allowed")]
    InvalidArgument { input: String },

    #[error("Config error: {0}")]
    Config(#[from] toml::de::Error),

    #[error("Speedtest error: {message}")]
    Speedtest { message: String },

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
