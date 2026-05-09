use bytes::Bytes;
use serde::Deserialize;
use std::fmt;
use std::time::{Duration, Instant};
use tracing::{info, warn};

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

fn cmd_exists(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|d| d.join(name).is_file()))
        .unwrap_or(false)
}

fn is_valid_cli_output(text: &str) -> bool {
    text.contains("Download:") && text.contains("Upload:")
}

/// Returns the median of a non-empty sample list (middle element after sort).
/// Uses unwrap_or(Equal) to avoid panicking on NaN values.
fn median_f64(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    samples[samples.len() / 2]
}

async fn try_system_speedtest_cli() -> Option<String> {
    if !cmd_exists("speedtest-cli") {
        return None;
    }
    info!("speedtest: trying system speedtest-cli binary");
    let result = tokio::time::timeout(
        Duration::from_secs(25),
        tokio::process::Command::new("speedtest-cli")
            .arg("--simple")
            .output(),
    )
    .await;
    let out = match result {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => {
            warn!("speedtest: speedtest-cli spawn failed: {e}");
            return None;
        }
        Err(_) => {
            warn!("speedtest: speedtest-cli timed out after 25s");
            return None;
        }
    };
    if out.status.success() {
        let text = String::from_utf8_lossy(&out.stdout);
        let trimmed = text.trim();
        if is_valid_cli_output(trimmed) {
            return Some(trimmed.to_string());
        }
    }
    warn!(
        "speedtest: speedtest-cli failed (exit={:?})",
        out.status.code()
    );
    None
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
        match get_best_server(&client).await {
            Ok(server) => {
                info!(url = %server.url, "speedtest: auto-selected server");
                server.url
            }
            Err(e) => {
                warn!("speedtest: Ookla server discovery failed: {e}");
                match run_cloudflare_speedtest().await {
                    Ok(result) => return Ok(result),
                    Err(cf_err) => warn!("speedtest: Cloudflare fallback failed: {cf_err}"),
                }
                if let Some(result) = try_system_speedtest_cli().await {
                    return Ok(result);
                }
                return Err(BotError::Speedtest {
                    message: "all speedtest methods failed (Ookla, Cloudflare, speedtest-cli). \
                              Set server_url in [speedtest] config."
                        .into(),
                });
            }
        }
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

/// Cloudflare fallback time budget (fits within outer 90s):
///   Ookla discovery:  10s (per-request timeout in get_best_server)
///   CF ping:           5s (3 parallel tasks)
///   CF download:      30s (tokio timeout wrapper)
///   CF upload:        20s (tokio timeout wrapper)
///   speedtest-cli:    25s (only if CF also fails, has its own timeout)
async fn run_cloudflare_speedtest() -> Result<String, BotError> {
    const BASE: &str = "https://speed.cloudflare.com";
    info!("speedtest: using Cloudflare fallback");

    // HTTP/1.1: each parallel task gets its own TCP connection.
    // 30s client timeout is the backstop; explicit tokio timeouts below are primary.
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0")
        .http1_only()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| BotError::Speedtest { message: e.to_string() })?;

    // Ping: 3 tasks in parallel, 5s per-request timeout.
    // Parallel execution caps total ping phase at 5s instead of 3×5=15s.
    let ping_url = format!("{BASE}/__down?bytes=1");
    let ping_tasks: Vec<_> = (0..3)
        .map(|_| {
            let (c, u) = (client.clone(), ping_url.clone());
            tokio::spawn(async move {
                let t = Instant::now();
                c.get(&u)
                    .timeout(Duration::from_secs(5))
                    .send()
                    .await?
                    .chunk()
                    .await?;
                Ok::<f64, reqwest::Error>(t.elapsed().as_secs_f64() * 1000.0)
            })
        })
        .collect();
    let mut ping_samples: Vec<f64> = Vec::new();
    for task in ping_tasks {
        match task.await {
            Ok(Ok(ms)) => ping_samples.push(ms),
            Ok(Err(e)) => warn!("speedtest: cloudflare ping error: {e}"),
            Err(e) => warn!("speedtest: cloudflare ping task error: {e}"),
        }
    }
    if ping_samples.is_empty() {
        return Err(BotError::Speedtest {
            message: "cloudflare: ping failed (all 3 samples timed out or errored)".into(),
        });
    }
    let ping = median_f64(ping_samples);

    // Download: 4 parallel streams of 10 MB each via chunk() streaming.
    // tokio timeout ensures we never block past 30s even if chunk() hangs.
    // Partial data (bytes > 0) is accepted as a valid measurement sample.
    let down_url = format!("{BASE}/__down?bytes=10485760");
    let t0 = Instant::now();
    let download_result = tokio::time::timeout(Duration::from_secs(30), async {
        let tasks: Vec<_> = (0..4)
            .map(|_| {
                let (c, u) = (client.clone(), down_url.clone());
                tokio::spawn(async move {
                    let mut resp = c.get(&u).send().await?;
                    let mut bytes = 0usize;
                    loop {
                        match resp.chunk().await {
                            Ok(Some(chunk)) => bytes += chunk.len(),
                            Ok(None) => break,
                            Err(e) => {
                                if bytes == 0 {
                                    return Err(e);
                                }
                                break; // partial data is usable
                            }
                        }
                    }
                    Ok::<usize, reqwest::Error>(bytes)
                })
            })
            .collect();
        let mut total = 0usize;
        for t in tasks {
            match t.await {
                Ok(Ok(n)) => total += n,
                Ok(Err(e)) => warn!("speedtest: cloudflare download stream error: {e}"),
                Err(e) => warn!("speedtest: cloudflare download task error: {e}"),
            }
        }
        total
    })
    .await;
    let total_down = match download_result {
        Ok(n) if n > 0 => n,
        Ok(_) => {
            return Err(BotError::Speedtest {
                message: "cloudflare: all download streams failed".into(),
            })
        }
        Err(_) => {
            return Err(BotError::Speedtest {
                message: "cloudflare: download timed out after 30s".into(),
            })
        }
    };
    let download = bytes_to_mbps(total_down as f64 / t0.elapsed().as_secs_f64());

    // Upload: 4 parallel streams of 10 MB each.
    // send() is sufficient — HTTP/1.1 server responds only after receiving the full body.
    let up_url = format!("{BASE}/__up");
    let payload = Bytes::from(
        (0..10_485_760_usize)
            .map(|_| rand::random::<u8>())
            .collect::<Vec<_>>(),
    );
    let t0 = Instant::now();
    let upload_result = tokio::time::timeout(Duration::from_secs(20), async {
        let tasks: Vec<_> = (0..4)
            .map(|_| {
                let (c, u, d) = (client.clone(), up_url.clone(), payload.clone());
                tokio::spawn(async move {
                    c.post(&u).body(d).send().await?;
                    Ok::<usize, reqwest::Error>(10_485_760)
                })
            })
            .collect();
        let mut total = 0usize;
        for t in tasks {
            match t.await {
                Ok(Ok(n)) => total += n,
                Ok(Err(e)) => warn!("speedtest: cloudflare upload stream error: {e}"),
                Err(e) => warn!("speedtest: cloudflare upload task error: {e}"),
            }
        }
        total
    })
    .await;
    let total_up = match upload_result {
        Ok(n) if n > 0 => n,
        Ok(_) => {
            return Err(BotError::Speedtest {
                message: "cloudflare: all upload streams failed".into(),
            })
        }
        Err(_) => {
            return Err(BotError::Speedtest {
                message: "cloudflare: upload timed out after 20s".into(),
            })
        }
    };
    let upload = bytes_to_mbps(total_up as f64 / t0.elapsed().as_secs_f64());

    Ok(SpeedtestResult { ping_ms: ping, download_mbps: download, upload_mbps: upload }.to_string())
}

