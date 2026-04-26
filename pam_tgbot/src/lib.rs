#![allow(dead_code)]

mod config;
mod ipc;
mod telegram;

use libc::{c_char, c_int, c_uint, c_void};
use std::ffi::{CStr, CString};
use std::ptr;

// ── PAM handle (opaque) ───────────────────────────────────────────────────────
pub enum PamHandle {}
pub type PamHandleT = *mut PamHandle;

// ── PAM item types ────────────────────────────────────────────────────────────
const PAM_USER:  c_int = 2;
const PAM_TTY:   c_int = 3;
const PAM_RHOST: c_int = 4;
const PAM_CONV:  c_int = 5;

// ── PAM return codes ──────────────────────────────────────────────────────────
const PAM_SUCCESS:  c_int = 0;
const PAM_AUTH_ERR: c_int = 7;
const PAM_IGNORE:   c_int = 25;

// ── Conversation message styles ───────────────────────────────────────────────
const PAM_TEXT_INFO: c_int = 4;
const PAM_ERROR_MSG: c_int = 3;

#[repr(C)]
struct PamMessage {
    msg_style: c_int,
    msg:       *const c_char,
}

#[repr(C)]
struct PamResponse {
    resp:         *mut c_char,
    resp_retcode: c_int,
}

type PamConvFn = unsafe extern "C" fn(
    c_int,
    *const *const PamMessage,
    *mut *mut PamResponse,
    *mut c_void,
) -> c_int;

#[repr(C)]
struct PamConv {
    conv:        PamConvFn,
    appdata_ptr: *mut c_void,
}

// ── libpam imports ────────────────────────────────────────────────────────────
#[link(name = "pam")]
extern "C" {
    fn pam_get_item(pamh: PamHandleT, item_type: c_int, item: *mut *const c_void) -> c_int;
    fn pam_get_user(pamh: PamHandleT, user: *mut *const c_char, prompt: *const c_char) -> c_int;
    fn pam_getenv(pamh: PamHandleT, name: *const c_char) -> *const c_char;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn get_item_str(pamh: PamHandleT, item_type: c_int) -> Option<String> {
    let mut item: *const c_void = ptr::null();
    unsafe {
        if pam_get_item(pamh, item_type, &mut item) != PAM_SUCCESS || item.is_null() {
            return None;
        }
        CStr::from_ptr(item as *const c_char)
            .to_str()
            .ok()
            .map(String::from)
    }
}

fn get_user(pamh: PamHandleT) -> Option<String> {
    let mut user: *const c_char = ptr::null();
    unsafe {
        if pam_get_user(pamh, &mut user, ptr::null()) != PAM_SUCCESS || user.is_null() {
            return None;
        }
        CStr::from_ptr(user).to_str().ok().map(String::from)
    }
}

fn get_env_str(pamh: PamHandleT, name: &str) -> Option<String> {
    let Ok(cname) = CString::new(name) else { return None };
    let ptr = unsafe { pam_getenv(pamh, cname.as_ptr()) };
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr).to_str().ok().map(String::from) }
}

/// Display a message on the user's terminal via the PAM conversation function.
fn conv_info(pamh: PamHandleT, text: &str, style: c_int) {
    let mut item: *const c_void = ptr::null();
    unsafe {
        if pam_get_item(pamh, PAM_CONV, &mut item) != PAM_SUCCESS || item.is_null() {
            return;
        }
        let conv = &*(item as *const PamConv);
        let Ok(cstr) = CString::new(text) else { return };
        let msg = PamMessage { msg_style: style, msg: cstr.as_ptr() };
        let msg_ptr: *const PamMessage = &msg;
        let msgs = [msg_ptr];
        let mut resp: *mut PamResponse = ptr::null_mut();
        let rc = (conv.conv)(1, msgs.as_ptr(), &mut resp, conv.appdata_ptr);
        // Free resp even on error — the conversation function may allocate it regardless of return code.
        if !resp.is_null() {
            if !(*resp).resp.is_null() {
                libc::free((*resp).resp as *mut c_void);
            }
            libc::free(resp as *mut c_void);
        }
        if rc != PAM_SUCCESS {
            return;
        }
    }
}

