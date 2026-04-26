use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::sleep;
use tracing::warn;

use crate::config::TelegramConfig;
use crate::error::BotError;
use crate::telegram::types::{InlineKeyboardMarkup, Update, WebhookInfo};

pub struct TelegramClient {
    pub(crate) base_urls: Vec<String>,
    client: reqwest::Client,
    timeout: Duration,
    retries: u32,
}

#[derive(Deserialize)]
struct TgResult<T> {
    ok: bool,
    result: Option<T>,
    error_code: Option<i64>,
    description: Option<String>,
}

impl TelegramClient {
    pub fn new(cfg: &TelegramConfig) -> Result<Self, BotError> {
        let mut builder = reqwest::Client::builder();
        if !cfg.proxy.is_empty() {
            builder = builder.proxy(reqwest::Proxy::all(&cfg.proxy)?);
        }
        Ok(Self {
            base_urls: cfg.api_base_urls(),
            client: builder.build()?,
            timeout: Duration::from_secs(cfg.request_timeout_secs),
            retries: cfg.request_retries,
        })
    }

    /// POST a JSON-serializable body with retry on timeout/5xx.
    /// Tries each URL in base_urls in order; moves to next URL only after all retries fail.
    /// API errors (wrong params, auth) are returned immediately without trying other URLs.
    async fn post_json<T, B>(&self, method: &str, body: &B) -> Result<T, BotError>
    where
        T: for<'de> Deserialize<'de>,
        B: Serialize,
    {
        let mut last: Option<BotError> = None;

        for (url_idx, base_url) in self.base_urls.iter().enumerate() {
            let url = format!("{}{}", base_url, method);

            for attempt in 1..=self.retries {
                let req = self.client.post(&url).json(body).send();
                match tokio::time::timeout(self.timeout, req).await {
                    Err(_) => {
                        warn!(method, attempt, %url, "Telegram API timeout");
                        last = Some(BotError::TelegramTimeout {
                            method: method.to_string(),
                        });
                    }
                    Ok(Err(e)) => {
                        warn!(method, attempt, %url, %e, "Telegram API network error");
                        last = Some(BotError::TelegramNetwork(e));
                    }
                    Ok(Ok(resp)) => {
                        if resp.status().is_server_error() {
                            let status = resp.status().as_u16();
                            warn!(method, attempt, status, "Telegram API 5xx");
                            last = Some(BotError::TelegramTimeout {
                                method: method.to_string(),
                            });
                        } else {
                            let r: TgResult<T> = resp.json().await?;
                            if !r.ok {
                                // API errors are not retried — wrong params, auth error, etc.
                                return Err(BotError::TelegramApi {
                                    code: r.error_code.unwrap_or(0),
                                    description: r.description.unwrap_or_default(),
                                });
                            }
                            return r.result.ok_or_else(|| BotError::TelegramApi {
                                code: 0,
                                description: "ok=true but result field missing".into(),
                            });
                        }
                    }
                }
                if attempt < self.retries {
                    sleep(Duration::from_millis(300 * attempt as u64)).await;
                }
            }

            if url_idx + 1 < self.base_urls.len() {
                warn!(method, url = %base_url, "all retries failed, trying next API address");
            }
        }

        Err(last.unwrap_or_else(|| BotError::TelegramTimeout {
            method: method.to_string(),
        }))
    }

    /// POST multipart (file upload) — single attempt, 60s timeout.
    async fn post_multipart<T>(
        &self,
        method: &str,
        form: reqwest::multipart::Form,
    ) -> Result<T, BotError>
    where
        T: for<'de> Deserialize<'de>,
    {
        let url = format!("{}{}", self.base_urls[0], method);
        let req = self.client.post(&url).multipart(form).send();
        let resp = tokio::time::timeout(Duration::from_secs(60), req)
            .await
            .map_err(|_| BotError::TelegramTimeout {
                method: method.to_string(),
            })??;
        let r: TgResult<T> = resp.json().await?;
        if !r.ok {
            return Err(BotError::TelegramApi {
                code: r.error_code.unwrap_or(0),
                description: r.description.unwrap_or_default(),
            });
        }
        r.result.ok_or_else(|| BotError::TelegramApi {
            code: 0,
            description: "ok=true but result field missing".into(),
        })
    }

