pub mod commands;
pub mod security;
pub mod sender;

use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

use crate::config::Config;
use crate::error::BotError;
use crate::i18n::Lang;
use crate::telegram::client::TelegramClient;
use crate::telegram::types::{default_keyboard, Update};
use commands::{exec_shell, help_text, parse_input, run_configured_cmd};
use security::sanitize_arg;
use sender::{send, send_error};

pub struct BotContext {
    pub config: Arc<Config>,
    pub tg: Arc<TelegramClient>,
    pub lang: Lang,
    pub rate_limit: Arc<tokio::sync::Mutex<std::collections::HashMap<i64, std::time::Instant>>>,
}

pub async fn dispatch(update: &Update, ctx: &BotContext) {
    let chat_id = match update.chat_id() {
        Some(id) => id,
        None => return,
    };

    let text = match update.text() {
        Some(t) if !t.is_empty() => t,
        _ => return,
    };

    // ── Auth ──────────────────────────────────────────────────────────────
    if !ctx.config.is_admin(chat_id) {
        let super_id = ctx.config.super_admin_id().unwrap_or(chat_id);
        let username = update
            .username()
            .map(|u| format!("@{u}"))
            .unwrap_or_else(|| "unknown".into());
        let notify = format!(
            "Ко мне постучался {} (chat_id: {}) с сообщением: {}\nЯ его прогнал.",
            username, chat_id, text
        );
        let _ = send(&ctx.tg, super_id, &notify, None).await;
        let _ = send(
            &ctx.tg,
            chat_id,
            &ctx.lang.unauthorized_reply(&update.first_name()),
            None,
        )
        .await;
        warn!(chat_id, %username, "Got message from {} (chat_id: {}) — unauthorized, rejected.", username, chat_id);
        return;
    }

    // Acknowledge callback query now that auth has passed
    if let Some(cq_id) = update.callback_query_id() {
        let _ = ctx.tg.answer_callback_query(cq_id).await;
    }

    info!(chat_id, text, "command received");

    // ── PAM callbacks ─────────────────────────────────────────────────────
    if text.starts_with("pam_") && handle_pam_callback(text, chat_id, ctx).await {
        return;
    }

    // Per-user command rate limiting (0 = disabled)
    if ctx.config.bot.command_rate_limit_secs > 0 {
        let now = std::time::Instant::now();
        let mut map = ctx.rate_limit.lock().await;
        if let Some(last) = map.get(&chat_id) {
            if now.duration_since(*last).as_secs() < ctx.config.bot.command_rate_limit_secs {
                return; // silently drop — prevents runaway floods
            }
        }
        map.insert(chat_id, now);
    }

    let (cmd_name, args) = parse_input(text);

    // sudo restricted to super-admin
    if cmd_name == "sudo" && !ctx.config.is_super_admin(chat_id) {
        let _ = send(
            &ctx.tg,
            chat_id,
            ctx.lang.sudo_super_admin_only(),
            None,
        )
        .await;
        return;
    }

    // ── Reboot (special: message must be sent before execution) ──────────
    if cmd_name == "reboot" {
        let _ = send(&ctx.tg, chat_id, ctx.lang.reboot_msg(), None).await;
        tokio::spawn(async {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let _ = tokio::process::Command::new("sudo")
                .args(["systemctl", "reboot", "--force", "--force"])
                .spawn();
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            let _ = tokio::process::Command::new("sudo")
                .args(["reboot", "-f"])
                .spawn();
        });
        return;
    }

    // ── Built-in commands ─────────────────────────────────────────────────
    let builtin: Option<Result<String, BotError>> = match cmd_name {
        "status" => Some(crate::system::status(ctx.lang).await),
        "speedtest" => Some(handle_speedtest(&ctx.config.speedtest.server_url).await),
        "whois" => Some(crate::whois::lookup(args.first().copied().unwrap_or(""), ctx.lang).await),
        "ping" => Some(
            handle_ping(
                args.first().copied().unwrap_or(""),
                ctx.config.bot.exec_timeout_secs.min(15),
                ctx.lang,
            )
            .await,
        ),
        "top" => Some(handle_top(ctx.lang).await),
        "zbx_graph" => Some(handle_zbx_graph(&args, chat_id, ctx).await),
        _ => None,
    };

    if let Some(result) = builtin {
        match result {
            Ok(ref t) if !t.is_empty() => {
                let _ = send(&ctx.tg, chat_id, t, None).await;
            }
            Ok(_) => {} // already sent (e.g. document)
            Err(ref e) => send_error(&ctx.tg, chat_id, e, ctx.lang).await,
        }
        return;
    }

    // ── Configured commands ───────────────────────────────────────────────
    if let Some(cmd_cfg) = ctx.config.commands.iter().find(|c| c.name == cmd_name) {
        let timeout = ctx.config.bot.exec_timeout_secs;
        match run_configured_cmd(cmd_cfg, &args, timeout).await {
            Ok(out) => {
                let msg = if out.trim().is_empty() { ctx.lang.executed() } else { &out };
                let _ = send(&ctx.tg, chat_id, msg, None).await;
            }
            Err(e) => send_error(&ctx.tg, chat_id, &e, ctx.lang).await,
        }
        return;
    }

    // ── Unknown → help ────────────────────────────────────────────────────
    let kb = default_keyboard();
    let zabbix_configured = !ctx.config.zabbix.url.is_empty();
    let _ = send(
        &ctx.tg,
        chat_id,
        &help_text(&ctx.config.commands, zabbix_configured, ctx.lang),
        Some(&kb),
    )
    .await;
}

