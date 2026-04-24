mod bot;
mod config;
mod error;
mod speedtest;
mod telegram;
mod zabbix;

use clap::Parser;
use std::sync::Arc;
use tracing::info;

use bot::{security::IpWhitelist, BotContext};
use config::{BotMode, Config};
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
    let zbx = if !config.zabbix.url.is_empty() {
        Some(Arc::new(zabbix::ZabbixClient::new(&config.zabbix)))
    } else {
        None
    };

    let mut patched = config.clone();
    sudo_check_commands(&mut patched.commands).await;

    let ctx = Arc::new(BotContext {
        config: Arc::new(patched),
        tg: Arc::clone(&tg),
        zabbix: zbx,
    });

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
    Ok(())
}

async fn run_polling(ctx: Arc<BotContext>, tg: Arc<TelegramClient>) {
    use tokio::signal::unix::{signal, SignalKind};
    use tokio::time::{sleep, Duration};

    let _ = tg.delete_webhook().await;
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

async fn sudo_check_commands(commands: &mut Vec<config::CommandConfig>) {
    for cmd in commands.iter_mut() {
        if !cmd.sudo_check {
            continue;
        }
        let first_word = cmd
            .cmd
            .split_whitespace()
            .find(|w| *w != "sudo")
            .unwrap_or("")
            .to_string();
        if first_word.is_empty() {
            tracing::warn!(cmd = %cmd.name, "sudo_check: no command word found, skipping");
            continue;
        }

        let ok = tokio::process::Command::new("sudo")
            .args(["-l", "-U", "tgbot", &first_word])
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
