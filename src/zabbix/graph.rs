use reqwest::cookie::Jar;
use std::sync::Arc;
use tracing::info;

use crate::config::ZabbixConfig;
use crate::error::BotError;

/// Fetch a Zabbix graph as PNG bytes (in-memory, no temp files on disk).
///
/// 1. POST login to {url}index.php → session cookie
/// 2. GET {url}chart3.php?... → PNG body
/// 3. Validate Content-Type is image/png
pub async fn fetch(
    cfg: &ZabbixConfig,
    item_id: u64,
    period: &str,
    graph_name: &str,
) -> Result<Vec<u8>, BotError> {
    let jar = Arc::new(Jar::default());
    let client = reqwest::Client::builder()
        .cookie_provider(Arc::clone(&jar))
        .timeout(std::time::Duration::from_secs(60))
        .build()?;

    // Step 1: Login via Zabbix web UI
    let base = cfg.url.trim_end_matches('/');
    let login_resp = client
        .post(format!("{}/index.php", base))
        .form(&[
            ("name", cfg.user.as_str()),
            ("password", cfg.password.as_str()),
            ("enter", "Sign in"),
        ])
        .send()
        .await?;

    if !login_resp.status().is_success() && login_resp.status().as_u16() != 302 {
        return Err(BotError::ZabbixGraph {
            message: format!("Zabbix login failed: HTTP {}", login_resp.status()),
        });
    }
    let _ = login_resp.bytes().await; // consume body to store session cookie

    // Step 2: Fetch graph image
    let chart_url = format!(
        "{}/chart3.php?from=now-{period}&to=now&name={name}&width=1920&height=540\
         &graphtype=0&legend=1\
         &items[0][itemid]={item_id}&items[0][sortorder]=0\
         &items[0][drawtype]=5&items[0][color]=00CC00",
        base,
        period = period,
        name = urlencoding::encode(graph_name),
        item_id = item_id,
    );

    info!(item_id, period, "fetching Zabbix graph");
    let resp = client.get(&chart_url).send().await?;

    // Step 3: Validate Content-Type
    let ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    if !ct.contains("image/png") {
        return Err(BotError::ZabbixGraph {
            message: format!(
                "Response is not PNG (content-type: {}). Zabbix login may have failed.",
                ct
            ),
        });
    }

    let bytes = resp.bytes().await?.to_vec();
    if bytes.is_empty() {
        return Err(BotError::ZabbixGraph {
            message: "Empty PNG response".into(),
        });
    }
    Ok(bytes)
}
