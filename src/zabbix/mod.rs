pub mod graph;

use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::warn;

use crate::config::ZabbixConfig;
use crate::error::BotError;

pub struct ZabbixClient {
    config: ZabbixConfig,
    client: reqwest::Client,
    auth: Arc<Mutex<Option<String>>>,
}

pub fn rpc_body(method: &str, params: Value, auth: Option<&str>) -> Value {
    serde_json::json!({
        "jsonrpc": "2.0",
        "method":  method,
        "params":  params,
        "id":      1,
        "auth":    auth,
    })
}

impl ZabbixClient {
    pub fn new(cfg: &ZabbixConfig) -> Self {
        Self {
            config: cfg.clone(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("Zabbix HTTP client build failed"),
            auth: Arc::new(Mutex::new(None)),
        }
    }

    fn endpoint(&self) -> String {
        let url = self.config.url.trim_end_matches('/');
        format!("{}/api_jsonrpc.php", url)
    }

    async fn login(&self) -> Result<String, BotError> {
        let body = rpc_body(
            "user.login",
            serde_json::json!({
                "username": self.config.user,
                "password": self.config.password,
            }),
            None,
        );
        let resp: Value = self
            .client
            .post(&self.endpoint())
            .json(&body)
            .send()
            .await?
            .json()
            .await?;
        resp["result"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| BotError::ZabbixApi {
                message: resp["error"]["data"]
                    .as_str()
                    .unwrap_or("login failed")
                    .to_string(),
            })
    }

    /// Call a Zabbix API method. Re-authenticates once on "Not authorised".
    pub async fn call(&self, method: &str, params: Value) -> Result<Value, BotError> {
        let token = {
            let mut g = self.auth.lock().await;
            if g.is_none() {
                *g = Some(self.login().await?);
            }
            g.clone().unwrap()
        };

        let body = rpc_body(method, params.clone(), Some(&token));
        let resp: Value = self
            .client
            .post(&self.endpoint())
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        if resp["error"]["data"]
            .as_str()
            .map(|s| s.contains("Not authorised"))
            .unwrap_or(false)
        {
            warn!("Zabbix session expired, re-authenticating");
            let new_tok = self.login().await?;
            *self.auth.lock().await = Some(new_tok.clone());
            let body2 = rpc_body(method, params, Some(&new_tok));
            let resp2: Value = self
                .client
                .post(&self.endpoint())
                .json(&body2)
                .send()
                .await?
                .json()
                .await?;
            return extract_result(resp2);
        }

        extract_result(resp)
    }

    /// Check Zabbix API version — no authentication required.
    pub async fn check_version(&self) -> Result<String, BotError> {
        let body = serde_json::json!({
            "jsonrpc": "2.0", "method": "apiinfo.version",
            "params": {}, "id": 1,
        });
        let resp: Value = self
            .client
            .post(&self.endpoint())
            .json(&body)
            .send()
            .await?
            .json()
            .await?;
        resp["result"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| BotError::ZabbixApi {
                message: "no version in response".into(),
            })
    }
}

fn extract_result(resp: Value) -> Result<Value, BotError> {
    if resp["error"].is_object() {
        Err(BotError::ZabbixApi {
            message: resp["error"]["data"]
                .as_str()
                .or_else(|| resp["error"]["message"].as_str())
                .unwrap_or("unknown Zabbix error")
                .to_string(),
        })
    } else {
        Ok(resp["result"].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rpc_body_no_auth() {
        let body = rpc_body("user.login", serde_json::json!({"user": "u"}), None);
        assert_eq!(body["jsonrpc"], "2.0");
        assert_eq!(body["method"], "user.login");
        assert_eq!(body["id"], 1);
        assert!(body["auth"].is_null());
    }

    #[test]
    fn test_rpc_body_with_auth() {
        let body = rpc_body("trigger.get", serde_json::json!({}), Some("mytoken"));
        assert_eq!(body["auth"], "mytoken");
    }
}
