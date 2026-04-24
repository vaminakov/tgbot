BIN      := tgbot
DESTDIR  := /usr/local/bin
SVCDIR   := /etc/systemd/system
CFGDIR   := /etc/tgbot
SVCUSER  := tgbot

# ── Build ─────────────────────────────────────────────────────────────────────

.PHONY: build
build:
	cargo build --release

.PHONY: dev
dev:
	RUST_LOG=tgbot=debug cargo run -- --config config.toml

.PHONY: check
check:
	cargo check

.PHONY: test
test:
	cargo test

.PHONY: lint
lint:
	cargo clippy -- -D warnings

.PHONY: fmt
fmt:
	cargo fmt

.PHONY: fmt-check
fmt-check:
	cargo fmt --check

# ── Install / Deploy ──────────────────────────────────────────────────────────

.PHONY: install
install: build
	install -m 755 target/release/$(BIN) $(DESTDIR)/$(BIN)

.PHONY: deploy
deploy: install install-service

.PHONY: install-service
install-service:
	@# Create system user if missing
	id -u $(SVCUSER) >/dev/null 2>&1 || \
	    useradd --system --no-create-home --shell /usr/sbin/nologin $(SVCUSER)
	@# Config directory
	install -d -m 750 -o $(SVCUSER) -g $(SVCUSER) $(CFGDIR)
	@if [ ! -f $(CFGDIR)/config.toml ]; then \
	    install -m 600 -o $(SVCUSER) -g $(SVCUSER) config.example.toml $(CFGDIR)/config.toml; \
	    echo ""; \
	    echo "  >> $(CFGDIR)/config.toml created from example."; \
	    echo "  >> Edit it and fill in token, zabbix password, admin chat_id."; \
	    echo ""; \
	fi
	@# systemd unit
	install -m 644 systemd/tgbot.service $(SVCDIR)/tgbot.service
	systemctl daemon-reload
	systemctl enable tgbot
	@echo "Run: systemctl start tgbot"

.PHONY: update
update: build
	install -m 755 target/release/$(BIN) $(DESTDIR)/$(BIN)
	systemctl restart tgbot
	systemctl status tgbot --no-pager

.PHONY: uninstall
uninstall:
	systemctl disable --now tgbot || true
	rm -f $(SVCDIR)/tgbot.service $(DESTDIR)/$(BIN)
	systemctl daemon-reload

# ── Service control ───────────────────────────────────────────────────────────

.PHONY: start
start:
	systemctl start tgbot

.PHONY: stop
stop:
	systemctl stop tgbot

.PHONY: restart
restart:
	systemctl restart tgbot

.PHONY: status
status:
	systemctl status tgbot --no-pager

.PHONY: logs
logs:
	journalctl -u tgbot -f

# ── Dev helpers ───────────────────────────────────────────────────────────────

# Send a test message: make send CHAT=123456789 MSG="hello"
.PHONY: send
send:
	./target/release/$(BIN) -m $(CHAT) "$(MSG)"

# Send a silent test message: make send-silent CHAT=123456789 MSG="hello"
.PHONY: send-silent
send-silent:
	./target/release/$(BIN) -m $(CHAT) "$(MSG)" --silent

.PHONY: set-webhook
set-webhook:
	@[ -n "$(URL)" ] || (echo "Usage: make set-webhook URL=https://your.domain/path"; exit 1)
	@TOKEN=$$(grep 'token' $(CFGDIR)/config.toml | head -1 | cut -d'"' -f2); \
	curl -s "https://api.telegram.org/bot$$TOKEN/setWebhook?url=$(URL)" | python3 -m json.tool

.PHONY: delete-webhook
delete-webhook:
	@TOKEN=$$(grep 'token' $(CFGDIR)/config.toml | head -1 | cut -d'"' -f2); \
	curl -s "https://api.telegram.org/bot$$TOKEN/deleteWebhook" | python3 -m json.tool

.PHONY: clean
clean:
	cargo clean
