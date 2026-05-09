use crate::config::CommandConfig;
use crate::error::BotError;
use crate::i18n::Lang;

use super::security::{expand_cmd, sanitize_arg};

/// Split user input into (command_name, args). Strips leading '/'.
pub fn parse_input(text: &str) -> (&str, Vec<&str>) {
    let text = text.trim_start_matches('/');
    let mut parts = text.splitn(2, ' ');
    let cmd = parts.next().unwrap_or("").trim();
    // Strip @botname suffix sent in group chats (e.g. "/status@mybotname")
    let cmd = cmd.split('@').next().unwrap_or(cmd);
    let rest = parts.next().unwrap_or("").trim();
    let args = if rest.is_empty() {
        vec![]
    } else {
        rest.split_whitespace().collect()
    };
    (cmd, args)
}

/// Execute a shell command, merge stdout+stderr, enforce timeout.
/// Returns combined output truncated to 4096 chars.
pub async fn exec_shell(cmd: &str, timeout_secs: u64) -> Result<String, BotError> {
    use tokio::io::AsyncReadExt;

    let mut child = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    // Take the I/O handles before the timeout select so we can still use child.kill().
    let mut stdout = child.stdout.take().expect("stdout piped");
    let mut stderr = child.stderr.take().expect("stderr piped");

    let deadline = tokio::time::sleep(std::time::Duration::from_secs(timeout_secs));
    tokio::pin!(deadline);

    // Read stdout + stderr concurrently then wait, or hit the deadline.
    let collected = tokio::select! {
        _ = &mut deadline => {
            let _ = child.kill().await;
            return Err(BotError::CommandTimeout { secs: timeout_secs });
        }
        result = async {
            let mut out_buf = Vec::new();
            let mut err_buf = Vec::new();
            // Read both streams concurrently.
            tokio::try_join!(
                stdout.read_to_end(&mut out_buf),
                stderr.read_to_end(&mut err_buf),
            )?;
            let status = child.wait().await?;
            Ok::<_, std::io::Error>((out_buf, err_buf, status))
        } => result,
    };

    let (out_buf, err_buf, status) = collected.map_err(BotError::Io)?;

    let mut combined = String::from_utf8_lossy(&out_buf).to_string();
    let stderr_str = String::from_utf8_lossy(&err_buf);
    if !stderr_str.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(&stderr_str);
    }
    if !status.success() {
        let code = status.code().unwrap_or(-1);
        combined.push_str(&format!("\n[exit: {}]", code));
    }
    if combined.len() > 4096 {
        // Walk back from byte 4093 to find a valid UTF-8 boundary (handles
        // 2-byte Cyrillic, 3-byte CJK, 4-byte emoji without panicking).
        let end = (0..=4093.min(combined.len()))
            .rev()
            .find(|&i| combined.is_char_boundary(i))
            .unwrap_or(0);
        combined.truncate(end);
        combined.push_str("...");
    }
    Ok(combined)
}

/// Execute a configured command with placeholder expansion and availability check.
pub async fn run_configured_cmd(
    cmd_cfg: &CommandConfig,
    args: &[&str],
    timeout_secs: u64,
) -> Result<String, BotError> {
    if cmd_cfg.unavailable {
        return Err(BotError::CommandUnavailable {
            cmd: cmd_cfg.name.clone(),
        });
    }
    let shell_cmd = if cmd_cfg.cmd.contains("{arg1}") {
        let arg1 = args.first().copied().unwrap_or("");
        sanitize_arg(arg1)?;
        expand_cmd(&cmd_cfg.cmd, args)
    } else {
        // {args} is passed verbatim to sh -c — no sanitization by design.
        // Admin responsibility; used for commands like /sudo and /snft.
        expand_cmd(&cmd_cfg.cmd, args)
    };
    exec_shell(&shell_cmd, timeout_secs).await
}

