mod album;
mod artist;
mod cover_art;
pub mod proxy;
mod search;
mod song;
mod star;
mod stream;

use std::sync::Arc;

use anyhow::Result;
use axum::extract::State;
use axum::routing::get;
use axum::{Extension, Router};
use reqwest::Method;
use tower_http::cors::{Any, CorsLayer};
use tower_real_ip::{RealIpLayer, TrustedProxyConfig};

use crate::AppState;
use crate::middleware::{AuthMiddlewareLayer, SubsonicClient};
use crate::models::SubsonicRespFormat;

pub type LydianState = State<Arc<AppState>>;
pub type SubsonicExt<'sc> = Extension<Arc<SubsonicClient<'sc>>>;

pub struct AppRouter {
    _state: Arc<AppState>,
}

impl AppRouter {
    /*
     * Initializes the router and sets up the necessary state
     */
    pub async fn init(state: Arc<AppState>) -> Result<Router> {
        let mut subsonic_client = SubsonicClient::new(
            state.http_client.clone(),
            &state.config.subsonic.url,
            state.config.subsonic.username.clone(),
            state.config.subsonic.password.clone(),
        )?;

        subsonic_client.init().await?;

        let client = Arc::new(subsonic_client);

        let cors = CorsLayer::new()
            .allow_origin(Any)
            .allow_methods([
                Method::GET,
                Method::POST,
                Method::PUT,
                Method::DELETE,
                Method::OPTIONS,
            ])
            .allow_headers([axum::http::header::CONTENT_TYPE, axum::http::header::AUTHORIZATION]);

        let proxy_config = TrustedProxyConfig::new(state.config.server.trusted_proxies.clone());

        /*
         * It is really important in what order the routes are defined.
         *
         * FIRST we define the routes we want to handle ourselves,
         * Then we define the fallback route.
         *
         * At last put the state, which the previous routes will depend, and finally
         * the middlewares.
         *
         * Generally, What's on the bottom wraps what's on the top!!
         */
        let fallback_client = client.clone();
        let router = Router::new()
            /* Album */
            .route("/rest/getAlbum", get(album::get_album))
            .route("/rest/getAlbumInfo2", get(album::get_album_info))
            /* Artist */
            .route("/rest/getArtist", get(artist::get_artist))
            .route("/rest/getArtistInfo", get(artist::get_artist_info))
            .route("/rest/getArtist.view", get(artist::get_artist))
            .route("/rest/getArtistInfo.view", get(artist::get_artist_info))
            /* Song */
            .route("/rest/getSong", get(song::get_song))
            .route("/rest/getSong.view", get(song::get_song))
            .route("/rest/getTopSongs", get(song::get_top_songs))
            .route("/rest/getTopSongs.view", get(song::get_top_songs))
            /* Services */
            .route("/rest/stream", get(stream::stream_media))
            .route("/rest/search3", get(search::search3))
            /* Misc */
            .route("/rest/getCoverArt", get(cover_art::get_cover_art))
            .route("/rest/getCoverArt.view", get(cover_art::get_cover_art))
            .route("/rest/star", get(star::star))
            .route("/rest/star.view", get(star::star))
            .fallback(move |format: SubsonicRespFormat, req| {
                proxy::subsonic_proxy(format, fallback_client, req)
            })
            .layer(AuthMiddlewareLayer::new(state.clone()))
            .layer(RealIpLayer::new(proxy_config))
            .with_state(state)
            .layer(Extension(client))
            .layer(cors);

        Ok(router)
    }
}
