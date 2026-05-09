use indexmap::IndexMap;
use serde::Deserialize;

/// Per-admin entry. Supports both shorthand and full form:
///   "123" = "Name"
///   "123" = { name = "Name", notify_login = false }
#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
#[serde(untagged)]
pub enum AdminEntry {
    Simple(String),
    Full(AdminConfig),
}

#[allow(dead_code)]
#[derive(Debug, Deserialize, Clone)]
pub struct AdminConfig {
    pub name: String,
    #[serde(default = "default_notify_login")]
    pub notify_login: bool,
}

fn default_notify_login() -> bool {
    true
}

impl AdminEntry {
    #[allow(dead_code)]
    pub fn notify_login(&self) -> bool {
        match self {
            Self::Simple(_) => true,
            Self::Full(c) => c.notify_login,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub bot: BotConfig,
    pub telegram: TelegramConfig,
    /// IndexMap preserves TOML insertion order — first entry is super-admin.
    pub admins: IndexMap<String, AdminEntry>,
    pub zabbix: ZabbixConfig,
    #[serde(default)]
    pub speedtest: SpeedtestConfig,
    #[serde(default)]
    pub monitor: MonitorConfig,
    #[serde(default)]
    pub pam: PamConfig,
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
        if config.bot.webhook_path != "/" && !config.bot.webhook_path.starts_with('/') {
            return Err(crate::error::BotError::Config(
                <toml::de::Error as serde::de::Error>::custom(
                    "bot.webhook_path must start with '/'",
                ),
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

    #[allow(dead_code)]
    /// All admin IDs that have login notifications enabled (notify_login = true, which is the default).
    pub fn notify_login_admin_ids(&self) -> Vec<i64> {
        self.admins
            .iter()
            .filter(|(_, entry)| entry.notify_login())
            .filter_map(|(k, _)| k.parse().ok())
            .collect()
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
    /// URL вебхука для автоматической регистрации при старте.
    /// Если None — управляй вебхуком вручную.
    pub webhook_address: Option<String>,
    #[serde(default = "defaults::always_set_webhook")]
    pub always_set_webhook: bool,
    #[serde(default = "defaults::notify_on_webhook_error")]
    pub notify_on_webhook_error: bool,
    #[serde(default = "defaults::language")]
    pub language: String,
    /// Drop commands if the same chat_id sends faster than this interval (0 = disabled).
    #[serde(default)]
    pub command_rate_limit_secs: u64,
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
    /// Single Telegram Bot API address (legacy; use api_addresses for fallback support).
    #[serde(default)]
    pub api_address: String,
    /// Ordered list of Telegram Bot API addresses tried in sequence on failure.
    #[serde(default)]
    pub api_addresses: Vec<String>,
    #[serde(default)]
    pub proxy: String,
    #[serde(default = "defaults::request_timeout")]
    pub request_timeout_secs: u64,
    #[serde(default = "defaults::request_retries")]
    pub request_retries: u32,
}

impl TelegramConfig {
    /// Returns ordered list of full base URLs (including /bot<token>/) to try.
    /// Prefers api_addresses (list) over api_address (single), falls back to api.telegram.org.
    pub fn api_base_urls(&self) -> Vec<String> {
        if !self.api_addresses.is_empty() {
            // api_addresses: full URLs expected (e.g. "https://mirror.example.com")
            return self
                .api_addresses
                .iter()
                .map(|addr| format!("{}/bot{}/", addr.trim_end_matches('/'), self.token))
                .collect();
        }

        // Legacy api_address: bare hostname or empty
        let host = if self.api_address.is_empty() {
            "api.telegram.org"
        } else {
            self.api_address.as_str()
        };
        vec![format!("https://{}/bot{}/", host, self.token)]
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
pub struct MonitorConfig {
    /// Enable background threshold monitoring.
    #[serde(default)]
    pub enabled: bool,
    /// Check interval in seconds.
    #[serde(default = "defaults::monitor_interval")]
    pub interval_secs: u64,
    /// CPU usage % threshold.
    #[serde(default = "defaults::cpu_warn")]
    pub cpu_warn: u8,
    /// RAM usage % threshold.
    #[serde(default = "defaults::ram_warn")]
    pub ram_warn: u8,
    /// Root disk usage % threshold.
    #[serde(default = "defaults::disk_warn")]
    pub disk_warn: u8,
    /// Seconds between repeat alerts while threshold remains breached.
    #[serde(default = "defaults::remind_secs")]
    pub remind_secs: u64,
}

impl Default for MonitorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: defaults::monitor_interval(),
            cpu_warn: defaults::cpu_warn(),
            ram_warn: defaults::ram_warn(),
            disk_warn: defaults::disk_warn(),
            remind_secs: defaults::remind_secs(),
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
#[allow(dead_code)]
pub struct PamConfig {
    /// PAM integration enabled (notifications and/or 2FA).
    #[serde(default)]
    pub enabled: bool,
    /// Send notification when a session opens.
    #[serde(default = "defaults::pam_notify_login")]
    pub notify_login: bool,
    /// Block login until super-admin approves in Telegram.
    #[serde(default)]
    pub two_factor_enabled: bool,
    /// Seconds before 2FA request times out.
    #[serde(default = "defaults::pam_timeout")]
    pub two_factor_timeout_secs: u64,
    /// Bot command name to block the remote IP (e.g. "ban-cs"). Empty = no button.
    #[serde(default)]
    pub block_ip_cmd: String,
    /// Minimum seconds between 2FA requests for the same user (0 = disabled).
    #[serde(default = "defaults::two_factor_rate_limit_secs")]
    pub two_factor_rate_limit_secs: u64,
}

impl Default for PamConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            notify_login: defaults::pam_notify_login(),
            two_factor_enabled: false,
            two_factor_timeout_secs: defaults::pam_timeout(),
            block_ip_cmd: String::new(),
            two_factor_rate_limit_secs: defaults::two_factor_rate_limit_secs(),
        }
    }
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
    pub fn always_set_webhook() -> bool {
        false
    }
    pub fn notify_on_webhook_error() -> bool {
        true
    }
    pub fn monitor_interval() -> u64 {
        60
    }
    pub fn cpu_warn() -> u8 {
        85
    }
    pub fn ram_warn() -> u8 {
        90
    }
    pub fn disk_warn() -> u8 {
        85
    }
    pub fn remind_secs() -> u64 {
        1800
    }
    pub fn pam_notify_login() -> bool {
        true
    }
    pub fn pam_timeout() -> u64 {
        60
    }
    pub fn two_factor_rate_limit_secs() -> u64 {
        30
    }
    pub fn language() -> String {
        "auto".to_string()
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
    fn test_notify_login_default_true() {
        let cfg: Config = toml::from_str(MINIMAL).unwrap();
        assert_eq!(cfg.notify_login_admin_ids(), vec![115237453i64]);
    }

    #[test]
    fn test_notify_login_explicit_false() {
        let toml = r#"
[bot]
mode = "polling"
bind = "127.0.0.1:8080"
[telegram]
token = "t"
[admins]
"111" = { name = "Super", notify_login = true }
"222" = { name = "Silent", notify_login = false }
"333" = { name = "Default" }
[zabbix]
url = "https://z/"
user = "u"
password = "p"
[speedtest]
server_url = ""
"#;
        let cfg: Config = toml::from_str(toml).unwrap();
        let ids = cfg.notify_login_admin_ids();
        assert!(ids.contains(&111));
        assert!(!ids.contains(&222));
        assert!(ids.contains(&333));
    }

    #[test]
    fn test_api_base_urls_default() {
        let cfg: Config = toml::from_str(MINIMAL).unwrap();
        assert_eq!(
            cfg.telegram.api_base_urls(),
            vec!["https://api.telegram.org/bot12345:TOKEN/"]
        );
    }

    #[test]
    fn test_api_base_urls_custom() {
        let toml = MINIMAL.replace(
            "token = \"12345:TOKEN\"",
            "token = \"12345:TOKEN\"\napi_address = \"vpn.example.com\"",
        );
        let cfg: Config = toml::from_str(&toml).unwrap();
        assert_eq!(
            cfg.telegram.api_base_urls(),
            vec!["https://vpn.example.com/bot12345:TOKEN/"]
        );
    }

    #[test]
    fn test_api_base_urls_multi() {
        let toml = MINIMAL.replace(
            "token = \"12345:TOKEN\"",
            "token = \"12345:TOKEN\"\napi_addresses = [\"https://mirror1.example.com\", \"https://api.telegram.org\"]",
        );
        let cfg: Config = toml::from_str(&toml).unwrap();
        let urls = cfg.telegram.api_base_urls();
        assert_eq!(urls.len(), 2);
        assert_eq!(urls[0], "https://mirror1.example.com/bot12345:TOKEN/");
        assert_eq!(urls[1], "https://api.telegram.org/bot12345:TOKEN/");
    }

    #[test]
    fn test_monitor_config_defaults() {
        let cfg: Config = toml::from_str(MINIMAL).unwrap();
        assert!(!cfg.monitor.enabled);
        assert_eq!(cfg.monitor.interval_secs, 60);
        assert_eq!(cfg.monitor.cpu_warn, 85);
        assert_eq!(cfg.monitor.ram_warn, 90);
        assert_eq!(cfg.monitor.disk_warn, 85);
        assert_eq!(cfg.monitor.remind_secs, 1800);
    }

    #[test]
    fn test_monitor_config_explicit() {
        let toml = MINIMAL.to_string() + "\n[monitor]\nenabled = true\ncpu_warn = 70\n";
        let cfg: Config = toml::from_str(&toml).unwrap();
        assert!(cfg.monitor.enabled);
        assert_eq!(cfg.monitor.cpu_warn, 70);
        assert_eq!(cfg.monitor.ram_warn, 90); // still default
    }

    #[test]
    fn test_pam_config_defaults() {
        let cfg: Config = toml::from_str(MINIMAL).unwrap();
        assert!(!cfg.pam.enabled);
        assert!(cfg.pam.notify_login);
        assert!(!cfg.pam.two_factor_enabled);
        assert_eq!(cfg.pam.two_factor_timeout_secs, 60);
        assert!(cfg.pam.block_ip_cmd.is_empty());
        assert_eq!(cfg.pam.two_factor_rate_limit_secs, 30);
    }

    #[test]
    fn test_pam_config_explicit() {
        let toml = MINIMAL.to_string() + "\n[pam]\nenabled = true\nblock_ip_cmd = \"ban-cs\"\n";
        let cfg: Config = toml::from_str(&toml).unwrap();
        assert!(cfg.pam.enabled);
        assert_eq!(cfg.pam.block_ip_cmd, "ban-cs");
        assert_eq!(cfg.pam.two_factor_timeout_secs, 60); // default preserved
    }

    #[test]
    fn test_is_super_admin() {
        let cfg: Config = toml::from_str(MINIMAL).unwrap();
        assert!(cfg.is_super_admin(115237453));
        assert!(!cfg.is_super_admin(999999));
    }

    #[test]
    fn test_load_file_not_found() {
        let result = Config::load("/nonexistent/path/config.toml");
        assert!(result.is_err());
    }
}
