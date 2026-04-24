pub mod commands;
pub mod security;
pub mod sender;

use std::sync::Arc;
use tracing::{info, warn};

use crate::config::Config;
use crate::error::BotError;
use crate::telegram::client::TelegramClient;
use crate::telegram::types::{default_keyboard, Update};
use commands::{help_text, parse_input, run_configured_cmd};
use sender::{send, send_error};

pub struct BotContext {
    pub config: Arc<Config>,
    pub tg: Arc<TelegramClient>,
    pub zabbix: Option<Arc<crate::zabbix::ZabbixClient>>,
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
            &format!("{}, не пиши мне больше!", update.first_name()),
            None,
        )
        .await;
        warn!(chat_id, %username, "unauthorized access attempt");
        return;
    }

    // Acknowledge callback query now that auth has passed
    if let Some(cq_id) = update.callback_query_id() {
        let _ = ctx.tg.answer_callback_query(cq_id).await;
    }

    info!(chat_id, text, "command received");
    let (cmd_name, args) = parse_input(text);

    // sudo restricted to super-admin
    if cmd_name == "sudo" && !ctx.config.is_super_admin(chat_id) {
        let _ = send(
            &ctx.tg,
            chat_id,
            "Команда sudo доступна только super-admin.",
            None,
        )
        .await;
        return;
    }

    // ── Built-in commands ─────────────────────────────────────────────────
    let builtin: Option<Result<String, BotError>> = match cmd_name {
        "speedtest" => Some(handle_speedtest().await),
        "zbx_graph" => Some(handle_zbx_graph(&args, chat_id, ctx).await),
        "tr" => Some(handle_zabbix_check(ctx).await),
        _ => None,
    };

    if let Some(result) = builtin {
        match result {
            Ok(ref t) if !t.is_empty() => {
                let _ = send(&ctx.tg, chat_id, t, None).await;
            }
            Ok(_) => {} // already sent (e.g. document)
            Err(ref e) => send_error(&ctx.tg, chat_id, e).await,
        }
        return;
    }

    // ── Configured commands ───────────────────────────────────────────────
    if let Some(cmd_cfg) = ctx.config.commands.iter().find(|c| c.name == cmd_name) {
        let timeout = ctx.config.bot.exec_timeout_secs;
        match run_configured_cmd(cmd_cfg, &args, timeout).await {
            Ok(out) => {
                let _ = send(&ctx.tg, chat_id, &out, None).await;
            }
            Err(e) => send_error(&ctx.tg, chat_id, &e).await,
        }
        return;
    }

    // ── Unknown → help ────────────────────────────────────────────────────
    let kb = default_keyboard();
    let _ = send(
        &ctx.tg,
        chat_id,
        &help_text(&ctx.config.commands),
        Some(&kb),
    )
    .await;
}

async fn handle_speedtest() -> Result<String, BotError> {
    tokio::task::spawn_blocking(crate::speedtest::run)
        .await
        .map_err(|e| BotError::Speedtest {
            message: e.to_string(),
        })?
}

async fn handle_zabbix_check(ctx: &BotContext) -> Result<String, BotError> {
    match &ctx.zabbix {
        Some(zbx) => zbx
            .check_version()
            .await
            .map(|v| format!("Zabbix API: {}", v)),
        None => Ok("Zabbix не настроен.".into()),
    }
}

fn validate_period(p: &str) -> bool {
    if p.is_empty() {
        return false;
    }
    let (digits, suffix) = p.split_at(p.len() - 1);
    !digits.is_empty()
        && digits.chars().all(|c| c.is_ascii_digit())
        && matches!(suffix, "s" | "m" | "h" | "d" | "w" | "M" | "y")
}

async fn handle_zbx_graph(
    args: &[&str],
    chat_id: i64,
    ctx: &BotContext,
) -> Result<String, BotError> {
    let _zbx = match &ctx.zabbix {
        Some(z) => z,
        None => return Ok("Zabbix не настроен.".into()),
    };
    if args.is_empty() {
        return Ok("Использование: /zbx_graph <itemid> <period> [name]".into());
    }
    let item_id: u64 = args[0].parse().map_err(|_| BotError::InvalidArgument {
        input: args[0].to_string(),
    })?;
    let period = args.get(1).copied().unwrap_or("1h");
    if !validate_period(period) {
        return Err(BotError::InvalidArgument {
            input: period.to_string(),
        });
    }
    let name = args.get(2).copied().unwrap_or("");
    let bytes = crate::zabbix::graph::fetch(&ctx.config.zabbix, item_id, period, name).await?;
    ctx.tg.send_document(chat_id, "graph.png", bytes).await?;
    Ok(String::new()) // document already sent
}