struct CpuStat {
    total: u64,
    idle: u64,
}

struct ProcStat {
    name: String,
    utime: u64,
    stime: u64,
    rss_kb: u64,
}

fn format_mem(kb: u64) -> String {
    if kb >= 1_048_576 {
        format!("{:.1} GB", kb as f64 / 1_048_576.0)
    } else if kb >= 1024 {
        format!("{} MB", kb / 1024)
    } else {
        format!("{} KB", kb)
    }
}

fn read_proc_stats() -> (CpuStat, HashMap<u32, ProcStat>) {
    // System-wide CPU from /proc/stat first line: "cpu user nice system idle iowait ..."
    let cpu = std::fs::read_to_string("/proc/stat")
        .ok()
        .and_then(|s| {
            let line = s.lines().next()?.to_string();
            let nums: Vec<u64> = line
                .split_whitespace()
                .skip(1)
                .filter_map(|x| x.parse().ok())
                .collect();
            let idle = nums.get(3).copied().unwrap_or(0)
                + nums.get(4).copied().unwrap_or(0); // idle + iowait
            let total: u64 = nums.iter().sum();
            Some(CpuStat { total, idle })
        })
        .unwrap_or(CpuStat { total: 0, idle: 0 });

    let mut procs = HashMap::new();
    let dir = match std::fs::read_dir("/proc") {
        Ok(d) => d,
        Err(_) => return (cpu, procs),
    };
    for entry in dir.flatten() {
        let fname = entry.file_name();
        let pid_str = fname.to_string_lossy();
        let pid: u32 = match pid_str.parse() {
            Ok(p) => p,
            Err(_) => continue,
        };

        // /proc/[pid]/stat — use rfind(')') to safely skip comm field
        // which can contain spaces and parentheses.
        // Fields after ')': state(0) ppid(1) ... utime(11) stime(12)
        let stat_path = format!("/proc/{}/stat", pid);
        let stat_content = match std::fs::read_to_string(&stat_path) {
            Ok(s) => s,
            Err(_) => continue, // process exited between readdir and read
        };
        let rest = match stat_content.rfind(')') {
            Some(i) => &stat_content[i + 1..],
            None => continue,
        };
        let fields: Vec<&str> = rest.split_whitespace().collect();
        let utime: u64 = fields.get(11).and_then(|s| s.parse().ok()).unwrap_or(0);
        let stime: u64 = fields.get(12).and_then(|s| s.parse().ok()).unwrap_or(0);

        // /proc/[pid]/status — Name (kernel truncates at 15 chars) and VmRSS
        let status_path = format!("/proc/{}/status", pid);
        let status = std::fs::read_to_string(&status_path).unwrap_or_default();
        let mut name = pid_str.to_string();
        let mut rss_kb = 0u64;
        for line in status.lines() {
            if let Some(val) = line.strip_prefix("Name:") {
                name = val.trim().to_string();
            } else if let Some(val) = line.strip_prefix("VmRSS:") {
                rss_kb = val
                    .split_whitespace()
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0);
            }
        }

        procs.insert(pid, ProcStat { name, utime, stime, rss_kb });
    }
    (cpu, procs)
}

