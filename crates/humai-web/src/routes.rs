use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::header;
use axum::response::{Html, IntoResponse, Json};
use axum::routing::get;

use humai_core::store::Store;
use humai_core::sync_ext::MutexExt;

use crate::status::SharedWebStatus;

#[derive(Clone)]
struct AppState {
    store: Arc<Store>,
    status: SharedWebStatus,
}

pub(crate) fn router(store: Arc<Store>, status: SharedWebStatus) -> Router {
    let state = AppState { store, status };
    Router::new()
        .route("/", get(index))
        .route("/app.js", get(app_js))
        .route("/style.css", get(style_css))
        .route("/api/status", get(get_status))
        .route("/api/messages", get(get_messages))
        .with_state(state)
}

// The dashboard is three small, dependency-free files with no build step —
// embedded at compile time so a distributed release binary never depends on
// a "web/" folder existing next to it at runtime (unlike assets/, nothing
// stages these into the dist folder today, and embedding avoids needing to).
async fn index() -> Html<&'static str> {
    Html(include_str!("../web/index.html"))
}

async fn app_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../web/app.js"),
    )
}

async fn style_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../web/style.css"),
    )
}

async fn get_status(State(state): State<AppState>) -> Json<crate::status::WebStatusSnapshot> {
    Json(state.status.lock_recover().clone())
}

#[derive(serde::Serialize)]
struct MessageView {
    from: Option<String>,
    content: String,
    source: &'static str,
    timestamp: String,
}

async fn get_messages(State(state): State<AppState>) -> Json<Vec<MessageView>> {
    Json(latest_room_messages(&state.store).unwrap_or_default())
}

/// Reads the most-recently-updated conversation's messages straight from the
/// same SQLite Store the desktop app writes to (WAL mode, so this concurrent
/// read never blocks the app's writes). No caching, no push — the dashboard
/// polls this endpoint on an interval, which is enough for a companion view.
fn latest_room_messages(store: &Store) -> Result<Vec<MessageView>, humai_core::store::StoreError> {
    let rooms = store.list_conversations(1)?;
    let Some(room) = rooms.first() else {
        return Ok(Vec::new());
    };
    let (messages, timestamps) = store.load_messages(&room.id)?;
    Ok(messages
        .into_iter()
        .zip(timestamps)
        .map(|(m, ts)| MessageView {
            from: m.from,
            content: m.content,
            source: m.source.as_db(),
            timestamp: ts,
        })
        .collect())
}
