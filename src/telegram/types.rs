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

#[cfg(test)]
mod tests {
    use super::*;

    fn msg_update(chat_id: i64, text: &str, first_name: &str, username: Option<&str>) -> Update {
        Update {
            update_id: 1,
            message: Some(Message {
                chat: Chat { id: chat_id },
                from: Some(User {
                    first_name: first_name.to_string(),
                    username: username.map(str::to_string),
                }),
                text: Some(text.to_string()),
            }),
            callback_query: None,
        }
    }

    fn cb_update(chat_id: i64, data: &str, first_name: &str, cb_id: &str) -> Update {
        Update {
            update_id: 2,
            message: None,
            callback_query: Some(CallbackQuery {
                id: cb_id.to_string(),
                from: User { first_name: first_name.to_string(), username: None },
                message: Some(Message {
                    chat: Chat { id: chat_id },
                    from: None,
                    text: None,
                }),
                data: Some(data.to_string()),
            }),
        }
    }

    #[test]
    fn chat_id_from_message() {
        assert_eq!(msg_update(42, "hi", "Bob", None).chat_id(), Some(42));
    }

    #[test]
    fn chat_id_from_callback_query() {
        assert_eq!(cb_update(99, "status", "Alice", "cq1").chat_id(), Some(99));
    }

    #[test]
    fn chat_id_none_when_both_absent() {
        let u = Update { update_id: 0, message: None, callback_query: None };
        assert_eq!(u.chat_id(), None);
    }

    #[test]
    fn text_from_message() {
        assert_eq!(msg_update(1, "/status", "Bob", None).text(), Some("/status"));
    }

    #[test]
    fn text_from_callback_data() {
        assert_eq!(cb_update(1, "speedtest", "Alice", "cq1").text(), Some("speedtest"));
    }

    #[test]
    fn text_none_when_both_absent() {
        let u = Update { update_id: 0, message: None, callback_query: None };
        assert_eq!(u.text(), None);
    }

    #[test]
    fn first_name_from_message() {
        assert_eq!(msg_update(1, "hi", "Ivan", None).first_name(), "Ivan");
    }

    #[test]
    fn first_name_from_callback_query() {
        assert_eq!(cb_update(1, "x", "Anna", "cq1").first_name(), "Anna");
    }

    #[test]
    fn first_name_fallback_when_both_absent() {
        let u = Update { update_id: 0, message: None, callback_query: None };
        assert_eq!(u.first_name(), "Человек");
    }

    #[test]
    fn username_present() {
        assert_eq!(msg_update(1, "x", "Bob", Some("bobuser")).username(), Some("bobuser"));
    }

    #[test]
    fn username_absent() {
        assert_eq!(msg_update(1, "x", "Bob", None).username(), None);
    }

    #[test]
    fn callback_query_id_present() {
        assert_eq!(cb_update(1, "x", "Alice", "cqid42").callback_query_id(), Some("cqid42"));
    }

    #[test]
    fn callback_query_id_absent_for_message() {
        assert_eq!(msg_update(1, "x", "Bob", None).callback_query_id(), None);
    }

    #[test]
    fn single_row_keyboard_shape() {
        let kb = InlineKeyboardMarkup::single_row(&[("OK", "ok"), ("No", "no")]);
        assert_eq!(kb.inline_keyboard.len(), 1);
        assert_eq!(kb.inline_keyboard[0].len(), 2);
        assert_eq!(kb.inline_keyboard[0][0].text, "OK");
        assert_eq!(kb.inline_keyboard[0][0].callback_data, "ok");
    }

    #[test]
    fn single_row_empty_produces_one_empty_row() {
        let kb = InlineKeyboardMarkup::single_row(&[]);
        assert_eq!(kb.inline_keyboard.len(), 1);
        assert!(kb.inline_keyboard[0].is_empty());
    }
}
