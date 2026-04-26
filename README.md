*Русская версия: [README.ru.md](README.ru.md)*

---

# tgbot

Telegram bot for server management — written in Rust. Single compiled binary, no interpreter, no runtime dependencies.

## Features

- **Webhook** (axum, TCP or Unix socket) and **polling** modes — switchable via config
- **Configurable commands** — shell commands defined in TOML with argument sanitization
- **Zabbix integration** — JSON-RPC API client + graph image fetch via web UI
- **Built-in speedtest** — Ookla-compatible (ping / download / upload), no subprocess
- **Server status** — uptime, load average, CPU%, RAM, disk usage from `/proc`
- **Process top** — instantaneous CPU and RAM usage aggregated by process name
- **Ping** — ICMP reachability check with sanitized input and flag-injection prevention
- **Whois / RDAP** — IP info (country, city, org, registrant contacts) via RDAP over HTTPS
- **Safe reboot** — sends acknowledgement before triggering `systemctl reboot --force`
- **Threshold monitor** — background loop alerts super-admin on CPU/RAM/disk threshold breaches
- **PAM module** — `pam_tgbot.so`: login notifications to super-admin with inline buttons, and optional Telegram-based 2FA
- **CLI sender** — send Telegram messages from scripts/cron: `tgbot -m <chat_id> "text"`
- **IP whitelist** — Telegram CIDRs, custom CIDR list, or disabled (for reverse proxy)
- **Proxy support** — SOCKS5/HTTP via config
- **systemd hardening** — ProtectSystem, PrivateTmp, RuntimeDirectory, and more
- **sudo availability check** — commands flagged unavailable at startup if sudo is not permitted

## Requirements

- Rust 1.75+ (`rustup`)
- Linux (systemd for service mode)
- `sudo` configured for the `tgbot` user (for commands with `sudo_check = true`)

## Quick start

```bash
# 1. Build
make build

# 2. Deploy (installs binary + service + creates /etc/tgbot/config.toml)
sudo make deploy

# 3. Edit config
sudo nano /etc/tgbot/config.toml

# 4. Start
sudo systemctl start tgbot
sudo journalctl -u tgbot -f
```

## Configuration

Config file: `/etc/tgbot/config.toml` (permissions: `600`, owner `tgbot:tgbot`).

A fully-annotated example is in [`config.example.toml`](config.example.toml).

### Minimal config

```toml
[bot]
mode = "polling"
bind = "unix:/run/tgbot/bot.sock"
exec_timeout_secs = 30
webhook_ip_whitelist = "disabled"

[telegram]
token = "123456:YOUR-TOKEN-HERE"
api_address = ""
proxy = ""
request_timeout_secs = 10
request_retries = 3

[admins]
"123456789" = "YourName"   # first entry = super-admin

[zabbix]
url = "https://monit.example.com/"
user = "zabbix_api"
password = "secret"

[speedtest]
server_url = ""
```

### Threshold monitor

The `[monitor]` section is optional — all fields have defaults and monitoring is **disabled** by default.

```toml
[monitor]
enabled = true
interval_secs = 60    # check every 60 seconds
cpu_warn  = 85        # alert when CPU  >= 85%
ram_warn  = 90        # alert when RAM  >= 90%
disk_warn = 85        # alert when disk /  >= 85%
remind_secs = 1800    # repeat alert every 30 min while threshold is exceeded
```

When a threshold is breached, super-admin receives e.g. `⚠️ CPU: 91% (threshold 85%)`. When it drops back below, they receive `✅ CPU: 72% — back to normal`.

### Bot modes

| `mode` | Description |
|---|---|
| `"webhook"` | axum HTTP server; set `bind` to TCP address or `unix:/run/tgbot/bot.sock` |
| `"polling"` | Long-poll `getUpdates`; no public port needed |

### IP whitelist (webhook mode)

```toml
webhook_ip_whitelist = "telegram"          # Telegram's official IP ranges (hardcoded)
webhook_ip_whitelist = "disabled"          # No check — use behind a trusted proxy
webhook_ip_whitelist = ["1.2.3.0/24"]     # Custom CIDR list
```