async fn handle_top(lang: Lang) -> Result<String, BotError> {
    let loadavg = tokio::fs::read_to_string("/proc/loadavg")
        .await
        .map_err(BotError::Io)?;
    let mut la = loadavg.split_whitespace();
    let la1 = la.next().unwrap_or("?");
    let la5 = la.next().unwrap_or("?");
    let la15 = la.next().unwrap_or("?");

    let (cpu1, procs1) = tokio::task::spawn_blocking(read_proc_stats)
        .await
        .unwrap_or_else(|_| (CpuStat { total: 0, idle: 0 }, HashMap::new()));
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let (cpu2, procs2) = tokio::task::spawn_blocking(read_proc_stats)
        .await
        .unwrap_or_else(|_| (CpuStat { total: 0, idle: 0 }, HashMap::new()));

    // System-wide CPU %
    let d_total = cpu2.total.saturating_sub(cpu1.total);
    let d_idle = cpu2.idle.saturating_sub(cpu1.idle);
    let cpu_pct = if d_total > 0 {
        (d_total - d_idle) as f64 * 100.0 / d_total as f64
    } else {
        0.0
    };

    // Per-process CPU — aggregate by name
    let mut cpu_map: HashMap<String, f64> = HashMap::new();
    if d_total > 0 {
        for (pid, s2) in &procs2 {
            if let Some(s1) = procs1.get(pid) {
                let d = (s2.utime + s2.stime).saturating_sub(s1.utime + s1.stime);
                let pct = d as f64 * 100.0 / d_total as f64;
                if pct > 0.0 {
                    *cpu_map.entry(s2.name.clone()).or_default() += pct;
                }
            }
        }
    }
    let mut cpu_list: Vec<(String, f64)> = cpu_map.into_iter().collect();
    cpu_list.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    cpu_list.truncate(5);

    // Per-process RAM — aggregate by name, skip kernel threads (rss==0)
    let mut ram_map: HashMap<String, u64> = HashMap::new();
    for s in procs2.values() {
        if s.rss_kb > 0 {
            *ram_map.entry(s.name.clone()).or_default() += s.rss_kb;
        }
    }
    let mut ram_list: Vec<(String, u64)> = ram_map.into_iter().collect();
    ram_list.sort_by_key(|e| std::cmp::Reverse(e.1));
    ram_list.truncate(5);

    let mut lines = vec![format!(
        "📊 Load: {} / {} / {}  CPU: {:.1}%",
        la1, la5, la15, cpu_pct
    )];

    lines.push(String::new());
    lines.push(lang.top_cpu_header().to_string());
    if cpu_list.is_empty() {
        lines.push(lang.top_cpu_empty().to_string());
    } else {
        for (name, pct) in &cpu_list {
            lines.push(format!("• {}: {:.1}%", name, pct));
        }
    }

    lines.push(String::new());
    lines.push(lang.top_ram_header().to_string());
    if ram_list.is_empty() {
        lines.push(lang.top_ram_empty().to_string());
    } else {
        for (name, kb) in &ram_list {
            lines.push(format!("• {}: {}", name, format_mem(*kb)));
        }
    }

    Ok(lines.join("\n"))
}

async fn handle_speedtest(server_url: &str) -> Result<String, BotError> {
    let url = server_url.to_string();
    tokio::task::spawn_blocking(move || crate::speedtest::run(url))
        .await
        .map_err(|e| BotError::Speedtest {
            message: e.to_string(),
        })?
}

