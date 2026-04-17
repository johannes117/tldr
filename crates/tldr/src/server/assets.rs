use axum::{
    body::Body,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../ui/dist"]
struct Assets;

pub async fn handler(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let candidate = if path.is_empty() { "index.html" } else { path };
    match Assets::get(candidate) {
        Some(f) => serve(candidate, f.data.into_owned()),
        None => {
            // SPA fallback
            match Assets::get("index.html") {
                Some(f) => serve("index.html", f.data.into_owned()),
                None => (StatusCode::NOT_FOUND, "not found").into_response(),
            }
        }
    }
}

fn serve(path: &str, data: Vec<u8>) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    Response::builder()
        .header(header::CONTENT_TYPE, mime.as_ref())
        .body(Body::from(data))
        .unwrap()
}