### Commands

Each `[[commands]]` block defines a bot command:

```toml
[[commands]]
name = "unban"
cmd  = "sudo /usr/bin/fail2ban-client unban {arg1}"
desc = "Unban IP via fail2ban. Usage: /unban <IP>"
sudo_check = true
```

**Placeholders:**

| Placeholder | Behaviour |
|---|---|
| `{arg1}` | First argument, validated against `^[a-zA-Z0-9._/:-]+$` |
| `{args}` | All arguments verbatim (no sanitization — admin responsibility) |

**`sudo_check = true`**: at startup, `sudo -l -U tgbot <command>` is run. If sudo is denied, the command is marked unavailable and returns an error when called.

## Built-in commands

| Command | Description |
|---|---|
| `/status` | Server overview: uptime, load, CPU%, RAM, disk |
| `/top` | Top 5 processes by CPU and RAM (500 ms snapshot) |
| `/ping <host>` | ICMP reachability check (4 packets) |
| `/whois <IP>` | IP info via RDAP: country, city, org, registrant contacts |
| `/reboot` | Immediate forced reboot (sends acknowledgement first) |
| `/speedtest` | Ookla-compatible bandwidth test (ping + download + upload) |
| `/zbx_graph <itemid> <period> [name]` | Fetch a Zabbix graph as PNG. Period examples: `1h`, `24h`, `7d`, `86400s` |
| `/sudo <command>` | Run arbitrary shell command (super-admin only) |

## CLI sender

Send a message from any script or cron job:

```bash
# Plain message
tgbot -m 123456789 "Backup completed"

# Silent (no notification sound)
tgbot -m 123456789 "Routine log rotation done" --silent

# With inline buttons
tgbot -m 123456789 "Deploy ready?" "Deploy" "deploy_yes" "Cancel" "deploy_no"
```

Config path defaults to `/etc/tgbot/config.toml`. Override with:

```bash
tgbot --config /path/to/config.toml -m 123456789 "hello"
# or via environment variable:
TGBOT_CONFIG=/path/to/config.toml tgbot -m 123456789 "hello"
```

## Makefile targets

| Target | Description |
|---|---|
| `make build` | Release build (`target/release/tgbot`) |
| `make dev` | Debug run with `config.toml` in the current directory |
| `make test` | Run all tests |
| `make lint` | Run clippy |
| `make fmt` | Format code |
| `make deploy` | Build + install binary + install systemd service |
| `make update` | Build + install + restart service |
| `make install-service` | Install/update systemd unit only |
| `make uninstall` | Stop service, remove binary and unit |
| `make start/stop/restart/status/logs` | Service control |
| `make send CHAT=<id> MSG="text"` | Send a test message |
| `make set-webhook URL=https://...` | Register webhook with Telegram |
| `make delete-webhook` | Remove webhook (switch to polling) |
| `make clean` | `cargo clean` |

## Deployment

### First-time setup

```bash
# Build
make build

# Deploy (creates tgbot user, /etc/tgbot/, installs service)
sudo make deploy

# Edit config (fill in token, zabbix password, admin chat_id)
sudo nano /etc/tgbot/config.toml

# Configure sudo for commands that need it (example)
echo 'tgbot ALL=(ALL) NOPASSWD: /etc/sh/status.sh, /usr/bin/fail2ban-client' \
    | sudo tee /etc/sudoers.d/tgbot

# Start
sudo systemctl start tgbot
sudo journalctl -u tgbot -f
```

### Webhook with nginx

```nginx
location /secret_webhook {
    proxy_pass http://unix:/run/tgbot/bot.sock;
    proxy_set_header Host $host;
}
```

Then register the webhook:

```bash
sudo make set-webhook URL=https://your.domain/secret_webhook
```

### Updates

```bash
git pull
sudo make update   # build → install → restart
```

## Security model