/// Returns true if the configured or detected language is Russian.
fn lang_is_ru(cfg: &config::LoadedCfg) -> bool {
    match cfg.pam.language.to_ascii_lowercase().as_str() {
        "ru" => true,
        "en" => false,
        _ => {
            for var in ["LC_ALL", "LANG"] {
                if let Ok(val) = std::env::var(var) {
                    let lo = val.to_ascii_lowercase();
                    if lo.starts_with("ru") { return true; }
                    if lo.starts_with("en") { return false; }
                }
            }
            false // default to English
        }
    }
}

/// Write a message to syslog AUTH facility — visible in `journalctl -t pam_tgbot`.
fn slog(priority: libc::c_int, msg: &str) {
    let Ok(cmsg) = CString::new(msg) else { return };
    unsafe {
        libc::openlog(
            b"pam_tgbot\0".as_ptr() as *const c_char,
            libc::LOG_NDELAY | libc::LOG_PID,
            libc::LOG_AUTH,
        );
        libc::syslog(priority, b"%s\0".as_ptr() as *const c_char, cmsg.as_ptr());
        libc::closelog();
    }
}

// ── PAM exports ───────────────────────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn pam_sm_authenticate(
    pamh:   PamHandleT,
    _flags: c_uint,
    _argc:  c_int,
    _argv:  *const *const c_char,
) -> c_int {
    let Some(cfg) = config::load(config::CONFIG_PATH) else {
        return PAM_IGNORE; // no config — not our problem
    };
    if !cfg.pam.two_factor_enabled {
        slog(libc::LOG_INFO,
             "two_factor_enabled=false, skipping 2FA (set it to true in [pam] config)");
        return PAM_IGNORE;
    }

    let ru = lang_is_ru(&cfg);

    let user  = get_user(pamh).unwrap_or_else(|| "unknown".to_string());
    let rhost = get_item_str(pamh, PAM_RHOST).unwrap_or_else(|| "unknown".to_string());

    // Rate limiting: reject if a 2FA request was made too recently for this user.
    if cfg.pam.two_factor_rate_limit_secs > 0 {
        let safe_user: String = user.chars()
            .map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '_' })
            .collect();
        let rate_file = format!("{}/rl_{}", ipc::IPC_DIR, safe_user);
        if let Ok(meta) = std::fs::metadata(&rate_file) {
            if let Ok(modified) = meta.modified() {
                let elapsed = modified.elapsed().unwrap_or(std::time::Duration::MAX);
                if elapsed.as_secs() < cfg.pam.two_factor_rate_limit_secs {
                    slog(libc::LOG_WARNING, &format!(
                        "2FA rate limit for user {} — {}s remaining",
                        user,
                        cfg.pam.two_factor_rate_limit_secs.saturating_sub(elapsed.as_secs())
                    ));
                    return PAM_AUTH_ERR;
                }
            }
        }
        // Touch the rate limit marker (best-effort)
        let _ = std::fs::write(&rate_file, "");
    }

    let Some(id) = ipc::gen_id() else {
        return PAM_AUTH_ERR;
    };

    let Ok(ipc_path) = ipc::create_pending(&id) else {
        // IPC dir not available — bot not running; fail open with a log entry
        slog(libc::LOG_ERR,
             "IPC dir /run/tgbot/pam unavailable — is tgbot.service running? Run: systemctl daemon-reload && systemctl restart tgbot");
        return PAM_IGNORE;
    };

    let msg = if ru {
        format!("🔐 Запрос 2FA: вход пользователя {} с {}\n\nАвторизовать?", user, rhost)
    } else {
        format!("🔐 2FA request: login by {} from {}\n\nAuthorise?", user, rhost)
    };
    let approve_data = format!("pam_approve:{}", id);
    let deny_data    = format!("pam_deny:{}", id);
    let buttons = [
        (if ru { "✅ Одобрить"   } else { "✅ Approve" }, approve_data.as_str()),
        (if ru { "❌ Отклонить" } else { "❌ Deny"    }, deny_data.as_str()),
    ];

    if telegram::send_required(
        &cfg.tg.api_base(),
        cfg.super_admin_id,
        &msg,
        &buttons,
        &cfg.tg.proxy,
    ).is_err() {
        let _ = std::fs::remove_file(&ipc_path);
        slog(libc::LOG_ERR, "failed to send 2FA request to Telegram — check bot token and network");
        conv_info(
            pamh,
            if ru { "Ошибка: не удалось отправить запрос 2FA." }
            else  { "Error: failed to send 2FA request." },
            PAM_ERROR_MSG,
        );
        return PAM_AUTH_ERR;
    }

    conv_info(
        pamh,
        if ru { "Ожидание подтверждения в Telegram..." }
        else  { "Waiting for Telegram confirmation..." },
        PAM_TEXT_INFO,
    );

    match ipc::poll_response(&ipc_path, cfg.pam.two_factor_timeout_secs) {
        Some(true)  => {
            slog(libc::LOG_INFO, &format!("2FA approved for user {user} from {rhost}"));
            PAM_SUCCESS
        }
        Some(false) => {
            slog(libc::LOG_WARNING, &format!("2FA denied for user {user} from {rhost}"));
            conv_info(
                pamh,
                if ru { "Доступ отклонён." } else { "Access denied." },
                PAM_ERROR_MSG,
            );
            PAM_AUTH_ERR
        }
        None => {
            slog(libc::LOG_WARNING, &format!("2FA timed out for user {user} from {rhost}"));
            conv_info(
                pamh,
                if ru { "Таймаут подтверждения 2FA." } else { "2FA confirmation timed out." },
                PAM_ERROR_MSG,
            );
            PAM_AUTH_ERR
        }
    }
}

