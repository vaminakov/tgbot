use crate::error::BotError;
use crate::telegram::client::TelegramClient;
use crate::telegram::types::InlineKeyboardMarkup;

pub async fn send(
    client: &TelegramClient,
    chat_id: i64,
    text: &str,
    markup: Option<&InlineKeyboardMarkup>,
) -> Result<(), BotError> {
    if text.is_empty() {
        return Ok(());
    }
    client.send_message(chat_id, text, markup, false).await
}

pub async fn send_error(client: &TelegramClient, chat_id: i64, err: &BotError) {
    let text = format!("Ошибка: {}", err);
    tracing::error!(%err, chat_id, "command error");
    if let Err(send_err) = client.send_message(chat_id, &text, None, false).await {
        tracing::error!(%send_err, chat_id, "failed to deliver error message to user");
    }
}
