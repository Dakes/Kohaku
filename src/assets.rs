//! Static assets, embedded and served under content-hashed names (host-routing: Hashed
//! static assets on both routers; change foundation D22).

use std::sync::OnceLock;

use axum::extract::Path;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use sha2::{Digest, Sha256};

use crate::routing::build::HashedAsset;

/// An embedded file.
pub struct Asset {
    /// The file's name in `static/`.
    pub name: &'static str,
    pub content_type: &'static str,
    pub bytes: &'static [u8],
}

/// Every asset a release build serves.
pub const ASSETS: &[Asset] = &[Asset {
    name: "kohaku.css",
    content_type: "text/css",
    bytes: include_bytes!("../static/kohaku.css"),
}];

struct Hashed {
    /// `kohaku.<16 hex digits>.css`.
    file: String,
    path: String,
    asset: &'static Asset,
}

fn registry() -> &'static [Hashed] {
    static REGISTRY: OnceLock<Vec<Hashed>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        ASSETS
            .iter()
            .map(|asset| {
                let digest = Sha256::digest(asset.bytes);
                let hash: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
                let (stem, extension) = asset
                    .name
                    .rsplit_once('.')
                    .expect("asset names have an extension");
                let file = format!("{stem}.{hash}.{extension}");
                let path = format!("/static/{file}");
                Hashed { file, path, asset }
            })
            .collect()
    })
}

/// The hashed path of the asset `name` (a file in `static/`), for templates.
pub fn path(name: &str) -> &'static str {
    registry()
        .iter()
        .find(|hashed| hashed.asset.name == name)
        .map(|hashed| hashed.path.as_str())
        .expect("every referenced asset is embedded")
}

/// Answers only from the registry: no name, stale hash or traversal reaches a file.
pub async fn serve(Path(file): Path<String>) -> Response {
    let Some(hashed) = registry().iter().find(|hashed| hashed.file == file) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(body) = crate::dev::asset_bytes(hashed.asset) else {
        tracing::error!("cannot read a static asset");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let mut response = (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static(hashed.asset.content_type),
        )],
        body,
    )
        .into_response();
    response.extensions_mut().insert(HashedAsset);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_carry_sixteen_hex_digits_of_the_hash() {
        let css = path("kohaku.css");
        let hash = css
            .strip_prefix("/static/kohaku.")
            .and_then(|rest| rest.strip_suffix(".css"))
            .unwrap();
        assert_eq!(hash.len(), 16);
        assert!(
            hash.bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        );
    }
}
