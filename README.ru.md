*English version: [README.md](README.md)*

---

# tgbot

Telegram-бот для управления сервером — написан на Rust. Один скомпилированный бинарник, без интерпретатора, без runtime-зависимостей.

## Возможности

- **Webhook** (axum, TCP или Unix-сокет) и **polling** — переключается через конфиг
- **Настраиваемые команды** — shell-команды в TOML с валидацией аргументов
- **Интеграция с Zabbix** — JSON-RPC API + получение графиков через веб-интерфейс
- **Встроенный speedtest** — совместим с Ookla (ping / download / upload), без подпроцессов
- **Статус сервера** — uptime, load average, CPU%, RAM, диск из `/proc`
- **Топ процессов** — мгновенное потребление CPU и RAM по именам процессов
- **Ping** — проверка ICMP-доступности с валидацией и защитой от инъекции флагов
- **Whois / RDAP** — информация об IP (страна, город, org, контакты) через RDAP over HTTPS
- **Безопасная перезагрузка** — сначала отправляет подтверждение, затем вызывает `systemctl reboot --force`
- **Монитор порогов** — фоновый цикл уведомляет супер-администратора при превышении CPU/RAM/диска
- **PAM-модуль** — `pam_tgbot.so`: уведомления о входе с inline-кнопками и опциональная Telegram-2FA
- **CLI-отправка** — отправка сообщений из скриптов/cron: `tgbot -m <chat_id> "текст"`
- **Белый список IP** — диапазоны Telegram, свой список CIDR или отключено (для reverse proxy)
- **Поддержка прокси** — SOCKS5/HTTP через конфиг
- **Hardening systemd** — ProtectSystem, PrivateTmp, RuntimeDirectory и другие
- **Проверка sudo при запуске** — команды помечаются недоступными, если sudo не разрешён

## Требования

- Rust 1.75+ (`rustup`)
- Linux (systemd для режима службы)
- `sudo` настроен для пользователя `tgbot` (для команд с `sudo_check = true`)

## Быстрый старт

```bash
# 1. Сборка
make build

# 2. Установка (бинарник + служба + /etc/tgbot/config.toml)
sudo make deploy

# 3. Редактирование конфига
sudo nano /etc/tgbot/config.toml

# 4. Запуск
sudo systemctl start tgbot
sudo journalctl -u tgbot -f
```

## Конфигурация

Файл конфигурации: `/etc/tgbot/config.toml` (права: `600`, владелец `tgbot:tgbot`).

Полностью аннотированный пример — в файле [`config.example.toml`](config.example.toml).

### Минимальный конфиг

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
"123456789" = "YourName"   # первая запись = супер-администратор

[zabbix]
url = "https://monit.example.com/"
user = "zabbix_api"
password = "secret"

[speedtest]
server_url = ""
```

### Монитор порогов

Секция `[monitor]` необязательна — все поля имеют значения по умолчанию, мониторинг **отключён** по умолчанию.

```toml
[monitor]
enabled = true
interval_secs = 60    # проверять каждые 60 секунд
cpu_warn  = 85        # уведомлять при CPU  >= 85%
ram_warn  = 90        # уведомлять при RAM  >= 90%
disk_warn = 85        # уведомлять при диске /  >= 85%
remind_secs = 1800    # повторять уведомление каждые 30 мин при превышении
```

При превышении порога супер-администратор получает, например, `⚠️ CPU: 91% (порог 85%)`. При снижении ниже порога — `✅ CPU: 72% — норма восстановлена`.

### Режимы работы бота

| `mode` | Описание |
|---|---|
| `"webhook"` | HTTP-сервер axum; `bind` — TCP-адрес или `unix:/run/tgbot/bot.sock` |
| `"polling"` | Long-poll `getUpdates`; публичный порт не нужен |

### Белый список IP (режим webhook)

```toml
webhook_ip_whitelist = "telegram"          # официальные диапазоны Telegram (встроены)
webhook_ip_whitelist = "disabled"          # без проверки — для использования за доверенным прокси
webhook_ip_whitelist = ["1.2.3.0/24"]     # свой список CIDR
```

### Команды

Каждый блок `[[commands]]` определяет команду бота:

```toml
[[commands]]
name = "unban"
cmd  = "sudo /usr/bin/fail2ban-client unban {arg1}"
desc = "Разблокировать IP через fail2ban. Использование: /unban <IP>"
sudo_check = true
```

**Плейсхолдеры:**

| Плейсхолдер | Поведение |
|---|---|
| `{arg1}` | Первый аргумент, проверяется по `^[a-zA-Z0-9._/:-]+$` |
| `{args}` | Все аргументы дословно (без валидации — ответственность администратора) |

**`sudo_check = true`**: при запуске выполняется `sudo -l -U tgbot <команда>`. Если sudo запрещён, команда помечается недоступной и возвращает ошибку при вызове.

## Встроенные команды

| Команда | Описание |
|---|---|
| `/status` | Обзор сервера: uptime, load, CPU%, RAM, диск |
| `/top` | Топ 5 процессов по CPU и RAM (снимок за 500 мс) |
| `/ping <host>` | Проверка ICMP-доступности (4 пакета) |
| `/whois <IP>` | Информация об IP через RDAP: страна, город, org, контакты |
| `/reboot` | Немедленная принудительная перезагрузка (сначала отправляет подтверждение) |
| `/speedtest` | Тест скорости, совместимый с Ookla (ping + download + upload) |
| `/zbx_graph <itemid> <period> [name]` | Получить график Zabbix в виде PNG. Примеры периода: `1h`, `24h`, `7d`, `86400s` |
| `/sudo <команда>` | Выполнить произвольную shell-команду (только супер-администратор) |

## CLI-отправка сообщений

Отправить сообщение из любого скрипта или cron-задания:

```bash
# Обычное сообщение
tgbot -m 123456789 "Резервная копия завершена"

