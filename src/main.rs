mod bot;
mod config;
mod error;
mod i18n;
mod monitor;
mod speedtest;
mod system;
mod telegram;
mod whois;
mod zabbix;

use clap::Parser;
use std::sync::Arc;
use tracing::{info, warn};

use bot::{security::IpWhitelist, BotContext};
use config::{BotMode, Config};
use i18n::Lang;
use telegram::client::TelegramClient;
use telegram::types::InlineKeyboardMarkup;

#[derive(Parser)]
#[command(name = "tgbot", about = "Telegram server management bot")]
struct Cli {
    #[arg(
        short,
        long,
        default_value = "/etc/tgbot/config.toml",
        env = "TGBOT_CONFIG"
    )]
    config: String,

    /// Send a message and exit: -m <chat_id> <text> [btn_label btn_data ...]
    #[arg(short = 'm', long = "message", num_args = 2..)]
    message: Option<Vec<String>>,

    /// Send silently (no notification). Used with -m.
    #[arg(long)]
    silent: bool,
}

async fn configure_telegram_mode(
    config: &config::Config,
    tg: &TelegramClient,
) -> Result<(), crate::error::BotError> {
    use config::BotMode;

    match config.bot.mode {
        BotMode::Polling => {
            info!("Polling mode: deleting webhook (drop_pending=true)");
            tg.delete_webhook(true).await?;
        }
        BotMode::Webhook => {
            let url = match &config.bot.webhook_address {
                Some(u) if !u.is_empty() => u,
                _ => {
                    info!("webhook_address not set, skipping webhook configuration");
                    return Ok(());
                }
            };

            if config.bot.always_set_webhook {
                info!(url, "Setting webhook (always_set_webhook=true)");
                tg.set_webhook(url, false).await?;
            } else {
                let wh_info = tg.get_webhook_info().await?;
                let url_mismatch = wh_info.url != *url;
                let has_error = wh_info.last_error_message.is_some();

                if !url_mismatch && !has_error {
                    info!("Webhook OK, no changes needed");
                    return Ok(());
                }

                if has_error {
                    warn!(
                        error = ?wh_info.last_error_message,
                        pending = wh_info.pending_update_count,
                        "Webhook error detected, re-registering with drop_pending=true"
                    );
                }

                tg.set_webhook(url, has_error).await?;
                info!(url, "Webhook updated");

                if has_error && config.bot.notify_on_webhook_error {
                    if let Some(super_id) = config.super_admin_id() {
                        let msg = format!(
                            "⚠️ Webhook ошибка при старте: {}\nВебхук переregister'ен.",
                            wh_info.last_error_message.as_deref().unwrap_or("unknown")
                        );
                        let _ = tg.send_message(super_id, &msg, None, false).await;
                    }
                }
            }
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive("tgbot=info".parse()?),
        )
        .init();

    let cli = Cli::parse();
    let config = Config::load(&cli.config)?;

    // ── CLI sender mode ───────────────────────────────────────────────────
    if let Some(args) = cli.message {
        let chat_id: i64 = args[0].parse()?;
        let text = &args[1];
        let mut markup: Option<InlineKeyboardMarkup> = None;
        if args.len() > 2 {
            let pairs: Vec<(&str, &str)> = args[2..]
                .chunks(2)
                .filter_map(|c| {
                    if c.len() == 2 {
                        Some((c[0].as_str(), c[1].as_str()))
                    } else {
                        None
                    }
                })
                .collect();
            if !pairs.is_empty() {
                markup = Some(InlineKeyboardMarkup::single_row(&pairs));
            }
        }
        let tg = TelegramClient::new(&config.telegram)?;
        tg.send_message(chat_id, text, markup.as_ref(), cli.silent)
            .await?;
        return Ok(());
    }

    // ── Build shared context ──────────────────────────────────────────────
    let tg = Arc::new(TelegramClient::new(&config.telegram)?);

    configure_telegram_mode(&config, &tg).await?;

    let mut patched = config.clone();
    sudo_check_commands(&mut patched.commands).await;
    pam_startup_check(&config);

    let lang = Lang::from_config(&config.bot.language);
    let ctx = Arc::new(BotContext {
        config: Arc::new(patched),
        tg: Arc::clone(&tg),
        lang,
        rate_limit: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
    });

    let monitor_handle = tokio::spawn(monitor::run(Arc::clone(&ctx)));

    // ── Start server ──────────────────────────────────────────────────────
    match config.bot.mode {
        BotMode::Webhook => {
            info!("starting in webhook mode");
            let wl = IpWhitelist::from_config(&config.bot.webhook_ip_whitelist)?;
            telegram::webhook::serve(ctx, &config.bot.bind, &config.bot.webhook_path, wl).await?;
        }
        BotMode::Polling => {
            info!("starting in polling mode");
            run_polling(ctx, tg).await;
        }
    }
    monitor_handle.abort();
    Ok(())
}

async fn run_polling(ctx: Arc<BotContext>, tg: Arc<TelegramClient>) {
    use tokio::signal::unix::{signal, SignalKind};
    use tokio::time::{sleep, Duration};

    let mut offset: i64 = 0;
    let mut backoff: u64 = 1;

    let mut sigterm = signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("SIGINT received, stopping");
                break;
            }
            _ = sigterm.recv() => {
                info!("SIGTERM received, stopping");
                break;
            }
            result = tg.get_updates(offset, 25) => {
                match result {
                    Ok(updates) => {
                        backoff = 1;
                        for update in &updates {
                            let next = update.update_id + 1;
                            if next > offset { offset = next; }
                            let ctx = Arc::clone(&ctx);
                            let update = update.clone();
                            tokio::spawn(async move { bot::dispatch(&update, &ctx).await });
                        }
                    }
                    Err(e) => {
                        tracing::warn!(%e, backoff, "getUpdates failed, retrying");
                        sleep(Duration::from_secs(backoff)).await;
                        backoff = (backoff * 2).min(30);
                    }
                }
            }
        }
    }
}

