use bytes::Bytes;
use serde::Deserialize;
use std::fmt;
use std::time::{Duration, Instant};
use tracing::info;

use crate::error::BotError;

pub struct SpeedtestResult {
    pub ping_ms: f64,
    pub download_mbps: f64,
    pub upload_mbps: f64,
}

impl fmt::Display for SpeedtestResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Ping: {:.1} ms\nDownload: {:.1} Mbit/s\nUpload: {:.1} Mbit/s",
            self.ping_ms, self.download_mbps, self.upload_mbps
        )
    }
}

pub fn bytes_to_mbps(bytes_per_sec: f64) -> f64 {
    bytes_per_sec * 8.0 / 1_000_000.0
}

pub fn server_base(url: &str) -> &str {
    url.trim_end_matches("/upload.php")
}

#[derive(Deserialize, Clone)]
struct OoklaServer {
    url: String,
    #[serde(default)]
    distance: f64,
}

/// Synchronous entry point — called via spawn_blocking from the async runtime.
/// `server_url`: explicit Ookla server URL (e.g. "https://host:8080/upload.php"),
/// or empty string to auto-select the nearest server via speedtest.net API.
pub fn run(server_url: String) -> Result<String, BotError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| BotError::Speedtest {
            message: e.to_string(),
        })?;
    rt.block_on(async {
        tokio::time::timeout(Duration::from_secs(90), run_async(server_url))
            .await
            .map_err(|_| BotError::Speedtest {
                message: "speedtest timed out after 90s".into(),
            })?
    })
}

async fn run_async(server_url: String) -> Result<String, BotError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("Mozilla/5.0")
        .build()
        .map_err(|e| BotError::Speedtest {
            message: e.to_string(),
        })?;

    let url = if server_url.is_empty() {
        let server = get_best_server(&client).await?;
        info!(url = %server.url, "speedtest: auto-selected server");
        server.url
    } else {
        info!(url = %server_url, "speedtest: using configured server");
        server_url
    };

    let ping = measure_ping(&client, &url).await?;
    let download = measure_download(&client, &url).await?;
    let upload = measure_upload(&client, &url).await?;

    Ok(SpeedtestResult {
        ping_ms: ping,
        download_mbps: download,
        upload_mbps: upload,
    }
    .to_string())

}

async fn get_best_server(client: &reqwest::Client) -> Result<OoklaServer, BotError> {
    let resp = client
        .get("https://www.speedtest.net/api/js/servers")
        .query(&[
            ("engine", "js"),
            ("https_functional", "true"),
            ("limit", "10"),
        ])
        .send()
        .await
        .map_err(|e| BotError::Speedtest {
            message: format!("server list: {e}"),
        })?;
    let servers: Vec<OoklaServer> = resp.json().await.map_err(|e| BotError::Speedtest {
        message: format!("server parse: {e}"),
    })?;
    servers
        .into_iter()
        .min_by(|a, b| {
            a.distance
                .partial_cmp(&b.distance)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .ok_or_else(|| BotError::Speedtest {
            message: "empty server list".into(),
        })
}

async fn measure_ping(client: &reqwest::Client, server_url: &str) -> Result<f64, BotError> {
    let url = format!("{}/latency.txt", server_base(server_url));
    let mut samples = Vec::with_capacity(3);
    for _ in 0..3 {
        let t = Instant::now();
        client
            .get(&url)
            .send()
            .await
            .map_err(|e| BotError::Speedtest {
                message: format!("ping: {e}"),
            })?;
        samples.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    Ok(samples[1]) // median of 3
}

async fn measure_download(client: &reqwest::Client, server_url: &str) -> Result<f64, BotError> {
    let url = format!("{}/random7500x7500.jpg", server_base(server_url));
    let t0 = Instant::now();
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let (c, u) = (client.clone(), url.clone());
        tasks.push(tokio::spawn(async move {
            let bytes = c.get(&u).send().await?.bytes().await?;
            Ok::<usize, reqwest::Error>(bytes.len())
        }));
    }
    let mut total = 0usize;
    for t in tasks {
        if let Ok(Ok(n)) = t.await {
            total += n;
        }
    }
    if total == 0 {
        return Err(BotError::Speedtest {
            message: "download failed (no data received from server)".into(),
        });
    }
    Ok(bytes_to_mbps(total as f64 / t0.elapsed().as_secs_f64()))
}

async fn measure_upload(client: &reqwest::Client, server_url: &str) -> Result<f64, BotError> {
    // bytes::Bytes is Arc-backed — clone() is O(1), no 10 MB copy per task
    let payload = Bytes::from(
        (0..10_485_760_usize)
            .map(|_| rand::random::<u8>())
            .collect::<Vec<_>>(),
    );
    let t0 = Instant::now();
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let (c, u, d) = (client.clone(), server_url.to_string(), payload.clone());
        tasks.push(tokio::spawn(async move {
            let len = d.len();
            let _ = c.post(&u).body(d).send().await?.bytes().await?;
            Ok::<usize, reqwest::Error>(len)
        }));
    }
    let mut total = 0usize;
    for t in tasks {
        if let Ok(Ok(n)) = t.await {
            total += n;
        }
    }
    if total == 0 {
        return Err(BotError::Speedtest {
            message: "upload failed (no data sent to server)".into(),
        });
    }
    Ok(bytes_to_mbps(total as f64 / t0.elapsed().as_secs_f64()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_output() {
        let r = SpeedtestResult {
            ping_ms: 12.3,
            download_mbps: 487.2,
            upload_mbps: 213.8,
        };
        let s = r.to_string();
        assert_eq!(
            s,
            "Ping: 12.3 ms\nDownload: 487.2 Mbit/s\nUpload: 213.8 Mbit/s"
        );
    }

    #[test]
    fn test_bytes_to_mbps() {
        // 125_000 bytes/s = 1 Mbit/s
        assert!((bytes_to_mbps(125_000.0) - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_server_base_strips_upload() {
        assert_eq!(
            server_base("https://host.example.com/speedtest/upload.php"),
            "https://host.example.com/speedtest"
        );
    }
}
