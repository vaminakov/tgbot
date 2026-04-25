use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct WebhookInfo {
    pub url: String,
    pub pending_update_count: u32,
    pub last_error_message: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Update {
    pub update_id: i64,
    pub message: Option<Message>,
    pub callback_query: Option<CallbackQuery>,
}

impl Update {
    /// Effective chat ID — from message or callback_query.
    pub fn chat_id(&self) -> Option<i64> {
        self.message.as_ref().map(|m| m.chat.id).or_else(|| {
            self.callback_query
                .as_ref()
                .and_then(|cq| cq.message.as_ref())
                .map(|m| m.chat.id)
        })
    }

    /// Text content — message text or callback data.
    pub fn text(&self) -> Option<&str> {
        self.message
            .as_ref()
            .and_then(|m| m.text.as_deref())
            .or_else(|| {
                self.callback_query
                    .as_ref()
                    .and_then(|cq| cq.data.as_deref())
            })
    }

    /// Sender's first name for display in rejection messages.
    pub fn first_name(&self) -> &str {
        self.message
            .as_ref()
            .and_then(|m| m.from.as_ref())
            .map(|u| u.first_name.as_str())
            .or_else(|| {
                self.callback_query
                    .as_ref()
                    .map(|cq| cq.from.first_name.as_str())
            })
            .unwrap_or("Человек")
    }

    /// Sender's @username if present.
    pub fn username(&self) -> Option<&str> {
        self.message
            .as_ref()
            .and_then(|m| m.from.as_ref())
            .and_then(|u| u.username.as_deref())
            .or_else(|| {
                self.callback_query
                    .as_ref()
                    .and_then(|cq| cq.from.username.as_deref())
            })
    }

    /// Callback query ID (for answerCallbackQuery).
    pub fn callback_query_id(&self) -> Option<&str> {
        self.callback_query.as_ref().map(|cq| cq.id.as_str())
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct Message {
    pub chat: Chat,
    pub from: Option<User>,
    pub text: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct Chat {
    pub id: i64,
}

#[derive(Debug, Deserialize, Clone)]
pub struct User {
    pub first_name: String,
    pub username: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct CallbackQuery {
    pub id: String,
    pub from: User,
    pub message: Option<Message>,
    pub data: Option<String>,
}

#[derive(Debug, Serialize, Clone, Default)]
pub struct InlineKeyboardMarkup {
    pub inline_keyboard: Vec<Vec<InlineKeyboardButton>>,
}

#[derive(Debug, Serialize, Clone)]
pub struct InlineKeyboardButton {
    pub text: String,
    pub callback_data: String,
}

impl InlineKeyboardMarkup {
    /// Build a single-row keyboard from (label, callback_data) pairs.
    pub fn single_row(buttons: &[(&str, &str)]) -> Self {
        Self {
            inline_keyboard: vec![buttons
                .iter()
                .map(|(t, d)| InlineKeyboardButton {
                    text: t.to_string(),
                    callback_data: d.to_string(),
                })
                .collect()],
        }
    }
}

/// Default inline keyboard shown with help / unknown command response.
pub fn default_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::single_row(&[
        ("status", "status"),
        ("speedtest", "speedtest"),
        ("ddos (20)", "snft -d -f 20"),
        ("geoip on", "mikrotik geoip_on"),
        ("reboot", "reboot"),
    ])
}
