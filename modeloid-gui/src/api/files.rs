//! Plain GET routes for dataset images, added to the Dioxus server in
//! `main.rs`. Server functions return data; these return files, so `<img src>`
//! can use them directly.

use dioxus::server::axum::{
    extract::Path,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};

/// Longest side of gallery previews, in pixels.
const THUMB_SIZE: u32 = 320;

pub fn routes() -> Router {
    Router::new()
        .route("/files/thumb/{id}/{index}", get(|Path((id, index)): Path<(u32, usize)>| serve(id, index, Some(THUMB_SIZE))))
        .route("/files/image/{id}/{index}", get(|Path((id, index)): Path<(u32, usize)>| serve(id, index, None)))
}

async fn serve(id: u32, index: usize, thumb: Option<u32>) -> Response {
    let found = tokio::task::spawn_blocking(move || {
        let path = crate::api::explore::image_file(id, index, thumb)?;
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        Ok::<_, String>((path, bytes))
    })
    .await;

    match found {
        Ok(Ok((path, bytes))) => {
            let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            let content_type = match ext.as_str() {
                "jpg" | "jpeg" => "image/jpeg",
                "png" => "image/png",
                "webp" => "image/webp",
                "bmp" => "image/bmp",
                "tif" | "tiff" => "image/tiff",
                _ => "application/octet-stream",
            };
            // Indexes can point at another image once the dataset changes; keep caching short.
            ([(header::CONTENT_TYPE, content_type), (header::CACHE_CONTROL, "private, max-age=60")], bytes).into_response()
        }
        Ok(Err(message)) => (StatusCode::NOT_FOUND, message).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
