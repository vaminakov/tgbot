use std::net::SocketAddr;
use std::sync::Arc;

use axum::http::Request;
use axum::{
    body::Bytes,
    extract::{ConnectInfo, State},
    http::StatusCode,
    middleware::{self, Next},
    response::IntoResponse,
    routing::post,
    Router,
};
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as AutoBuilder;
use hyper_util::service::TowerToHyperService;
use tower_service::Service;
use tracing::{info, warn};

use crate::bot::security::IpWhitelist;
use crate::bot::{dispatch, BotContext};
use crate::telegram::types::Update;

#[derive(Clone)]
struct WebhookState {
    ctx: Arc<BotContext>,
    whitelist: Arc<IpWhitelist>,
}

async fn ip_guard(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<WebhookState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> impl IntoResponse {
    if !state.whitelist.allows(addr.ip()) {
        warn!(ip = %addr.ip(), "webhook rejected — non-whitelisted IP");
        return StatusCode::FORBIDDEN.into_response();
    }
    next.run(req).await.into_response()
}

async fn webhook_handler(State(state): State<WebhookState>, body: Bytes) -> StatusCode {
    match serde_json::from_slice::<Update>(&body) {
        Ok(update) => {
            let ctx = Arc::clone(&state.ctx);
            tokio::spawn(async move { dispatch(&update, &ctx).await });
            StatusCode::OK
        }
        Err(e) => {
            warn!(%e, "failed to parse webhook body — possible direct access");
            if let Some(super_id) = state.ctx.config.super_admin_id() {
                let tg = Arc::clone(&state.ctx.tg);
                tokio::spawn(async move {
                    let _ = tg
                        .send_message(
                            super_id,
                            "Несанкционированное обращение к webhook (невалидный Update).",
                            None,
                            false,
                        )
                        .await;
                });
            }
            StatusCode::OK // always 200 to Telegram
        }
    }
}

/// Start the webhook server. Supports TCP and Unix socket.
pub async fn serve(
    ctx: Arc<BotContext>,
    bind: &str,
    webhook_path: &str,
    whitelist: IpWhitelist,
) -> Result<(), Box<dyn std::error::Error>> {
    let state = WebhookState {
        ctx,
        whitelist: Arc::new(whitelist),
    };

    if let Some(path) = bind.strip_prefix("unix:") {
        // Unix socket — no IP check (whitelist irrelevant for local socket)
        let app = Router::new()
            .route(webhook_path, post(webhook_handler))
            .with_state(state);
        info!(socket = path, "webhook listening on Unix socket");
        // Remove stale socket file from a previous unclean exit.
        if std::path::Path::new(path).exists() {
            std::fs::remove_file(path)?;
        }
        let listener = tokio::net::UnixListener::bind(path)?;
        // Allow nginx (or any local process) to connect; auth is at app layer.
        std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o666))?;
        // axum::serve does not support UnixListener; drive connections manually.
        let mut make_svc = app.into_make_service();
        loop {
            let (stream, _addr) = listener.accept().await?;
            let io = TokioIo::new(stream);
            // IntoMakeService::poll_ready is always Ready, so call directly.
            let svc = Service::<()>::call(&mut make_svc, ())
                .await
                .map_err(|e| format!("make_service call error: {e}"))?;
            tokio::spawn(async move {
                if let Err(e) = AutoBuilder::new(TokioExecutor::new())
                    .serve_connection(io, TowerToHyperService::new(svc))
                    .await
                {
                    warn!(err = %e, "Unix socket connection error");
                }
            });
        }
    } else {
        // TCP — attach IP whitelist middleware
        let app = Router::new()
            .route(webhook_path, post(webhook_handler))
            .layer(middleware::from_fn_with_state(state.clone(), ip_guard))
            .with_state(state);
        let addr: SocketAddr = bind.parse()?;
        info!(%addr, "webhook listening on TCP");
        let listener = tokio::net::TcpListener::bind(addr).await?;
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await?;
    }
    Ok(())
}
