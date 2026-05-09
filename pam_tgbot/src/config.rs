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
    /// Minimum seconds between 2FA requests for the same user (0 = disabled).
    #[serde(default = "default_2fa_rate_limit")]
    pub two_factor_rate_limit_secs: u64,
    /// Users whose login/2FA notifications are suppressed (e.g. service accounts).
    #[serde(default = "default_exclude_users")]
    pub notify_exclude_users: Vec<String>,
}

impl Default for PamCfg {
    fn default() -> Self {
        Self {
            notify_login: default_true(),
            two_factor_enabled: false,
            two_factor_timeout_secs: default_timeout(),
            block_ip_cmd: String::new(),
            language: default_language(),
            two_factor_rate_limit_secs: 30,
            notify_exclude_users: default_exclude_users(),
        }
    }
}

fn default_true() -> bool {
    true
}
fn default_timeout() -> u64 {
    60
}
fn default_language() -> String {
    "auto".to_string()
}
fn default_2fa_rate_limit() -> u64 {
    30
}
fn default_exclude_users() -> Vec<String> {
    vec!["gitlab".to_string()]
}

pub struct LoadedCfg {
    pub tg: TelegramCfg,
    pub pam: PamCfg,
    /// First admin in [admins] — receives 2FA approval requests.
    pub super_admin_id: i64,
    /// All admins with notify_login = true (default). Receive login notifications.
    pub notify_admin_ids: Vec<i64>,
}

/// Load only the sections pam_tgbot needs.
/// Returns None on any parse error or missing super-admin.
pub fn load(path: &str) -> Option<LoadedCfg> {
    let content = std::fs::read_to_string(path).ok()?;
    load_from_str(&content)
}

fn load_from_str(content: &str) -> Option<LoadedCfg> {
    let raw: toml::Value = toml::from_str(content).ok()?;

    let tg: TelegramCfg = raw
        .get("telegram")
        .and_then(|v| v.clone().try_into().ok())?;

    let pam: PamCfg = raw
        .get("pam")
        .and_then(|v| v.clone().try_into().ok())
        .unwrap_or_default();

    // toml 0.8 preserves insertion order — first key = super-admin
    let admins_table = raw
        .get("admins")
        .and_then(|v| v.as_table())?;

    let super_admin_id = admins_table
        .iter()
        .next()
        .and_then(|(k, _)| k.parse::<i64>().ok())?;

    // Collect admins where notify_login is true (default when absent or old string format).
    let notify_admin_ids: Vec<i64> = admins_table
        .iter()
        .filter_map(|(k, v)| {
            let id = k.parse::<i64>().ok()?;
            let notify = match v {
                toml::Value::String(_) => true,
                toml::Value::Table(t) => t
                    .get("notify_login")
                    .and_then(|b| b.as_bool())
                    .unwrap_or(true),
                _ => true,
            };
            if notify { Some(id) } else { None }
        })
        .collect();

    Some(LoadedCfg {
        tg,
        pam,
        super_admin_id,
        notify_admin_ids,
    })
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
        let cfg = load_from_str(SAMPLE).unwrap();
        assert_eq!(cfg.super_admin_id, 12345);
    }

    #[test]
    fn test_notify_admin_ids_old_format_defaults_true() {
        let cfg = load_from_str(SAMPLE).unwrap();
        assert_eq!(cfg.notify_admin_ids, vec![12345i64]);
    }

    #[test]
    fn test_notify_admin_ids_new_format() {
        // Keys must be in lexicographic order so BTreeMap (no preserve_order) puts
        // "12345" first — matching what a real config with this super-admin would do.
        let toml_str = SAMPLE.replace(
            "\"12345\" = \"Admin\"",
            "\"12345\" = { name = \"Super\", notify_login = true }\n\"23456\" = { name = \"Silent\", notify_login = false }\n\"34567\" = { name = \"Default\" }",
        );
        let cfg = load_from_str(&toml_str).unwrap();
        assert_eq!(cfg.super_admin_id, 12345);
        assert!(cfg.notify_admin_ids.contains(&12345));
        assert!(!cfg.notify_admin_ids.contains(&23456));
        assert!(cfg.notify_admin_ids.contains(&34567));
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
        assert_eq!(cfg.two_factor_rate_limit_secs, 30);
        assert_eq!(cfg.notify_exclude_users, vec!["gitlab"]);
    }

    #[test]
    fn test_notify_exclude_users_explicit_empty() {
        let toml_str = SAMPLE.to_string() + "\n[pam]\nnotify_exclude_users = []\n";
        let raw: toml::Value = toml::from_str(&toml_str).unwrap();
        let pam: PamCfg = raw["pam"].clone().try_into().unwrap();
        assert!(
            pam.notify_exclude_users.is_empty(),
            "explicit [] must override the default [\"gitlab\"]"
        );
    }

    #[test]
    fn test_notify_exclude_users_custom_list() {
        let toml_str =
            SAMPLE.to_string() + "\n[pam]\nnotify_exclude_users = [\"ci-runner\", \"deploy\"]\n";
        let raw: toml::Value = toml::from_str(&toml_str).unwrap();
        let pam: PamCfg = raw["pam"].clone().try_into().unwrap();
        assert_eq!(pam.notify_exclude_users, vec!["ci-runner", "deploy"]);
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
        assert_eq!(tg.api_base(), "https://local.api.example.com/bot999:TEST/");
    }
}
