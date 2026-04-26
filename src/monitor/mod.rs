use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::{info, warn};

use crate::bot::BotContext;

struct CpuSnapshot {
    total: u64,
    idle: u64,
}

#[derive(Default)]
struct AlertState {
    in_alert: bool,
    last_alert: Option<Instant>,
}

#[derive(Debug, PartialEq)]
enum AlertAction {
    Alert,
    Recover,
}

/// Pure function: decides whether to alert, recover, or do nothing.
/// `over` = current value is at or above threshold.
fn should_alert(state: &AlertState, over: bool, remind_secs: u64) -> Option<AlertAction> {
    if over {
        if !state.in_alert {
            return Some(AlertAction::Alert);
        }
        let due = state
            .last_alert
            .map(|t| t.elapsed().as_secs() >= remind_secs)
            .unwrap_or(true);
        if due { Some(AlertAction::Alert) } else { None }
    } else if state.in_alert {
        Some(AlertAction::Recover)
    } else {
        None
    }
}

async fn read_cpu_snapshot() -> Option<CpuSnapshot> {
    let content = tokio::fs::read_to_string("/proc/stat").await.ok()?;
    let line = content.lines().next()?;
    let nums: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|s| s.parse().ok())
        .collect();
    let idle = nums.get(3).copied().unwrap_or(0)
        + nums.get(4).copied().unwrap_or(0); // idle + iowait
    let total: u64 = nums.iter().sum();
    Some(CpuSnapshot { total, idle })
}

async fn read_ram_percent() -> Option<u64> {
    let content = tokio::fs::read_to_string("/proc/meminfo").await.ok()?;
    let mut total = 0u64;
    let mut avail = 0u64;
    for line in content.lines() {
        if let Some(val) = line.strip_prefix("MemTotal:") {
            total = val.split_whitespace().next()?.parse().ok()?;
        } else if let Some(val) = line.strip_prefix("MemAvailable:") {
            avail = val.split_whitespace().next()?.parse().ok()?;
        }
    }
    if total == 0 { return None; }
    let used = total.saturating_sub(avail);
    Some(used * 100 / total)
}

async fn read_disk_percent() -> Option<u64> {
    let out = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::process::Command::new("df")
            .args(["-B1", "/"])
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let line = s.lines().nth(1)?;
    let mut cols = line.split_whitespace();
    let _fs = cols.next()?;
    let _total = cols.next()?;
    let used: u64 = cols.next()?.parse().ok()?;
    let avail: u64 = cols.next()?.parse().ok()?;
    let denom = used + avail;
    if denom == 0 { return None; }
    Some(used * 100 / denom)
}

async fn fire(ctx: &BotContext, super_id: i64, msg: &str) {
    if let Err(e) = ctx.tg.send_message(super_id, msg, None, false).await {
        warn!(%e, "monitor: failed to send alert");
    }
}

pub async fn run(ctx: Arc<BotContext>) {
    let cfg = ctx.config.monitor.clone();
    if !cfg.enabled {
        return;
    }

    info!(
        interval_secs = cfg.interval_secs,
        cpu_warn = cfg.cpu_warn,
        ram_warn = cfg.ram_warn,
        disk_warn = cfg.disk_warn,
        "monitor: starting"
    );

    let remind_secs = cfg.remind_secs.max(1);
    let mut interval = tokio::time::interval(Duration::from_secs(cfg.interval_secs.max(1)));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let super_id = match ctx.config.super_admin_id() {
        Some(id) => id,
        None => {
            warn!("monitor: no super-admin configured, monitoring disabled");
            return;
        }
    };

    let mut cpu_prev: Option<CpuSnapshot> = None;
    let mut cpu_st = AlertState::default();
    let mut ram_st = AlertState::default();
    let mut disk_st = AlertState::default();

    loop {
        interval.tick().await;

        // ── CPU ────────────────────────────────────────────────────────
        if let Some(snap) = read_cpu_snapshot().await {
            if let Some(prev) = &cpu_prev {
                let d_total = snap.total.saturating_sub(prev.total);
                let d_idle = snap.idle.saturating_sub(prev.idle);
                if let Some(pct) = ((d_total - d_idle) * 100).checked_div(d_total) {
                    match should_alert(&cpu_st, pct >= cfg.cpu_warn as u64, remind_secs) {
                        Some(AlertAction::Alert) => {
                            fire(&ctx, super_id,
                                &ctx.lang.monitor_cpu_alert(pct as u8, cfg.cpu_warn)).await;
                            cpu_st.in_alert = true;
                            cpu_st.last_alert = Some(Instant::now());
                        }
                        Some(AlertAction::Recover) => {
                            fire(&ctx, super_id,
                                &ctx.lang.monitor_cpu_recover(pct as u8)).await;
                            cpu_st.in_alert = false;
                            cpu_st.last_alert = None;
                        }
                        None => {}
                    }
                }
            }
            cpu_prev = Some(snap);
        }

        // ── RAM ────────────────────────────────────────────────────────
        if let Some(pct) = read_ram_percent().await {
            match should_alert(&ram_st, pct >= cfg.ram_warn as u64, remind_secs) {
                Some(AlertAction::Alert) => {
                    fire(&ctx, super_id,
                        &ctx.lang.monitor_ram_alert(pct as u8, cfg.ram_warn)).await;
                    ram_st.in_alert = true;
                    ram_st.last_alert = Some(Instant::now());
                }
                Some(AlertAction::Recover) => {
                    fire(&ctx, super_id,
                        &ctx.lang.monitor_ram_recover(pct as u8)).await;
                    ram_st.in_alert = false;
                    ram_st.last_alert = None;
                }
                None => {}
            }
        }

        // ── Disk ───────────────────────────────────────────────────────
        if let Some(pct) = read_disk_percent().await {
            match should_alert(&disk_st, pct >= cfg.disk_warn as u64, remind_secs) {
                Some(AlertAction::Alert) => {
                    fire(&ctx, super_id,
                        &ctx.lang.monitor_disk_alert(pct as u8, cfg.disk_warn)).await;
                    disk_st.in_alert = true;
                    disk_st.last_alert = Some(Instant::now());
                }
                Some(AlertAction::Recover) => {
                    fire(&ctx, super_id,
                        &ctx.lang.monitor_disk_recover(pct as u8)).await;
                    disk_st.in_alert = false;
                    disk_st.last_alert = None;
                }
                None => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_alert_first_crossing() {
        let state = AlertState::default();
        assert_eq!(should_alert(&state, true, 1800), Some(AlertAction::Alert));
    }

    #[test]
    fn test_no_alert_below_threshold() {
        let state = AlertState::default();
        assert_eq!(should_alert(&state, false, 1800), None);
    }

    #[test]
    fn test_no_alert_when_in_alert_and_recent() {
        let state = AlertState {
            in_alert: true,
            last_alert: Some(Instant::now()),
        };
        assert_eq!(should_alert(&state, true, 1800), None);
    }

    #[test]
    fn test_recover_when_was_in_alert() {
        let state = AlertState {
            in_alert: true,
            last_alert: Some(Instant::now()),
        };
        assert_eq!(should_alert(&state, false, 1800), Some(AlertAction::Recover));
    }

    #[test]
    fn test_remind_when_elapsed() {
        let state = AlertState {
            in_alert: true,
            last_alert: Some(Instant::now() - Duration::from_secs(10)),
        };
        assert_eq!(should_alert(&state, true, 5), Some(AlertAction::Alert));
    }

    #[test]
    fn test_no_recover_when_already_normal() {
        let state = AlertState::default();
        assert_eq!(should_alert(&state, false, 1800), None);
    }
}