    // ── Public API methods ────────────────────────────────────────────────

    pub async fn send_message(
        &self,
        chat_id: i64,
        text: &str,
        markup: Option<&InlineKeyboardMarkup>,
        silent: bool,
    ) -> Result<(), BotError> {
        #[derive(Serialize)]
        struct Body<'a> {
            chat_id: i64,
            text: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            reply_markup: Option<&'a InlineKeyboardMarkup>,
            #[serde(skip_serializing_if = "std::ops::Not::not")]
            disable_notification: bool,
        }
        self.post_json::<serde_json::Value, _>(
            "sendMessage",
            &Body {
                chat_id,
                text,
                reply_markup: markup,
                disable_notification: silent,
            },
        )
        .await?;
        Ok(())
    }

    pub async fn send_document(
        &self,
        chat_id: i64,
        filename: &str,
        bytes: Vec<u8>,
    ) -> Result<(), BotError> {
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(filename.to_string())
            .mime_str("application/octet-stream")?;
        let form = reqwest::multipart::Form::new()
            .text("chat_id", chat_id.to_string())
            .part("document", part);
        self.post_multipart::<serde_json::Value>("sendDocument", form)
            .await?;
        Ok(())
    }


    pub async fn answer_callback_query(&self, callback_query_id: &str) -> Result<(), BotError> {
        #[derive(Serialize)]
        struct Body<'a> {
            callback_query_id: &'a str,
        }
        self.post_json::<serde_json::Value, _>("answerCallbackQuery", &Body { callback_query_id })
            .await?;
        Ok(())
    }

    /// Long-poll getUpdates. timeout_secs is the server-side wait (use 25).
    pub async fn get_updates(
        &self,
        offset: i64,
        timeout_secs: u64,
    ) -> Result<Vec<Update>, BotError> {
        #[derive(Serialize)]
        struct Body {
            offset: i64,
            timeout: u64,
            limit: u32,
        }
        let url = format!("{}getUpdates", self.base_urls[0]);
        let outer = Duration::from_secs(timeout_secs + 10);
        let req = self
            .client
            .post(&url)
            .json(&Body {
                offset,
                timeout: timeout_secs,
                limit: 100,
            })
            .send();
        let resp =
            tokio::time::timeout(outer, req)
                .await
                .map_err(|_| BotError::TelegramTimeout {
                    method: "getUpdates".into(),
                })??;
        let r: TgResult<Vec<Update>> = resp.json().await?;
        if !r.ok {
            return Err(BotError::TelegramApi {
                code: r.error_code.unwrap_or(0),
                description: r.description.unwrap_or_default(),
            });
        }
        Ok(r.result.unwrap_or_default())
    }

    pub async fn set_webhook(&self, url: &str, drop_pending_updates: bool) -> Result<(), BotError> {
        #[derive(Serialize)]
        struct Body<'a> {
            url: &'a str,
            drop_pending_updates: bool,
        }
        self.post_json::<serde_json::Value, _>(
            "setWebhook",
            &Body { url, drop_pending_updates },
        )
        .await?;
        Ok(())
    }

    pub async fn delete_webhook(&self, drop_pending_updates: bool) -> Result<(), BotError> {
        #[derive(Serialize)]
        struct Body {
            drop_pending_updates: bool,
        }
        self.post_json::<serde_json::Value, _>("deleteWebhook", &Body { drop_pending_updates })
            .await?;
        Ok(())
    }

    pub async fn get_webhook_info(&self) -> Result<WebhookInfo, BotError> {
        self.post_json::<WebhookInfo, _>("getWebhookInfo", &serde_json::json!({}))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TelegramConfig;

    fn test_cfg() -> TelegramConfig {
        TelegramConfig {
            token: "test_token".into(),
            api_address: String::new(),
            api_addresses: vec![],
            proxy: String::new(),
            request_timeout_secs: 5,
            request_retries: 2,
        }
    }

    #[test]
    fn test_client_builds() {
        assert!(TelegramClient::new(&test_cfg()).is_ok());
    }

    #[test]
    fn test_base_url() {
        let client = TelegramClient::new(&test_cfg()).unwrap();
        assert_eq!(client.base_urls.len(), 1);
        assert!(client.base_urls[0].contains("api.telegram.org"));
        assert!(client.base_urls[0].contains("test_token"));
    }
}
