use serde::Deserialize;

pub const CONFIG_PATH: &str = "/etc/tgbot/config.toml";

#[derive(Debug, Deserialize, Clone)]
pub struct TelegramCfg {
    pub token: String,
    #[serde(default)]
    pub api_address: String,
    #[serde(default)]
    pub proxy: String,
}

impl TelegramCfg {
    pub fn api_base(&self) -> String {
        let host = if self.api_address.is_empty() {
            "api.telegram.org"
        } else {
            self.api_address.trim_end_matches('/')
        };
        format!("https://{}/bot{}/", host, self.token)
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct PamCfg {
    #[serde(default = "default_true")]
    pub notify_login: bool,
    #[serde(default)]
    pub two_factor_enabled: bool,
    #[serde(default = "default_timeout")]
    pub two_factor_timeout_secs: u64,
    /// Bot command name to call for blocking the remote IP (e.g. "ban-cs").
    /// Empty string = do not show block button.
    #[serde(default)]
    pub block_ip_cmd: String,
    /// Notification language: "ru", "en", or "auto" (detect from LC_ALL/LANG).
    #[serde(default = "default_language")]
    pub language: String,
}

impl Default for PamCfg {
    fn default() -> Self {
        Self {
            notify_login:            default_true(),
            two_factor_enabled:      false,
            two_factor_timeout_secs: default_timeout(),
            block_ip_cmd:            String::new(),
            language:                default_language(),
        }
    }
}

fn default_true()     -> bool   { true          }
fn default_timeout()  -> u64   { 60             }
fn default_language() -> String { "auto".to_string() }

pub struct LoadedCfg {
    pub tg:             TelegramCfg,
    pub pam:            PamCfg,
    pub super_admin_id: i64,
}

/// Load only the sections pam_tgbot needs.
/// Returns None on any parse error or missing super-admin.
pub fn load(path: &str) -> Option<LoadedCfg> {
    let content = std::fs::read_to_string(path).ok()?;
    let raw: toml::Value = toml::from_str(&content).ok()?;

    let tg: TelegramCfg = raw.get("telegram")
        .and_then(|v| v.clone().try_into().ok())?;

    let pam: PamCfg = raw.get("pam")
        .and_then(|v| v.clone().try_into().ok())
        .unwrap_or_default();

    // toml 0.8 preserves insertion order — first key = super-admin
    let super_admin_id = raw.get("admins")
        .and_then(|v| v.as_table())
        .and_then(|t| t.iter().next())
        .and_then(|(k, _)| k.parse::<i64>().ok())?;

    Some(LoadedCfg { tg, pam, super_admin_id })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
[bot]
mode = "polling"
bind = "127.0.0.1:8080"

[telegram]
token      = "999:TEST"
api_address = ""
proxy      = ""

[admins]
"12345" = "Admin"

[zabbix]
url      = "https://z.example.com/"
user     = "u"
password = "p"
"#;

    #[test]
    fn test_super_admin_first_entry() {
        let raw: toml::Value = toml::from_str(SAMPLE).unwrap();
        let id = raw["admins"].as_table().unwrap()
            .iter().next().unwrap().0.parse::<i64>().unwrap();
        assert_eq!(id, 12345);
    }

    #[test]
    fn test_pam_defaults_when_section_absent() {
        let cfg: PamCfg = toml::Value::Table(Default::default())
            .try_into()
            .unwrap_or_default();
        assert!(cfg.notify_login);
        assert!(!cfg.two_factor_enabled);
        assert_eq!(cfg.two_factor_timeout_secs, 60);
        assert!(cfg.block_ip_cmd.is_empty());
    }

    #[test]
    fn test_api_base_default_host() {
        let raw: toml::Value = toml::from_str(SAMPLE).unwrap();
        let tg: TelegramCfg = raw["telegram"].clone().try_into().unwrap();
        assert_eq!(tg.api_base(), "https://api.telegram.org/bot999:TEST/");
    }

    #[test]
    fn test_api_base_custom_host() {
        let toml_str = SAMPLE.replace(
            "api_address = \"\"",
            "api_address = \"local.api.example.com\"",
        );
        let raw: toml::Value = toml::from_str(&toml_str).unwrap();
        let tg: TelegramCfg = raw["telegram"].clone().try_into().unwrap();
        assert_eq!(
            tg.api_base(),
            "https://local.api.example.com/bot999:TEST/"
        );
    }
}
