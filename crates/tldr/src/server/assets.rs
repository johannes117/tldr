use axum::{
    body::Body,
    extract::State,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

use super::ServerCtx;

#[derive(RustEmbed)]
#[folder = "../../ui/dist"]
struct Assets;

pub async fn handler(State(ctx): State<ServerCtx>, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let candidate = if path.is_empty() { "index.html" } else { path };
    match Assets::get(candidate) {
        Some(f) => serve(candidate, f.data.into_owned(), &ctx),
        None => match Assets::get("index.html") {
            Some(f) => serve("index.html", f.data.into_owned(), &ctx),
            None => (StatusCode::NOT_FOUND, "not found").into_response(),
        },
    }
}

fn serve(path: &str, data: Vec<u8>, ctx: &ServerCtx) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let body = if path == "index.html" {
        let s = String::from_utf8_lossy(&data).replace("__TLDR_CSRF__", ctx.csrf.as_str());
        Body::from(s.into_bytes())
    } else {
        Body::from(data)
    };
    Response::builder()
        .header(header::CONTENT_TYPE, mime.as_ref())
        .body(body)
        .unwrap()
}
