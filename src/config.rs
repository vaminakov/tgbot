use indexmap::IndexMap;
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub bot: BotConfig,
    pub telegram: TelegramConfig,
    /// IndexMap preserves TOML insertion order — first entry is super-admin.
    pub admins: IndexMap<String, String>,
    pub zabbix: ZabbixConfig,
    #[serde(default)]
    pub speedtest: SpeedtestConfig,
    #[serde(default)]
    pub commands: Vec<CommandConfig>,
}

impl Config {
    pub fn load(path: &str) -> Result<Self, crate::error::BotError> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        if config.admins.is_empty() {
            return Err(crate::error::BotError::Config(
                <toml::de::Error as serde::de::Error>::custom("no admins configured"),
            ));
        }
        if config.telegram.token.trim().is_empty() {
            return Err(crate::error::BotError::Config(
                <toml::de::Error as serde::de::Error>::custom("telegram token is empty"),
            ));
        }
        Ok(config)
    }

    /// Chat ID of the super-admin (first entry in [admins]).
    pub fn super_admin_id(&self) -> Option<i64> {
        self.admins.keys().next().and_then(|k| k.parse().ok())
    }

    pub fn is_admin(&self, chat_id: i64) -> bool {
        self.admins.contains_key(&chat_id.to_string())
    }

    pub fn is_super_admin(&self, chat_id: i64) -> bool {
        self.super_admin_id() == Some(chat_id)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct BotConfig {
    pub mode: BotMode,
    #[serde(default = "defaults::webhook_path")]
    pub webhook_path: String,
    #[serde(default = "defaults::bind")]
    pub bind: String,
    #[serde(default = "defaults::exec_timeout")]
    pub exec_timeout_secs: u64,
    #[serde(default = "defaults::ip_whitelist")]
    pub webhook_ip_whitelist: IpWhitelistConfig,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum BotMode {
    Webhook,
    Polling,
}

/// Controls source IP filtering for the webhook endpoint.
#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum IpWhitelistConfig {
    /// "telegram" — use hardcoded Telegram IP ranges.
    /// "disabled" — skip check (use behind trusted reverse proxy).
    Named(String),
    /// Custom CIDR list, e.g. ["10.0.0.0/8"].
    Custom(Vec<String>),
}

#[derive(Debug, Deserialize, Clone)]
pub struct TelegramConfig {
    pub token: String,
    #[serde(default)]
    pub api_address: String,
    #[serde(default)]
    pub proxy: String,
    #[serde(default = "defaults::request_timeout")]
    pub request_timeout_secs: u64,
    #[serde(default = "defaults::request_retries")]
    pub request_retries: u32,
}

impl TelegramConfig {
    pub fn api_base_url(&self) -> String {
        let host = if self.api_address.is_empty() {
            "api.telegram.org"
        } else {
            self.api_address.as_str()
        };
        format!("https://{}/bot{}/", host, self.token)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ZabbixConfig {
    pub url: String,
    pub user: String,
    pub password: String,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct SpeedtestConfig {
    #[serde(default)]
    pub server_url: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CommandConfig {
    pub name: String,
    pub cmd: String,
    pub desc: String,
    #[serde(default)]
    pub sudo_check: bool,
    /// Set at runtime by sudo_check; not from TOML.
    #[serde(skip)]
    pub unavailable: bool,
}

mod defaults {
    use super::IpWhitelistConfig;
    pub fn webhook_path() -> String {
        "/webhook".to_string()
    }
    pub fn bind() -> String {
        "127.0.0.1:8080".to_string()
    }
    pub fn exec_timeout() -> u64 {
        30
    }
    pub fn request_timeout() -> u64 {
        10
    }
    pub fn request_retries() -> u32 {
        3
    }
    pub fn ip_whitelist() -> IpWhitelistConfig {
        IpWhitelistConfig::Named("telegram".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"
[bot]
mode = "polling"
bind = "127.0.0.1:8080"

[telegram]
token = "12345:TOKEN"

[admins]
"115237453" = "Vladislav"

[zabbix]
url = "https://monit.example.com/"
user = "zabbix_api"
password = "secret"

[speedtest]
server_url = ""
"#;

    #[test]
    fn test_parse_minimal() {
        let cfg: Config = toml::from_str(MINIMAL).unwrap();
        assert_eq!(cfg.bot.mode, BotMode::Polling);
        assert_eq!(cfg.telegram.token, "12345:TOKEN");
        assert!(cfg.admins.contains_key("115237453"));
        assert_eq!(cfg.super_admin_id(), Some(115237453i64));
    }

    #[test]
    fn test_super_admin_first() {
        let toml = r#"
[bot]
mode = "polling"
bind = "127.0.0.1:8080"
[telegram]
token = "t"
[admins]
"111" = "First"
"222" = "Second"
[zabbix]
url = "https://z/"
user = "u"
password = "p"
[speedtest]
server_url = ""
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        assert_eq!(cfg.super_admin_id(), Some(111i64));
    }

    #[test]
    fn test_whitelist_disabled() {
        let toml = MINIMAL.replace(
            "[telegram]",
            "webhook_ip_whitelist = \"disabled\"\n[telegram]",
        );
        let cfg: Config = toml::from_str(&toml).unwrap();
        assert!(matches!(
            cfg.bot.webhook_ip_whitelist,
            IpWhitelistConfig::Named(ref s) if s == "disabled"
        ));
    }

    #[test]
    fn test_whitelist_custom_cidrs() {
        let toml = MINIMAL.replace(
            "[telegram]",
            "webhook_ip_whitelist = [\"10.0.0.0/8\", \"192.168.1.0/24\"]\n[telegram]",
        );
        let cfg: Config = toml::from_str(&toml).unwrap();
        assert!(matches!(
            cfg.bot.webhook_ip_whitelist,
            IpWhitelistConfig::Custom(ref v) if v.len() == 2
        ));
    }

    #[test]
    fn test_is_admin() {
        let cfg: Config = toml::from_str(MINIMAL).unwrap();
        assert!(cfg.is_admin(115237453));
        assert!(!cfg.is_admin(999999));
    }

    #[test]
    fn test_api_base_url_default() {
        let cfg: Config = toml::from_str(MINIMAL).unwrap();
        assert_eq!(
            cfg.telegram.api_base_url(),
            "https://api.telegram.org/bot12345:TOKEN/"
        );
    }

    #[test]
    fn test_api_base_url_custom() {
        let toml = MINIMAL.replace(
            "token = \"12345:TOKEN\"",
            "token = \"12345:TOKEN\"\napi_address = \"vpn.example.com\"",
        );
        let cfg: Config = toml::from_str(&toml).unwrap();
        assert_eq!(
            cfg.telegram.api_base_url(),
            "https://vpn.example.com/bot12345:TOKEN/"
        );
    }
}