- **IP whitelist**: Non-Telegram IPs get 403 with no body (webhook mode)
- **Admin check**: Every update's `chat_id` is verified against `[admins]`; strangers get dismissed and super-admin is notified
- **sudo restriction**: The `/sudo` command is restricted to the first entry in `[admins]` (super-admin)
- **Arg sanitization**: `{arg1}` validated against `^[a-zA-Z0-9._/:-]+$`; shell metacharacters rejected
- **No temp files**: Zabbix graphs are fetched and forwarded entirely in memory
- **rustls**: No OpenSSL dependency; TLS via rustls

## PAM module

`pam_tgbot.so` is a Rust shared library installed into `/usr/lib/security/`. It integrates with the Linux PAM stack to notify the super-admin of logins and optionally require Telegram approval before granting access.

### Features

| PAM type | Behaviour |
|---|---|
| `session optional pam_tgbot.so` | Sends a login notification with **[Terminate session]** and **[Block IP]** buttons |
| `auth required pam_tgbot.so` | Blocks login; super-admin receives ✅/❌ buttons; login proceeds only on approval |

### Setup

**1. Configure `/etc/tgbot/config.toml`:**

```toml
[pam]
enabled                 = true
notify_login            = true
two_factor_enabled      = false    # set true for full 2FA
two_factor_timeout_secs = 60
block_ip_cmd            = "ban-cs" # name of your block command in [[commands]]
```

**2. Edit `/etc/pam.d/sshd`** (start with notifications only):

```
session  optional  pam_tgbot.so
```

For 2FA (⚠️ **ensure you have console/backup access before enabling**):

```
# Add AFTER the existing auth lines:
auth     required  pam_tgbot.so
session  optional  pam_tgbot.so
```

**3. Allow session termination (optional — for the Terminate button):**

```bash
echo 'tgbot ALL=(ALL) NOPASSWD: /usr/bin/loginctl' \
    | sudo tee /etc/sudoers.d/tgbot-pam
sudo chmod 440 /etc/sudoers.d/tgbot-pam
```

### Notes

- The bot (`tgbot.service`) must be running. If it is down, **2FA is skipped** (fail-open) and notifications are not sent.
- The **Block IP** button sends `<block_ip_cmd> <ip>` as a Telegram callback, which the bot routes to your existing configured command.
- Do not enable `two_factor_enabled = true` on SSH without a tested fallback login path.

## Project structure

```
tgbot/
├── Cargo.toml
├── Makefile
├── config.example.toml        # Annotated config template
├── systemd/
│   └── tgbot.service          # Hardened systemd unit
└── src/
    ├── main.rs                # CLI parse, polling loop, webhook dispatch, sudo_check
    ├── config.rs              # TOML deserialization + validation helpers
    ├── error.rs               # BotError enum (11 typed variants)
    ├── bot/
    │   ├── mod.rs             # Dispatcher: auth, routing, built-ins
    │   ├── commands.rs        # Shell execution, parse_input, help_text
    │   ├── security.rs        # IpWhitelist, sanitize_arg, expand_cmd
    │   └── sender.rs          # Wrappers over TelegramClient
    ├── telegram/
    │   ├── client.rs          # reqwest Telegram API client (retry + timeout)
    │   ├── types.rs           # Update, Message, InlineKeyboardMarkup
    │   └── webhook.rs         # axum server (TCP + Unix socket + IP middleware)
    ├── zabbix/
    │   ├── mod.rs             # JSON-RPC client, auth token cache, auto re-login
    │   └── graph.rs           # Web-UI login + chart3.php PNG fetch (in-memory)
    ├── speedtest/
    │   └── mod.rs             # Ookla-compatible: server list → ping → dl → ul
    ├── system/
    │   └── mod.rs             # /status: uptime, load, CPU%, RAM, disk
    ├── whois/
    │   └── mod.rs             # RDAP lookup: country, city, org, contacts
    └── monitor/
        └── mod.rs             # Background threshold monitor with alert state machine
```

`pam_tgbot/` is a **separate cdylib crate** — built with `make pam`, installed with `make install-pam`. See the [PAM module](#pam-module) section for setup.