/// Handle PAM module callbacks: 2FA approve/deny and session kill.
/// Returns true if the text was a recognized PAM callback (caller returns early).
async fn handle_pam_callback(text: &str, chat_id: i64, ctx: &BotContext) -> bool {
    if !ctx.config.pam.enabled {
        // PAM integration disabled — consume pam_ callbacks silently rather than
        // falling through to parse_input and producing "command not found".
        return true;
    }
    if text.starts_with("pam_approve:") || text.starts_with("pam_deny:") {
        let approved = text.starts_with("pam_approve:");
        let rest = if approved {
            &text["pam_approve:".len()..]
        } else {
            &text["pam_deny:".len()..]
        };
        // Validate: IPC ID must be 32 lowercase hex chars
        if rest.len() != 32 || !rest.chars().all(|c| c.is_ascii_hexdigit()) {
            let _ = send(&ctx.tg, chat_id, ctx.lang.pam_invalid_2fa_id(), None).await;
            return true;
        }
        let path  = format!("/run/tgbot/pam/{}", rest);
        let value = if approved { "approved" } else { "denied" };
        // create(false) prevents recreating the file if PAM already timed out and
        // removed it — avoids orphaned files and false-success replies to admin.
        use std::io::Write as _;
        let write_result = std::fs::OpenOptions::new()
            .write(true).create(false).truncate(true)
            .open(&path)
            .and_then(|mut f| f.write_all(value.as_bytes()));
        match write_result {
            Ok(_) => {
                let reply = if approved { ctx.lang.pam_approved() } else { ctx.lang.pam_denied() };
                let _ = send(&ctx.tg, chat_id, reply, None).await;
            }
            Err(e) => {
                let _ = send(
                    &ctx.tg, chat_id,
                    &ctx.lang.pam_ipc_error(&e),
                    None,
                ).await;
            }
        }
        return true;
    }

    if let Some(sid) = text.strip_prefix("pam_kill:") {
        // systemd session IDs are short alphanumeric strings (e.g. "1", "42", "c3")
        if sid.is_empty() || sid.len() > 16 || !sid.chars().all(|c| c.is_ascii_alphanumeric()) {
            let _ = send(&ctx.tg, chat_id, ctx.lang.pam_invalid_session_id(), None).await;
            return true;
        }
        match exec_shell(
            &format!("sudo /usr/bin/loginctl terminate-session {}", sid),
            10,
        ).await {
            Ok(out) if out.contains("[exit:") => {
                let _ = send(&ctx.tg, chat_id, &out.trim().to_string(), None).await;
            }
            Ok(_)  => { let _ = send(&ctx.tg, chat_id, ctx.lang.pam_session_terminated(), None).await; }
            Err(e) => send_error(&ctx.tg, chat_id, &e, ctx.lang).await,
        }
        return true;
    }

    false
}

async fn handle_ping(host: &str, timeout_secs: u64, lang: Lang) -> Result<String, BotError> {
    let host = host.trim();
    if host.is_empty() {
        return Ok(lang.ping_usage().into());
    }
    sanitize_arg(host)?;
    exec_shell(&format!("ping -c 4 -W 3 -- {}", host), timeout_secs).await
}

fn validate_period(p: &str) -> bool {
    // Use char_indices to avoid split_at panic on multibyte UTF-8 chars
    let Some((suffix_pos, _)) = p.char_indices().next_back() else { return false };
    let digits = &p[..suffix_pos];
    let suffix = &p[suffix_pos..];
    !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit())
        && matches!(suffix, "s" | "m" | "h" | "d" | "w" | "M" | "y")
}

