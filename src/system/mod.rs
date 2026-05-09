use crate::error::BotError;
use crate::i18n::Lang;
use std::collections::HashMap;
use std::time::Duration;
use tokio::process::Command;
use tracing::{info, warn};

pub async fn status(lang: Lang) -> Result<String, BotError> {
    let mut lines: Vec<String> = Vec::new();

    // ── Hostname + Uptime ────────────────────────────────────────────────────
    let hostname =
        std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_else(|_| "unknown\n".into());
    let hostname = hostname.trim();

    let uptime_str = {
        let raw = std::fs::read_to_string("/proc/uptime").map_err(BotError::Io)?;
        let secs = raw
            .split_whitespace()
            .next()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0) as u64;
        let days = secs / 86400;
        let hours = (secs % 86400) / 3600;
        let mins = (secs % 3600) / 60;
        if days > 0 {
            format!(
                "{}{} {}{} {}{}",
                days,
                lang.uptime_days(),
                hours,
                lang.uptime_hours(),
                mins,
                lang.uptime_mins()
            )
        } else if hours > 0 {
            format!(
                "{}{} {}{}",
                hours,
                lang.uptime_hours(),
                mins,
                lang.uptime_mins()
            )
        } else {
            format!("{}{}", mins, lang.uptime_mins())
        }
    };

    // ── Load averages ────────────────────────────────────────────────────────
    let load_raw = std::fs::read_to_string("/proc/loadavg").map_err(BotError::Io)?;
    let mut load_parts = load_raw.split_whitespace();
    let la1 = load_parts.next().unwrap_or("?");
    let la5 = load_parts.next().unwrap_or("?");
    let la15 = load_parts.next().unwrap_or("?");

    // ── CPU temperature (/sys/class/thermal/) ────────────────────────────────
    let cpu_temp: Option<i32> = (|| {
        let entries = std::fs::read_dir("/sys/class/thermal/").ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.file_name()?.to_str()?.starts_with("thermal_zone") {
                let t: i32 = std::fs::read_to_string(path.join("temp"))
                    .ok()?
                    .trim()
                    .parse()
                    .ok()?;
                if t > 1000 {
                    return Some(t / 1000);
                }
            }
        }
        None
    })();

    // First line: hostname, uptime, temp, load
    lines.push(format!("🖥 {}  |  uptime: {}", hostname, uptime_str));
    match cpu_temp {
        Some(t) => lines.push(format!(
            "🌡 CPU: {}°C  |  load: {}  {}  {} {}",
            t,
            la1,
            la5,
            la15,
            lang.load_suffix()
        )),
        None => lines.push(format!(
            "📊 Load: {}  {}  {} {}",
            la1,
            la5,
            la15,
            lang.load_suffix()
        )),
    }

    // ── RAM (/proc/meminfo) ──────────────────────────────────────────────────
    {
        let meminfo = std::fs::read_to_string("/proc/meminfo").map_err(BotError::Io)?;
        let mem: HashMap<&str, u64> = meminfo
            .lines()
            .filter_map(|l| {
                let mut p = l.splitn(2, ':');
                let k = p.next()?.trim();
                let v = p.next()?.split_whitespace().next()?.parse().ok()?;
                Some((k, v))
            })
            .collect();
        let total = mem.get("MemTotal").copied().unwrap_or(1);
        let avail = mem.get("MemAvailable").copied().unwrap_or(0);
        let used = total.saturating_sub(avail);
        let pct = used * 100 / total;
        let u = lang.ram_unit();
        lines.push(format!(
            "💾 RAM: {}{u} / {}{u}  ({}%)",
            used / 1024,
            total / 1024,
            pct
        ));
    }

    // ── Disk (df -B1 /) ──────────────────────────────────────────────────────
    {
        let out = tokio::time::timeout(
            Duration::from_secs(5),
            Command::new("df").args(["-B1", "/"]).output(),
        )
        .await;
        if let Ok(Ok(output)) = out {
            let s = String::from_utf8_lossy(&output.stdout);
            if let Some(line) = s.lines().nth(1) {
                let mut cols = line.split_whitespace();
                let _fs = cols.next();
                let total = cols.next().and_then(|v| v.parse::<u64>().ok());
                let used = cols.next().and_then(|v| v.parse::<u64>().ok());
                let avail = cols.next().and_then(|v| v.parse::<u64>().ok());
                if let (Some(t), Some(u), Some(a)) = (total, used, avail) {
                    let pct = (u * 100).checked_div(t).unwrap_or(0);
                    let du = lang.disk_unit();
                    lines.push(format!(
                        "💿 {}: {}{du} / {}{du}  ({}%)  {}: {}{}",
                        lang.disk_label(),
                        u / 1_073_741_824,
                        t / 1_073_741_824,
                        pct,
                        lang.disk_free(),
                        a / 1_073_741_824,
                        du,
                    ));
                }
            }
        }
    }

    // ── Network interface (auto via /proc/net/route) ─────────────────────────
    {
        let iface: Option<String> = (|| {
            let route = std::fs::read_to_string("/proc/net/route").ok()?;
            // default route: Destination=00000000, Flags contains 0003 (UG)
            for line in route.lines().skip(1) {
                let mut cols = line.split_whitespace();
                let iface = cols.next()?;
                let dest = cols.next()?;
                let _gw = cols.next()?;
                let flags = u32::from_str_radix(cols.next()?, 16).ok()?;
                // flags 0x0003 = RTF_UP | RTF_GATEWAY (default route)
                if dest == "00000000" && flags & 0x0003 == 0x0003 {
                    return Some(iface.to_string());
                }
            }
            None
        })();

        if let Some(ref iface) = iface {
            let net = std::fs::read_to_string("/proc/net/dev").ok();
            if let Some(content) = net {
                for line in content.lines() {
                    let trimmed = line.trim_start();
                    if trimmed.starts_with(iface.as_str()) {
                        let parts: Vec<&str> = trimmed
                            .trim_start_matches(iface.as_str())
                            .trim_start_matches(':')
                            .split_whitespace()
                            .collect();
                        // fields: rx_bytes, rx_packets, rx_errs, rx_drop, rx_fifo, rx_frame,
                        //         rx_compressed, rx_multicast, tx_bytes, ...
                        if parts.len() >= 9 {
                            let rx: u64 = parts[0].parse().unwrap_or(0);
                            let tx: u64 = parts[8].parse().unwrap_or(0);
                            let rx_gb = rx as f64 / 1_073_741_824.0;
                            let tx_gb = tx as f64 / 1_073_741_824.0;
                            lines.push(format!(
                                "🌐 {}: ↓{:.1} {}  ↑{:.1} {}  ({})",
                                lang.traffic_label(),
                                rx_gb,
                                lang.traffic_unit(),
                                tx_gb,
                                lang.traffic_unit(),
                                iface
                            ));
                        }
                        break;
                    }
                }
            }
        }
    }

    // ── Updates (auto-detect PM, sync + count, 60s timeout) ─────────────────
    {
        fn cmd_exists(name: &str) -> bool {
            std::env::var_os("PATH")
                .map(|paths| std::env::split_paths(&paths).any(|d| d.join(name).is_file()))
                .unwrap_or(false)
        }

        enum Pm {
            Checkupdates,
            Pacman,
            Apt,
            Dnf,
            Yum,
        }

        let pm = if cmd_exists("checkupdates") {
            Some(Pm::Checkupdates)
        } else if cmd_exists("pacman") {
            Some(Pm::Pacman)
        } else if cmd_exists("apt-get") {
            Some(Pm::Apt)
        } else if cmd_exists("dnf") {
            Some(Pm::Dnf)
        } else if cmd_exists("yum") {
            Some(Pm::Yum)
        } else {
            None
        };

        let upd_line = match pm {
            None => lang.pkg_not_found().to_string(),
            Some(pm) => {
                let pm_name = match pm {
                    Pm::Checkupdates => "checkupdates",
                    Pm::Pacman => "pacman",
                    Pm::Apt => "apt",
                    Pm::Dnf => "dnf",
                    Pm::Yum => "yum",
                };
                info!(pm = pm_name, "status: checking updates");
                let result = tokio::time::timeout(Duration::from_secs(60), async move {
                    let count: usize = match pm {
                        // checkupdates синхронизирует во временную БД — root не нужен
                        Pm::Checkupdates => {
                            let out = Command::new("checkupdates").output().await
                                .map_err(|e| e.to_string())?;
                            // exit 0 = есть обновления, exit 2 = нет, exit 1 = ошибка
                            String::from_utf8_lossy(&out.stdout)
                                .lines().filter(|l| !l.is_empty()).count()
                        }
                        Pm::Pacman => {
                            let sync = Command::new("sudo")
                                .args(["pacman", "-Sy", "--noconfirm", "-q"])
                                .stdout(std::process::Stdio::null())
                                .stderr(std::process::Stdio::null())
                                .status().await.map_err(|e| e.to_string())?;
                            if !sync.success() {
                                warn!(exit = ?sync.code(), "status: sudo pacman -Sy failed — check sudoers for tgbot");
                                return Err(format!("sync failed (exit {:?}), check sudoers", sync.code()));
                            }
                            let out = Command::new("pacman").args(["-Qu"]).output().await
                                .map_err(|e| e.to_string())?;
                            String::from_utf8_lossy(&out.stdout)
                                .lines().filter(|l| !l.is_empty()).count()
                        }
                        Pm::Apt => {
                            let sync = Command::new("sudo")
                                .args(["apt-get", "update", "-qq"])
                                .stdout(std::process::Stdio::null())
                                .stderr(std::process::Stdio::null())
                                .status().await.map_err(|e| e.to_string())?;
                            if !sync.success() {
                                warn!(exit = ?sync.code(), "status: sudo apt-get update failed — check sudoers for tgbot");
                                return Err(format!("sync failed (exit {:?}), check sudoers", sync.code()));
                            }
                            let out = Command::new("apt")
                                .args(["list", "--upgradable"])
                                .stderr(std::process::Stdio::null())
                                .output().await.map_err(|e| e.to_string())?;
                            // первая строка — заголовок "Listing..."
                            String::from_utf8_lossy(&out.stdout)
                                .lines().skip(1).filter(|l| !l.is_empty()).count()
                        }
                        Pm::Dnf => {
                            // exit 100 = есть обновления, 0 = нет, иначе ошибка
                            let out = Command::new("sudo")
                                .args(["dnf", "check-update", "-q"])
                                .output().await.map_err(|e| e.to_string())?;
                            match out.status.code() {
                                Some(100) => String::from_utf8_lossy(&out.stdout)
                                    .lines()
                                    .filter(|l| !l.is_empty() && !l.starts_with("Last metadata"))
                                    .count(),
                                Some(0) => 0,
                                code => {
                                    warn!(exit = ?code, "status: sudo dnf check-update failed — check sudoers for tgbot");
                                    return Err(format!("dnf failed (exit {:?}), check sudoers", code));
                                }
                            }
                        }
                        Pm::Yum => {
                            let out = Command::new("sudo")
                                .args(["yum", "check-update", "-q"])
                                .output().await.map_err(|e| e.to_string())?;
                            match out.status.code() {
                                Some(100) => String::from_utf8_lossy(&out.stdout)
                                    .lines()
                                    .filter(|l| !l.is_empty() && !l.starts_with("Last metadata"))
                                    .count(),
                                Some(0) => 0,
                                code => {
                                    warn!(exit = ?code, "status: sudo yum check-update failed — check sudoers for tgbot");
                                    return Err(format!("yum failed (exit {:?}), check sudoers", code));
                                }
                            }
                        }
                    };
                    Ok::<usize, String>(count)
                }).await;

                match result {
                    Ok(Ok(0)) => lang.pkg_no_updates().to_string(),
                    Ok(Ok(n)) => lang.pkg_updates(n),
                    Ok(Err(e)) => lang.pkg_error(e),
                    Err(_) => {
                        warn!("status: update check timed out after 60s");
                        lang.pkg_timeout().to_string()
                    }
                }
            }
        };
        lines.push(upd_line);
    }

    // ── Failed services ──────────────────────────────────────────────────────
    {
        let out = tokio::time::timeout(
            Duration::from_secs(5),
            Command::new("systemctl")
                .args(["list-units", "--state=failed", "--no-legend", "--no-pager"])
                .output(),
        )
        .await;
        let failed: Vec<String> = match out {
            Ok(Ok(output)) => String::from_utf8_lossy(&output.stdout)
                .lines()
                .filter_map(|l| l.split_whitespace().next().map(str::to_string))
                .filter(|s| !s.is_empty())
                .collect(),
            _ => vec![],
        };

        if failed.is_empty() {
            lines.push(lang.services_ok().to_string());
        } else {
            lines.push(lang.services_failed(failed.len()));
            for svc in &failed {
                lines.push(format!("  • {}", svc));
            }
        }
    }

    Ok(lines.join("\n"))
}
