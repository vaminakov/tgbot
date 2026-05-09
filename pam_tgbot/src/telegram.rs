use serde::Serialize;

#[derive(Serialize)]
struct Button {
    text: String,
    callback_data: String,
}

#[derive(Serialize)]
struct InlineKeyboard {
    inline_keyboard: Vec<Vec<Button>>,
}

#[derive(Serialize)]
struct SendMessage<'a> {
    chat_id: i64,
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_markup: Option<InlineKeyboard>,
}

async fn send_async(
    api_base: &str,
    chat_id: i64,
    text: &str,
    buttons: &[(&str, &str)],
    proxy: &str,
) -> Result<(), reqwest::Error> {
    let mut cb = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .use_rustls_tls();
    if !proxy.is_empty() {
        cb = cb.proxy(reqwest::Proxy::all(proxy)?);
    }
    let client = cb.build()?;

    let reply_markup = if buttons.is_empty() {
        None
    } else {
        let row = buttons
            .iter()
            .map(|(label, data)| Button {
                text: label.to_string(),
                callback_data: data.to_string(),
            })
            .collect();
        Some(InlineKeyboard {
            inline_keyboard: vec![row],
        })
    };

    client
        .post(format!("{}sendMessage", api_base))
        .json(&SendMessage {
            chat_id,
            text,
            reply_markup,
        })
        .send()
        .await?;
    Ok(())
}

fn make_rt() -> Option<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()
}

/// Fire-and-forget: silently ignores errors (used for notifications).
pub fn send(api_base: &str, chat_id: i64, text: &str, buttons: &[(&str, &str)], proxy: &str) {
    if let Some(rt) = make_rt() {
        let _ = rt.block_on(send_async(api_base, chat_id, text, buttons, proxy));
    }
}

/// Returns Err if the send fails — used by 2FA to abort when Telegram unreachable.
pub fn send_required(
    api_base: &str,
    chat_id: i64,
    text: &str,
    buttons: &[(&str, &str)],
    proxy: &str,
) -> Result<(), String> {
    let rt = make_rt().ok_or_else(|| "failed to create tokio runtime".to_string())?;
    rt.block_on(send_async(api_base, chat_id, text, buttons, proxy))
        .map_err(|e| e.to_string())
}
