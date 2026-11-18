//! axum Router setup for the DLNA HTTP service.
//!
//! Exposes 4 route families :
//!   - `GET  /description.xml`           (device description)
//!   - `GET  /service/{name}.xml`        (service description)
//!   - `POST /control/{service}`         (SOAP control endpoint)
//!   - `GET  /media/{id}` + `/cover/{id}` (audio + image streaming)

use std::net::Ipv4Addr;
use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use crate::core::dlna_server::config::DlnaConfig;
use crate::core::dlna_server::http::{control, description, media};
use crate::core::dlna_server::library::provider::LibraryProvider;

/// State shared with every handler via axum's `with_state`.
///
/// `local_ip` is detected once at server start and embedded in DIDL `<res>`
/// URLs ; we don't re-detect on every SOAP request.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<DlnaConfig>,
    pub provider: Arc<dyn LibraryProvider>,
    pub local_ip: Ipv4Addr,
}

impl AppState {
    /// Absolute base URL used to build media stream links in DIDL responses.
    pub fn server_base_url(&self) -> String {
        format!("http://{}:{}", self.local_ip, self.config.port)
    }

    /// Base des liens donnés à un appareil : l'adresse qu'il a lui-même jointe (`Host`),
    /// donc celle de la bonne carte réseau. Sans en-tête valable, l'adresse détectée.
    pub fn base_url_for(&self, host: Option<&str>) -> String {
        base_from_host(host, self.config.port).unwrap_or_else(|| self.server_base_url())
    }
}

/// Build the full axum Router.
///
/// Note: axum's path matcher doesn't allow mixing a `{param}` with a literal
/// (e.g. `.xml`) in the same path segment. So we use a single `{name}`
/// parameter and the handler strips the `.xml` suffix itself.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(description::status_page))
        .route("/description.xml", get(description::device_description))
        .route("/rootDesc.xml", get(description::device_description))
        .route("/DeviceDescription.xml", get(description::device_description))
        .route("/service/{name}", get(description::service_description))
        .route("/control/{service}", post(control::soap_control))
        .route("/media/{id}", get(media::stream_media))
        .route("/cover/{id}", get(media::stream_cover))
        .with_state(state)
}

/// `http://<Host>` si l'en-tête est une adresse sage (lettres, chiffres, `.:-[]`), port ajouté s'il manque.
fn base_from_host(host: Option<&str>, port: u16) -> Option<String> {
    let h = host?.trim();
    if h.is_empty() || h.len() >= 256 || !h.chars().all(|c| c.is_ascii_alphanumeric() || ".:-[]".contains(c)) {
        return None;
    }
    Some(if h.contains(':') { format!("http://{h}") } else { format!("http://{h}:{port}") })
}

#[cfg(test)]
mod host_tests {
    use super::base_from_host;

    #[test]
    fn links_follow_the_address_the_device_reached() {
        assert_eq!(base_from_host(Some("10.0.0.24:8200"), 8200).as_deref(), Some("http://10.0.0.24:8200"));
        assert_eq!(base_from_host(Some("pc-salon"), 8200).as_deref(), Some("http://pc-salon:8200"));
        // Rien d'exotique dans les liens générés.
        assert_eq!(base_from_host(Some("evil\"><x>"), 8200), None);
        assert_eq!(base_from_host(Some(""), 8200), None);
        assert_eq!(base_from_host(None, 8200), None);
    }

    #[tokio::test]
    async fn root_and_description_aliases_return_ok() {
        use std::path::PathBuf;
        use async_trait::async_trait;
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;
        use crate::core::dlna_server::error::DlnaError;
        use crate::core::dlna_server::library::provider::{CoverKind, DlnaAlbum, DlnaArtist, DlnaFolder, DlnaLibrary, DlnaTrack, FolderEntries, LibraryProvider};

        struct Dummy;
        #[async_trait]
        impl LibraryProvider for Dummy {
            async fn list_libraries(&self) -> Result<Vec<DlnaLibrary>, DlnaError> { Ok(vec![]) }
            async fn list_artists(&self, _: i64) -> Result<Vec<DlnaArtist>, DlnaError> { Ok(vec![]) }
            async fn list_albums(&self, _: i64) -> Result<Vec<DlnaAlbum>, DlnaError> { Ok(vec![]) }
            async fn list_albums_by_artist(&self, _: i64, _: &str) -> Result<Vec<DlnaAlbum>, DlnaError> { Ok(vec![]) }
            async fn list_tracks_by_album(&self, _: i64, _: &str) -> Result<Vec<DlnaTrack>, DlnaError> { Ok(vec![]) }
            async fn track_path(&self, _: &str) -> Result<PathBuf, DlnaError> { Ok(PathBuf::new()) }
            async fn cover_path(&self, _: CoverKind, _: &str) -> Result<PathBuf, DlnaError> { Ok(PathBuf::new()) }
            async fn list_root_folders(&self, _: i64) -> Result<Vec<DlnaFolder>, DlnaError> { Ok(vec![]) }
            async fn list_folder_entries(&self, _: i64, _: &str) -> Result<FolderEntries, DlnaError> {
                Ok(FolderEntries { subfolders: vec![], tracks: vec![] })
            }
        }

        let app = super::build_router(super::AppState {
            config: std::sync::Arc::new(super::DlnaConfig::default()),
            provider: std::sync::Arc::new(Dummy),
            local_ip: std::net::Ipv4Addr::new(127, 0, 0, 1),
        });

        for uri in ["/", "/description.xml", "/rootDesc.xml", "/DeviceDescription.xml"] {
            let resp = app
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "failed on {uri}");
        }
    }
}
