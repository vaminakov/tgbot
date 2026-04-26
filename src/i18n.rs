use std::env;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Ru,
    En,
}

impl Lang {
    /// Resolve from config value: "ru" → Ru, "en" → En, anything else → detect().
    pub fn from_config(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "ru" => Self::Ru,
            "en" => Self::En,
            _ => Self::detect(),
        }
    }

    /// Detect from LC_ALL / LANG env vars. Falls back to En.
    pub fn detect() -> Self {
        for var in ["LC_ALL", "LANG"] {
            if let Ok(val) = env::var(var) {
                let lo = val.to_ascii_lowercase();
                if lo.starts_with("ru") {
                    return Self::Ru;
                }
                if lo.starts_with("en") {
                    return Self::En;
                }
            }
        }
        Self::En
    }

    // ── sender ────────────────────────────────────────────────────────────

    pub fn error_msg(self, err: impl std::fmt::Display) -> String {
        match self {
            Self::Ru => format!("Ошибка: {}", err),
            Self::En => format!("Error: {}", err),
        }
    }

    // ── auth / dispatch ───────────────────────────────────────────────────

    pub fn unauthorized_reply(self, name: &str) -> String {
        match self {
            Self::Ru => format!("{}, не пиши мне больше!", name),
            Self::En => format!("{}, don't write to me again!", name),
        }
    }

    pub fn sudo_super_admin_only(self) -> &'static str {
        match self {
            Self::Ru => "Команда sudo доступна только super-admin.",
            Self::En => "Sudo commands are only available to super-admin.",
        }
    }

    pub fn reboot_msg(self) -> &'static str {
        match self {
            Self::Ru => "🔄 Перезагрузка...",
            Self::En => "🔄 Rebooting...",
        }
    }

    pub fn executed(self) -> &'static str {
        match self {
            Self::Ru => "✅ Выполнено.",
            Self::En => "✅ Done.",
        }
    }

    // ── /top ─────────────────────────────────────────────────────────────

    pub fn top_cpu_header(self) -> &'static str {
        match self {
            Self::Ru => "⚡ Топ CPU:",
            Self::En => "⚡ Top CPU:",
        }
    }

    pub fn top_cpu_empty(self) -> &'static str {
        match self {
            Self::Ru => "  (нет активных процессов)",
            Self::En => "  (no active processes)",
        }
    }

    pub fn top_ram_header(self) -> &'static str {
        match self {
            Self::Ru => "💾 Топ RAM:",
            Self::En => "💾 Top RAM:",
        }
    }

    pub fn top_ram_empty(self) -> &'static str {
        match self {
            Self::Ru => "  (нет данных)",
            Self::En => "  (no data)",
        }
    }

    // ── PAM callbacks ─────────────────────────────────────────────────────

    pub fn pam_invalid_2fa_id(self) -> &'static str {
        match self {
            Self::Ru => "Некорректный ID запроса 2FA.",
            Self::En => "Invalid 2FA request ID.",
        }
    }

    pub fn pam_approved(self) -> &'static str {
        match self {
            Self::Ru => "✅ Вход одобрен.",
            Self::En => "✅ Login approved.",
        }
    }

    pub fn pam_denied(self) -> &'static str {
        match self {
            Self::Ru => "🚫 Вход отклонён.",
            Self::En => "🚫 Login denied.",
        }
    }

    pub fn pam_ipc_error(self, e: impl std::fmt::Display) -> String {
        match self {
            Self::Ru => format!("Ошибка IPC: {}", e),
            Self::En => format!("IPC error: {}", e),
        }
    }

    pub fn pam_invalid_session_id(self) -> &'static str {
        match self {
            Self::Ru => "Некорректный ID сессии.",
            Self::En => "Invalid session ID.",
        }
    }

    pub fn pam_session_terminated(self) -> &'static str {
        match self {
            Self::Ru => "🔌 Сессия завершена.",
            Self::En => "🔌 Session terminated.",
        }
    }

    // ── /ping ─────────────────────────────────────────────────────────────

    pub fn ping_usage(self) -> &'static str {
        match self {
            Self::Ru => "Использование: /ping <хост или IP>",
            Self::En => "Usage: /ping <host or IP>",
        }
    }

    // ── /zbx_graph ────────────────────────────────────────────────────────

    pub fn zabbix_not_configured(self) -> &'static str {
        match self {
            Self::Ru => "Zabbix не настроен.",
            Self::En => "Zabbix not configured.",
        }
    }

    pub fn zabbix_usage(self) -> &'static str {
        match self {
            Self::Ru => "Использование: /zbx_graph <itemid> <period> [name]",
            Self::En => "Usage: /zbx_graph <itemid> <period> [name]",
        }
    }

    pub fn invalid_period(self, input: &str) -> String {
        match self {
            Self::Ru => format!(
                "Неверный формат периода '{}'. Примеры: 1h, 30m, 7d, 2w, 86400s",
                input
            ),
            Self::En => format!(
                "Invalid period format '{}'. Examples: 1h, 30m, 7d, 2w, 86400s",
                input
            ),
        }
    }

    // ── /help (commands.rs) ───────────────────────────────────────────────

    pub fn help_header(self) -> &'static str {
        match self {
            Self::Ru => "Доступные команды:",
            Self::En => "Available commands:",
        }
    }

    pub fn help_status(self) -> &'static str {
        match self {
            Self::Ru => "/status — состояние сервера",
            Self::En => "/status — server status",
        }
    }

    pub fn help_top(self) -> &'static str {
        match self {
            Self::Ru => "/top — топ процессов по CPU и памяти",
            Self::En => "/top — top processes by CPU and RAM",
        }
    }

    pub fn help_reboot(self) -> &'static str {
        match self {
            Self::Ru => "/reboot — немедленная перезагрузка сервера",
            Self::En => "/reboot — reboot the server immediately",
        }
    }

    pub fn help_speedtest(self) -> &'static str {
        match self {
            Self::Ru => "/speedtest — замер скорости канала",
            Self::En => "/speedtest — internet speed test",
        }
    }

    pub fn help_whois(self) -> &'static str {
        match self {
            Self::Ru => "/whois <IP> — информация об IP (RDAP)",
            Self::En => "/whois <IP> — IP address info (RDAP)",
        }
    }

    pub fn help_ping(self) -> &'static str {
        match self {
            Self::Ru => "/ping <хост> — проверить доступность хоста",
            Self::En => "/ping <host> — check host connectivity",
        }
    }

    pub fn help_zbx_graph(self) -> &'static str {
        match self {
            Self::Ru => "/zbx_graph <itemid> <period> [name] — график Zabbix",
            Self::En => "/zbx_graph <itemid> <period> [name] — Zabbix graph",
        }
    }

    // ── monitor/mod.rs ────────────────────────────────────────────────────

    pub fn monitor_cpu_alert(self, pct: u8, thresh: u8) -> String {
        match self {
            Self::Ru => format!("⚠️ CPU: {}% (порог {}%)", pct, thresh),
            Self::En => format!("⚠️ CPU: {}% (threshold {}%)", pct, thresh),
        }
    }

    pub fn monitor_cpu_recover(self, pct: u8) -> String {
        match self {
            Self::Ru => format!("✅ CPU: {}% — норма восстановлена", pct),
            Self::En => format!("✅ CPU: {}% — back to normal", pct),
        }
    }

    pub fn monitor_ram_alert(self, pct: u8, thresh: u8) -> String {
        match self {
            Self::Ru => format!("⚠️ RAM: {}% (порог {}%)", pct, thresh),
            Self::En => format!("⚠️ RAM: {}% (threshold {}%)", pct, thresh),
        }
    }

    pub fn monitor_ram_recover(self, pct: u8) -> String {
        match self {
            Self::Ru => format!("✅ RAM: {}% — норма восстановлена", pct),
            Self::En => format!("✅ RAM: {}% — back to normal", pct),
        }
    }

    pub fn monitor_disk_alert(self, pct: u8, thresh: u8) -> String {
        match self {
            Self::Ru => format!("⚠️ Диск /: {}% (порог {}%)", pct, thresh),
            Self::En => format!("⚠️ Disk /: {}% (threshold {}%)", pct, thresh),
        }
    }

    pub fn monitor_disk_recover(self, pct: u8) -> String {
        match self {
            Self::Ru => format!("✅ Диск /: {}% — норма восстановлена", pct),
            Self::En => format!("✅ Disk /: {}% — back to normal", pct),
        }
    }

    // ── system/mod.rs ─────────────────────────────────────────────────────

    pub fn uptime_days(self) -> &'static str {
        match self { Self::Ru => "д", Self::En => "d" }
    }

    pub fn uptime_hours(self) -> &'static str {
        match self { Self::Ru => "ч", Self::En => "h" }
    }

    pub fn uptime_mins(self) -> &'static str {
        match self { Self::Ru => "м", Self::En => "m" }
    }

    pub fn load_suffix(self) -> &'static str {
        match self {
            Self::Ru => "(1/5/15 мин)",
            Self::En => "(1/5/15 min)",
        }
    }

    pub fn ram_unit(self) -> &'static str {
        match self { Self::Ru => "М", Self::En => "M" }
    }

    pub fn disk_label(self) -> &'static str {
        match self { Self::Ru => "Диск", Self::En => "Disk" }
    }

    pub fn disk_unit(self) -> &'static str {
        match self { Self::Ru => "Г", Self::En => "G" }
    }

    pub fn disk_free(self) -> &'static str {
        match self { Self::Ru => "свободно", Self::En => "free" }
    }

    pub fn traffic_label(self) -> &'static str {
        match self { Self::Ru => "Трафик", Self::En => "Traffic" }
    }

    pub fn traffic_unit(self) -> &'static str {
        match self { Self::Ru => "ГБ", Self::En => "GB" }
    }

    pub fn pkg_not_found(self) -> &'static str {
        match self {
            Self::Ru => "📦 Менеджер пакетов не определён",
            Self::En => "📦 Package manager not detected",
        }
    }

    pub fn pkg_no_updates(self) -> &'static str {
        match self {
            Self::Ru => "📦 Обновлений нет",
            Self::En => "📦 No updates available",
        }
    }

    pub fn pkg_updates(self, n: usize) -> String {
        match self {
            Self::Ru => format!("📦 Обновлений: {}", n),
            Self::En => format!("📦 Updates available: {}", n),
        }
    }

    pub fn pkg_error(self, e: impl std::fmt::Display) -> String {
        match self {
            Self::Ru => format!("📦 Ошибка подсчёта обновлений: {}", e),
            Self::En => format!("📦 Update check error: {}", e),
        }
    }

    pub fn pkg_timeout(self) -> &'static str {
        match self {
            Self::Ru => "📦 Ошибка подсчёта обновлений: таймаут",
            Self::En => "📦 Update check timed out",
        }
    }

    pub fn services_ok(self) -> &'static str {
        match self {
            Self::Ru => "✅ Все службы в норме",
            Self::En => "✅ All services running",
        }
    }

    pub fn services_failed(self, n: usize) -> String {
        match self {
            Self::Ru => format!("⚠️ Failed служб: {}", n),
            Self::En => format!("⚠️ Failed services: {}", n),
        }
    }

    // ── whois/mod.rs ──────────────────────────────────────────────────────

    pub fn whois_usage(self) -> &'static str {
        match self {
            Self::Ru => "Использование: /whois <IP>",
            Self::En => "Usage: /whois <IP>",
        }
    }

    pub fn whois_not_ip(self, input: &str) -> String {
        match self {
            Self::Ru => format!(
                "'{}' не является IP-адресом. Использование: /whois <IP>",
                input
            ),
            Self::En => format!("'{}' is not an IP address. Usage: /whois <IP>", input),
        }
    }

    pub fn whois_private(self) -> &'static str {
        match self {
            Self::Ru => "приватный/служебный адрес",
            Self::En => "private/reserved address",
        }
    }

    pub fn whois_network(self) -> &'static str {
        match self { Self::Ru => "Сеть", Self::En => "Network" }
    }

    pub fn whois_country(self) -> &'static str {
        match self { Self::Ru => "Страна", Self::En => "Country" }
    }

    pub fn whois_org(self) -> &'static str {
        match self { Self::Ru => "Организация", Self::En => "Organisation" }
    }

    pub fn whois_city(self) -> &'static str {
        match self { Self::Ru => "Город", Self::En => "City" }
    }

    pub fn whois_contact(self) -> &'static str {
        match self { Self::Ru => "Контакт", Self::En => "Contact" }
    }

    pub fn whois_phone(self) -> &'static str {
        match self { Self::Ru => "Телефон", Self::En => "Phone" }
    }

    pub fn whois_abuse(self) -> &'static str {
        match self { Self::Ru => "Абьюз", Self::En => "Abuse" }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_config_explicit() {
        assert_eq!(Lang::from_config("ru"), Lang::Ru);
        assert_eq!(Lang::from_config("en"), Lang::En);
        assert_eq!(Lang::from_config("RU"), Lang::Ru);
        assert_eq!(Lang::from_config("EN"), Lang::En);
    }

    #[test]
    fn from_config_auto_returns_valid_lang() {
        // "auto" calls detect() — may be either variant depending on test env
        let lang = Lang::from_config("auto");
        assert!(lang == Lang::Ru || lang == Lang::En);
    }

    #[test]
    fn translations_non_empty() {
        for lang in [Lang::Ru, Lang::En] {
            assert!(!lang.ping_usage().is_empty());
            assert!(!lang.help_header().is_empty());
            assert!(!lang.monitor_cpu_alert(90, 85).is_empty());
            assert!(!lang.whois_usage().is_empty());
            assert!(!lang.pkg_no_updates().is_empty());
            assert!(!lang.error_msg("test error").is_empty());
        }
    }

    #[test]
    fn translations_differ_between_langs() {
        assert_ne!(Lang::Ru.ping_usage(), Lang::En.ping_usage());
        assert_ne!(Lang::Ru.help_header(), Lang::En.help_header());
        assert_ne!(Lang::Ru.monitor_cpu_alert(90, 85), Lang::En.monitor_cpu_alert(90, 85));
        assert_ne!(Lang::Ru.whois_country(), Lang::En.whois_country());
        assert_ne!(Lang::Ru.uptime_days(), Lang::En.uptime_days());
        assert_ne!(Lang::Ru.services_ok(), Lang::En.services_ok());
    }

    #[test]
    fn parameterized_translations_contain_value() {
        let msg = Lang::Ru.monitor_cpu_alert(90, 85);
        assert!(msg.contains("90") && msg.contains("85"));
        let msg = Lang::En.pkg_updates(5);
        assert!(msg.contains("5"));
        let msg = Lang::Ru.whois_not_ip("hello");
        assert!(msg.contains("hello"));
    }
}