# Без звука
tgbot -m 123456789 "Плановая ротация логов" --silent

# С inline-кнопками
tgbot -m 123456789 "Деплой готов?" "Деплоить" "deploy_yes" "Отмена" "deploy_no"
```

Путь к конфигу по умолчанию: `/etc/tgbot/config.toml`. Переопределить:

```bash
tgbot --config /path/to/config.toml -m 123456789 "hello"
# или через переменную окружения:
TGBOT_CONFIG=/path/to/config.toml tgbot -m 123456789 "hello"
```

## Цели Makefile

| Цель | Описание |
|---|---|
| `make build` | Сборка релиза (`target/release/tgbot`) |
| `make dev` | Запуск debug-версии с `config.toml` в текущем каталоге |
| `make test` | Запуск всех тестов |
| `make lint` | Запуск clippy |
| `make fmt` | Форматирование кода |
| `make deploy` | Сборка + установка бинарника + установка службы systemd |
| `make update` | Сборка + установка + перезапуск службы |
| `make install-service` | Только установка/обновление unit-файла systemd |
| `make uninstall` | Остановка службы, удаление бинарника и unit-файла |
| `make start/stop/restart/status/logs` | Управление службой |
| `make send CHAT=<id> MSG="text"` | Отправить тестовое сообщение |
| `make set-webhook URL=https://...` | Зарегистрировать webhook в Telegram |
| `make delete-webhook` | Удалить webhook (переключиться на polling) |
| `make clean` | `cargo clean` |

## Развёртывание

### Первичная установка

```bash
# Сборка
make build

# Установка (создаёт пользователя tgbot, /etc/tgbot/, устанавливает службу)
sudo make deploy

# Редактирование конфига (заполнить token, пароль zabbix, chat_id администратора)
sudo nano /etc/tgbot/config.toml

# Настройка sudo для команд, которым он нужен (пример)
echo 'tgbot ALL=(ALL) NOPASSWD: /etc/sh/status.sh, /usr/bin/fail2ban-client' \
    | sudo tee /etc/sudoers.d/tgbot

# Запуск
sudo systemctl start tgbot
sudo journalctl -u tgbot -f
```

### Webhook с nginx

```nginx
location /secret_webhook {
    proxy_pass http://unix:/run/tgbot/bot.sock;
    proxy_set_header Host $host;
}
```

Затем зарегистрировать webhook:

```bash
sudo make set-webhook URL=https://your.domain/secret_webhook
```

### Обновление

```bash
git pull
sudo make update   # сборка → установка → перезапуск
```

## Модель безопасности

- **Белый список IP**: запросы с не-Telegram IP получают 403 без тела (режим webhook)
- **Проверка администратора**: `chat_id` каждого обновления проверяется по `[admins]`; посторонние игнорируются, супер-администратор получает уведомление
- **Ограничение sudo**: команда `/sudo` доступна только первой записи в `[admins]` (супер-администратор)
- **Валидация аргументов**: `{arg1}` проверяется по `^[a-zA-Z0-9._/:-]+$`; shell-метасимволы отклоняются
- **Без временных файлов**: графики Zabbix получаются и пересылаются полностью в памяти
- **rustls**: без зависимости от OpenSSL; TLS через rustls