#[no_mangle]
pub extern "C" fn pam_sm_setcred(
    _pamh:  PamHandleT,
    _flags: c_uint,
    _argc:  c_int,
    _argv:  *const *const c_char,
) -> c_int {
    PAM_IGNORE
}

#[no_mangle]
pub extern "C" fn pam_sm_open_session(
    pamh:   PamHandleT,
    _flags: c_uint,
    _argc:  c_int,
    _argv:  *const *const c_char,
) -> c_int {
    let Some(cfg) = config::load(config::CONFIG_PATH) else {
        return PAM_SUCCESS; // no config — proceed silently
    };
    if !cfg.pam.notify_login {
        return PAM_SUCCESS;
    }

    let ru = lang_is_ru(&cfg);

    let user  = get_user(pamh).unwrap_or_else(|| "unknown".to_string());
    let rhost = get_item_str(pamh, PAM_RHOST).unwrap_or_else(|| "unknown".to_string());
    let session_id = get_env_str(pamh, "XDG_SESSION_ID");

    if session_id.is_none() {
        slog(libc::LOG_WARNING,
             "XDG_SESSION_ID not set — ensure pam_systemd.so runs before pam_tgbot.so in session stack; 'Terminate session' button will be absent");
    }

    let msg = if ru {
        format!("🔑 Авторизован: {} с {}", user, rhost)
    } else {
        format!("🔑 Authorised: {} from {}", user, rhost)
    };

    let kill_data = session_id.as_ref().map(|id| format!("pam_kill:{}", id));
    // Validate rhost against safe charset (alphanumeric + ._:-).
    // PAM_RHOST is attacker-controlled (TCP source); embedding it unsanitized in
    // block_ip_cmd callback_data enables shell injection via {args}-style commands.
    let rhost_safe = !rhost.is_empty()
        && rhost != "unknown"
        && rhost.chars().all(|c| c.is_ascii_alphanumeric() || "._:-".contains(c));
    let block_data: Option<String> = if !cfg.pam.block_ip_cmd.is_empty() && rhost_safe {
        Some(format!("{} {}", cfg.pam.block_ip_cmd, rhost))
    } else {
        None
    };

    let mut buttons: Vec<(&str, &str)> = Vec::new();
    if let Some(ref kd) = kill_data {
        buttons.push((if ru { "🚫 Завершить сессию" } else { "🚫 Terminate session" }, kd.as_str()));
    }
    if let Some(ref bd) = block_data {
        buttons.push((if ru { "🛡 Блокировать IP" } else { "🛡 Block IP" }, bd.as_str()));
    }

    telegram::send(
        &cfg.tg.api_base(),
        cfg.super_admin_id,
        &msg,
        &buttons,
        &cfg.tg.proxy,
    );

    PAM_SUCCESS
}

#[no_mangle]
pub extern "C" fn pam_sm_close_session(
    _pamh:  PamHandleT,
    _flags: c_uint,
    _argc:  c_int,
    _argv:  *const *const c_char,
) -> c_int {
    PAM_IGNORE
}

#[no_mangle]
pub extern "C" fn pam_sm_acct_mgmt(
    _pamh:  PamHandleT,
    _flags: c_uint,
    _argc:  c_int,
    _argv:  *const *const c_char,
) -> c_int {
    PAM_IGNORE
}