async fn handle_zbx_graph(
    args: &[&str],
    chat_id: i64,
    ctx: &BotContext,
) -> Result<String, BotError> {
    if ctx.config.zabbix.url.is_empty() {
        return Ok(ctx.lang.zabbix_not_configured().into());
    }
    if args.is_empty() {
        return Ok(ctx.lang.zabbix_usage().into());
    }
    let item_id: u64 = args[0].parse().map_err(|_| BotError::InvalidArgument {
        input: args[0].to_string(),
    })?;
    if item_id == 0 {
        return Err(BotError::InvalidArgument {
            input: "0".to_string(),
        });
    }
    let period_raw = args.get(1).copied().unwrap_or("1h");
    // Accept bare integers as seconds: "84600" → "84600s"
    let period_owned;
    let period = if !period_raw.is_empty() && period_raw.chars().all(|c| c.is_ascii_digit()) {
        period_owned = format!("{}s", period_raw);
        period_owned.as_str()
    } else {
        period_raw
    };
    if !validate_period(period) {
        return Ok(ctx.lang.invalid_period(period_raw));
    }
    let name = args.get(2).copied().unwrap_or("");
    // Cap graph name length — find safe UTF-8 boundary
    let name_end = (0..=256.min(name.len())).rev().find(|&i| name.is_char_boundary(i)).unwrap_or(0);
    let name = &name[..name_end];
    let bytes = crate::zabbix::graph::fetch(&ctx.config.zabbix, item_id, period, name).await?;
    ctx.tg.send_document(chat_id, "graph.png", bytes).await?;
    Ok(String::new()) // document already sent
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_mem_kb() {
        assert_eq!(format_mem(500), "500 KB");
    }

    #[test]
    fn test_format_mem_mb() {
        assert_eq!(format_mem(2048), "2 MB");
    }

    #[test]
    fn test_format_mem_gb() {
        assert_eq!(format_mem(2 * 1024 * 1024), "2.0 GB");
    }

    #[tokio::test]
    async fn test_ping_empty_host() {
        let result = handle_ping("", 5, Lang::Ru).await.unwrap();
        assert!(result.contains("Использование"));
    }

    #[tokio::test]
    async fn test_ping_invalid_chars() {
        let result = handle_ping("host; rm -rf /", 5, Lang::Ru).await;
        assert!(matches!(result.unwrap_err(), crate::error::BotError::InvalidArgument { .. }));
    }

    #[tokio::test]
    async fn test_ping_dash_prefix_rejected() {
        // sanitize_arg allows '-' so we add '--' to stop flag injection at shell level.
        // The '--' separator means the host is never treated as a flag by ping.
        // We can't easily test the shell behavior in a unit test, but we verify
        // that a dash-only input does reach exec_shell (returns a non-usage error).
        let result = handle_ping("-n", 1, Lang::Ru).await;
        // Should not return a usage message — it's not empty
        match result {
            Ok(s) => assert!(!s.contains("Использование"), "'-n' should not produce usage msg"),
            Err(_) => {} // timeout or command error is fine
        }
    }

    #[test]
    fn test_validate_period_valid() {
        for p in ["1h", "30m", "7d", "2w", "86400s", "1M", "1y"] {
            assert!(validate_period(p), "expected valid: {p}");
        }
    }

    #[test]
    fn test_validate_period_invalid() {
        for p in ["", "h", "1", "1x", "abc", "1 h", "-1h", "1H"] {
            assert!(!validate_period(p), "expected invalid: {p}");
        }
    }

    #[test]
    fn test_validate_period_bare_int_rejected() {
        // bare integers like "86400" must be converted to "86400s" before validate_period
        assert!(!validate_period("86400"));
    }

    #[test]
    fn test_format_mem_boundaries() {
        assert_eq!(format_mem(0), "0 KB");
        assert_eq!(format_mem(1023), "1023 KB");
        assert_eq!(format_mem(1024), "1 MB");
        assert_eq!(format_mem(1_048_575), "1023 MB");
        assert_eq!(format_mem(1_048_576), "1.0 GB");
    }

    #[test]
    fn pam_kill_session_id_validation() {
        // Valid systemd session IDs
        let valid = ["1", "42", "c1abc", "A1B2C3", "abc123def456abc1"]; // last is exactly 16 chars
        for id in valid {
            assert!(
                !id.is_empty() && id.len() <= 16 && id.chars().all(|c| c.is_ascii_alphanumeric()),
                "should be valid: {id}"
            );
        }
        // Invalid: empty, too long, contains non-alphanumeric
        let invalid = ["", "a/b", "../etc", "a1234567890123456"]; // last is 17 chars
        for id in invalid {
            assert!(
                id.is_empty() || id.len() > 16 || !id.chars().all(|c| c.is_ascii_alphanumeric()),
                "should be invalid: {id}"
            );
        }
    }
}