/// Build help text listing all available commands.
pub fn help_text(commands: &[CommandConfig], zabbix_configured: bool, lang: Lang) -> String {
    let mut lines = vec![lang.help_header().to_string()];
    for c in commands {
        lines.push(format!("/{} — {}", c.name, c.desc));
    }
    lines.push(lang.help_status().to_string());
    lines.push(lang.help_top().to_string());
    lines.push(lang.help_reboot().to_string());
    lines.push(lang.help_speedtest().to_string());
    lines.push(lang.help_whois().to_string());
    lines.push(lang.help_ping().to_string());
    if zabbix_configured {
        lines.push(lang.help_zbx_graph().to_string());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_exec_echo() {
        let result = exec_shell("echo hello", 5).await.unwrap();
        assert_eq!(result.trim(), "hello");
    }

    #[tokio::test]
    async fn test_exec_timeout() {
        let err = exec_shell("sleep 10", 1).await.unwrap_err();
        assert!(matches!(err, crate::error::BotError::CommandTimeout { .. }));
    }

    #[tokio::test]
    async fn test_exec_stderr_included() {
        let result = exec_shell("echo err >&2; echo out", 5).await.unwrap();
        assert!(result.contains("err"));
        assert!(result.contains("out"));
    }

    #[test]
    fn test_parse_no_args() {
        let (cmd, args) = parse_input("status");
        assert_eq!(cmd, "status");
        assert!(args.is_empty());
    }

    #[test]
    fn test_parse_with_args() {
        let (cmd, args) = parse_input("unban 1.2.3.4");
        assert_eq!(cmd, "unban");
        assert_eq!(args, vec!["1.2.3.4"]);
    }

    #[test]
    fn test_parse_slash_prefix() {
        let (cmd, _) = parse_input("/status");
        assert_eq!(cmd, "status");
    }

    #[test]
    fn test_parse_botname_suffix() {
        let (cmd, args) = parse_input("/status@mybotname");
        assert_eq!(cmd, "status");
        assert!(args.is_empty());
    }

    #[test]
    fn test_parse_botname_suffix_with_args() {
        let (cmd, args) = parse_input("/unban@mybot 1.2.3.4");
        assert_eq!(cmd, "unban");
        assert_eq!(args, vec!["1.2.3.4"]);
    }

    #[tokio::test]
    async fn test_exec_shell_nonzero_exit_appends_code() {
        let result = exec_shell("exit 5", 5).await.unwrap();
        assert!(result.contains("[exit: 5]"));
    }

    #[tokio::test]
    async fn test_run_configured_cmd_unavailable() {
        use crate::config::CommandConfig;
        let cfg = CommandConfig {
            name: "broken".into(),
            cmd: "echo hi".into(),
            desc: "d".into(),
            sudo_check: false,
            unavailable: true,
        };
        let err = run_configured_cmd(&cfg, &[], 5).await.unwrap_err();
        assert!(matches!(err, crate::error::BotError::CommandUnavailable { .. }));
    }

    #[tokio::test]
    async fn test_run_configured_cmd_arg1_valid() {
        use crate::config::CommandConfig;
        let cfg = CommandConfig {
            name: "test".into(),
            cmd: "echo {arg1}".into(),
            desc: "d".into(),
            sudo_check: false,
            unavailable: false,
        };
        let result = run_configured_cmd(&cfg, &["hello"], 5).await.unwrap();
        assert!(result.contains("hello"));
    }

    #[tokio::test]
    async fn test_run_configured_cmd_arg1_rejects_shell_injection() {
        use crate::config::CommandConfig;
        let cfg = CommandConfig {
            name: "test".into(),
            cmd: "echo {arg1}".into(),
            desc: "d".into(),
            sudo_check: false,
            unavailable: false,
        };
        let err = run_configured_cmd(&cfg, &["; evil"], 5).await.unwrap_err();
        assert!(matches!(err, crate::error::BotError::InvalidArgument { .. }));
    }

    #[test]
    fn test_help_text_excludes_zbx_without_zabbix() {
        let text = help_text(&[], false, crate::i18n::Lang::En);
        assert!(!text.contains("zbx_graph"));
    }

    #[test]
    fn test_help_text_includes_zbx_with_zabbix() {
        let text = help_text(&[], true, crate::i18n::Lang::En);
        assert!(text.contains("zbx_graph"));
    }
}