fn pam_startup_check(cfg: &config::Config) {
    if !cfg.pam.enabled {
        return;
    }
    if !cfg.pam.notify_login && !cfg.pam.two_factor_enabled {
        tracing::warn!(
            "PAM: integration enabled but both notify_login=false and two_factor_enabled=false \
             — pam_tgbot.so will do nothing; set at least one to true in [pam] config"
        );
    }
    let ipc_dir = std::path::Path::new("/run/tgbot/pam");
    if !ipc_dir.exists() {
        tracing::warn!(
            "PAM: /run/tgbot/pam does not exist — \
             run 'systemctl daemon-reload && systemctl restart tgbot' to create it via ExecStartPre"
        );
    } else {
        tracing::info!(
            two_factor = cfg.pam.two_factor_enabled,
            notify_login = cfg.pam.notify_login,
            "PAM: IPC directory OK"
        );
    }
}

async fn sudo_check_commands(commands: &mut [config::CommandConfig]) {
    for cmd in commands.iter_mut() {
        if !cmd.sudo_check {
            continue;
        }
        // Pass the full command (minus "sudo") to `sudo -l` so that
        // argument-specific sudoers rules (e.g. NOPASSWD: /bin/foo bar *)
        // are matched correctly. Placeholders are replaced with a dummy value
        // that matches any wildcard rule in sudoers.
        let args: Vec<String> = cmd
            .cmd
            .split_whitespace()
            .filter(|w| *w != "sudo")
            .map(|w| match w {
                "{arg1}" | "{args}" => "_check_".to_string(),
                other => other.to_string(),
            })
            .collect();
        if args.is_empty() {
            tracing::warn!(cmd = %cmd.name, "sudo_check: no command word found, skipping");
            continue;
        }

        let ok = tokio::process::Command::new("sudo")
            .arg("-l")
            .arg("-U")
            .arg("tgbot")
            .arg("--")
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false);

        if ok {
            info!(cmd = %cmd.name, "sudo_check: OK");
        } else {
            tracing::warn!(cmd = %cmd.name, "sudo_check: DENIED");
            cmd.unavailable = true;
        }
    }
}