async fn get_best_server(client: &reqwest::Client) -> Result<OoklaServer, BotError> {
    let resp = client
        .get("https://www.speedtest.net/api/js/servers")
        .query(&[
            ("engine", "js"),
            ("https_functional", "true"),
            ("limit", "10"),
        ])
        // 10s per-request override: fast-fail when DPI-blocked,
        // without affecting the 30s timeout used for actual measurements.
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| BotError::Speedtest {
            message: format!("speedtest.net unreachable: {e}"),
        })?;
    let servers: Vec<OoklaServer> = resp.json().await.map_err(|e| BotError::Speedtest {
        message: format!("server list parse error: {e}"),
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
            })?
            .bytes()
            .await
            .map_err(|e| BotError::Speedtest {
                message: format!("ping read: {e}"),
            })?;
        samples.push(t.elapsed().as_secs_f64() * 1000.0);
    }
    Ok(median_f64(samples))
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
        assert_eq!(
            r.to_string(),
            "Ping: 12.3 ms\nDownload: 487.2 Mbit/s\nUpload: 213.8 Mbit/s"
        );
    }

    #[test]
    fn test_bytes_to_mbps() {
        assert!((bytes_to_mbps(125_000.0) - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_server_base_strips_upload() {
        assert_eq!(
            server_base("https://host.example.com/speedtest/upload.php"),
            "https://host.example.com/speedtest"
        );
    }

    #[test]
    fn test_server_base_no_suffix_unchanged() {
        assert_eq!(server_base("https://host.example.com"), "https://host.example.com");
    }

    #[test]
    fn test_cmd_exists_sh() {
        assert!(cmd_exists("sh"), "sh should always exist");
    }

    #[test]
    fn test_cmd_exists_nonexistent() {
        assert!(!cmd_exists("__nonexistent_cmd_12345__"));
    }

    #[test]
    fn test_cli_output_valid() {
        assert!(is_valid_cli_output(
            "Ping: 12.3 ms\nDownload: 100.00 Mbit/s\nUpload: 50.00 Mbit/s"
        ));
    }

    #[test]
    fn test_cli_output_missing_download() {
        assert!(!is_valid_cli_output("Ping: 12.3 ms\nUpload: 50.00 Mbit/s"));
    }

    #[test]
    fn test_cli_output_missing_upload() {
        assert!(!is_valid_cli_output("Ping: 12.3 ms\nDownload: 100.00 Mbit/s"));
    }

    #[test]
    fn test_cli_output_empty() {
        assert!(!is_valid_cli_output(""));
    }

    #[test]
    fn test_cli_output_error_message() {
        assert!(!is_valid_cli_output("Cannot retrieve speedtest configuration"));
    }

    #[test]
    fn test_bytes_to_mbps_zero() {
        assert_eq!(bytes_to_mbps(0.0), 0.0);
    }

    #[test]
    fn test_bytes_to_mbps_gigabit() {
        // 125_000_000 bytes/s = 1 Gbit/s
        assert!((bytes_to_mbps(125_000_000.0) - 1000.0).abs() < 0.01);
    }

    #[test]
    fn test_median_three_sorted() {
        assert_eq!(median_f64(vec![10.0, 20.0, 30.0]), 20.0);
    }

    #[test]
    fn test_median_three_unsorted() {
        assert_eq!(median_f64(vec![30.0, 10.0, 20.0]), 20.0);
    }

    #[test]
    fn test_median_two_samples_returns_higher() {
        // len/2 = 1 → second element after sort (higher value)
        assert_eq!(median_f64(vec![5.0, 15.0]), 15.0);
    }

    #[test]
    fn test_median_one_sample() {
        assert_eq!(median_f64(vec![42.0]), 42.0);
    }

    #[test]
    fn test_server_base_with_subpath() {
        assert_eq!(
            server_base("https://host.example.com/speedtest/upload.php"),
            "https://host.example.com/speedtest"
        );
    }

    #[test]
    fn test_server_base_no_upload_suffix_unchanged() {
        assert_eq!(
            server_base("https://host.example.com/path"),
            "https://host.example.com/path"
        );
    }
}