## PAM-модуль

`pam_tgbot.so` — разделяемая библиотека на Rust, устанавливаемая в `/usr/lib/security/`. Интегрируется со стеком Linux PAM для уведомления супер-администратора о входах и опционального требования подтверждения через Telegram перед предоставлением доступа.

### Возможности

| Тип PAM | Поведение |
|---|---|
| `session optional pam_tgbot.so` | Отправляет "🔑 Авторизован: user с IP" с кнопками **[Завершить сессию]** и **[Блокировать IP]** |
| `auth required pam_tgbot.so` | Блокирует вход; супер-администратор получает кнопки ✅/❌; вход разрешается только при одобрении |

### Настройка

**1. Настроить `/etc/tgbot/config.toml`:**

```toml
[pam]
enabled                 = true
notify_login            = true
two_factor_enabled      = false    # установить true для полной 2FA
two_factor_timeout_secs = 60
block_ip_cmd            = "ban-cs" # имя команды блокировки в [[commands]]
```

**2. Отредактировать `/etc/pam.d/sshd`** (начать с уведомлений):

```
session  optional  pam_tgbot.so
```

Для 2FA (⚠️ **убедитесь в наличии консольного/резервного доступа перед включением**):

```
# Добавить ПОСЛЕ существующих строк auth:
auth     required  pam_tgbot.so
session  optional  pam_tgbot.so
```

**3. Разрешить завершение сессии (опционально — для кнопки «Завершить сессию»):**

```bash
echo 'tgbot ALL=(ALL) NOPASSWD: /usr/bin/loginctl' \
    | sudo tee /etc/sudoers.d/tgbot-pam
sudo chmod 440 /etc/sudoers.d/tgbot-pam
```

### Примечания

- Служба `tgbot.service` должна быть запущена. Если она остановлена, **2FA пропускается** (fail-open) и уведомления не отправляются.
- Кнопка **Блокировать IP** отправляет `<block_ip_cmd> <ip>` как Telegram callback, который бот маршрутизирует к вашей настроенной команде.
- Не включайте `two_factor_enabled = true` для SSH без проверенного резервного пути входа.

## Структура проекта

```
tgbot/
├── Cargo.toml
├── Makefile
├── config.example.toml        # Аннотированный шаблон конфигурации
├── systemd/
│   └── tgbot.service          # Hardened unit-файл systemd
└── src/
    ├── main.rs                # Парсинг CLI, цикл polling, диспетчер webhook, sudo_check
    ├── config.rs              # Десериализация TOML + вспомогательные функции валидации
    ├── error.rs               # Перечисление BotError (11 типизированных вариантов)
    ├── bot/
    │   ├── mod.rs             # Диспетчер: аутентификация, маршрутизация, встроенные команды
    │   ├── commands.rs        # Выполнение shell, parse_input, help_text
    │   ├── security.rs        # IpWhitelist, sanitize_arg, expand_cmd
    │   └── sender.rs          # Обёртки над TelegramClient
    ├── telegram/
    │   ├── client.rs          # reqwest-клиент Telegram API (повторы + таймаут)
    │   ├── types.rs           # Update, Message, InlineKeyboardMarkup
    │   └── webhook.rs         # Сервер axum (TCP + Unix-сокет + IP-middleware)
    ├── zabbix/
    │   ├── mod.rs             # JSON-RPC клиент, кэш токена авторизации, авто-переподключение
    │   └── graph.rs           # Вход через веб-интерфейс + получение PNG chart3.php (в памяти)
    ├── speedtest/
    │   └── mod.rs             # Совместимый с Ookla: список серверов → ping → dl → ul
    ├── system/
    │   └── mod.rs             # /status: uptime, load, CPU%, RAM, диск
    ├── whois/
    │   └── mod.rs             # RDAP-запрос: страна, город, org, контакты
    └── monitor/
        └── mod.rs             # Фоновый монитор порогов с автоматом состояний для алертов
```

`pam_tgbot/` — **отдельный cdylib-крейт** — собирается командой `make pam`, устанавливается командой `make install-pam`. Подробности в разделе [PAM-модуль](#pam-модуль).
