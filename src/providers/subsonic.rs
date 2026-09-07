use anyhow::Result;
use async_trait::async_trait;
use rand::RngExt;
use rand::distr::Alphanumeric;
use reqwest::header::{CONTENT_TYPE, RANGE};
use reqwest::{Client, Url};
use thiserror::Error as TError;

use crate::models::{
    AlbumInfo,
    AlbumWithSongsID3,
    ArtistInfo,
    ArtistWithAlbumsID3,
    Id,
    IdType,
    SearchResult,
    Song,
    SubsonicResponse,
    TagOrigin,
    TopSongs,
};
use crate::providers::{MusicProvider, SearchRequest};
use crate::services::{RemoteError, StreamMedia};

#[derive(TError, Debug)]
enum FederatedError {
    #[error("Unsupported url scheme for a federated server: {0}")]
    UnsupportedScheme(String),
    #[error("Upstream server answered getCoverArt with {0}")]
    NotAnImage(String),
}

pub struct SubsonicProvider {
    client: Client,
    base_url: Url,
    username: String,
    password: String,
    provider_id: String,
}

impl SubsonicProvider {
    pub fn new(
        client: Client,
        base_url: &str,
        username: &str,
        password: &str,
        provider_id: &str,
    ) -> Result<Self> {
        let parsed_url = Url::parse(base_url)?;
        if !matches!(parsed_url.scheme(), "http" | "https") {
            return Err(FederatedError::UnsupportedScheme(parsed_url.scheme().to_string()).into());
        }

        Ok(Self {
            client,
            base_url: parsed_url,
            username: username.to_string(),
            password: password.to_string(),
            provider_id: provider_id.to_string(),
        })
    }

    fn build_auth_query(&self) -> Vec<(&'static str, String)> {
        let salt: String =
            rand::rng().sample_iter(&Alphanumeric).take(12).map(char::from).collect();

        let mut context = md5::Context::new();
        context.consume(self.password.as_bytes());
        context.consume(salt.as_bytes());
        let token = format!("{:x}", context.finalize());

        vec![
            ("u", self.username.clone()),
            ("t", token),
            ("s", salt),
            ("v", "1.16.1".to_string()),
            ("c", "lydian_fed".to_string()),
            ("f", "json".to_string()),
        ]
    }
}

#[async_trait]
impl MusicProvider for SubsonicProvider {
    fn id(&self) -> &str {
        self.provider_id.as_str()
    }

    async fn get_artist<'a>(&self, id: &Id<'_>) -> Result<ArtistWithAlbumsID3<'a>> {
        let mut url = self.base_url.join("/rest/getArtist")?;

        url.query_pairs_mut().append_pair("id", id.key());

        let req = self.client.get(url).query(&self.build_auth_query());
        let res =
            req.send().await?.json::<SubsonicResponse<ArtistWithAlbumsID3<'static>>>().await?;

        let mut artist = res.into_answer()?;
        artist.tag_origin(self.id(), IdType::Artist);
        artist.base.starred = None;

        Ok(artist)
    }

    async fn get_artist_info<'a>(&self, id: &Id<'_>) -> Result<ArtistInfo<'a>> {
        let mut url = self.base_url.join("/rest/getArtistInfo")?;

        url.query_pairs_mut().append_pair("id", id.key());

        let req = self.client.get(url).query(&self.build_auth_query());
        let res = req.send().await?.json::<SubsonicResponse<ArtistInfo<'static>>>().await?;

        let mut info = res.into_answer()?;
        info.tag_origin(self.id(), IdType::Artist);

        Ok(info)
    }

    async fn get_album_info<'a>(&self, id: &Id<'_>) -> Result<AlbumInfo<'a>> {
        let mut url = self.base_url.join("/rest/getAlbumInfo2")?;

        url.query_pairs_mut().append_pair("id", id.key());

        let req = self.client.get(url).query(&self.build_auth_query());
        let res = req.send().await?.json::<SubsonicResponse<AlbumInfo<'static>>>().await?;

        Ok(res.into_answer()?)
    }

    async fn get_album<'a>(&self, id: &Id<'_>) -> Result<AlbumWithSongsID3<'a>> {
        let mut url = self.base_url.join("/rest/getAlbum")?;

        url.query_pairs_mut().append_pair("id", id.key());

        let req = self.client.get(url).query(&self.build_auth_query());
        let res = req.send().await?.json::<SubsonicResponse<AlbumWithSongsID3<'static>>>().await?;

        let mut album = res.into_answer()?;
        album.tag_origin(self.id(), IdType::Album);
        album.base.starred = None;
        album.base.played = None;
        album.song.iter_mut().for_each(|song| {
            song.suffix =
                Some(self.tagged_suffix(song.suffix.as_deref().unwrap_or_default()).into());
            song.starred = None;
            song.played = None;
        });

        Ok(album)
    }

    async fn get_song<'a>(&self, id: &Id<'_>) -> Result<Song<'a>> {
        let mut url = self.base_url.join("/rest/getSong")?;

        url.query_pairs_mut().append_pair("id", id.key());

        let req = self.client.get(url).query(&self.build_auth_query());
        let res = req.send().await?.json::<SubsonicResponse<Song<'static>>>().await?;

        let mut song = res.into_answer()?;
        song.tag_origin(self.id(), IdType::Song);
        song.starred = None;
        song.played = None;
        song.suffix = Some(self.tagged_suffix(song.suffix.as_deref().unwrap_or_default()).into());

        Ok(song)
    }

    async fn get_top_songs<'a>(&self, artist: &str, count: Option<u32>) -> Result<TopSongs<'a>> {
        let mut url = self.base_url.join("/rest/getTopSongs")?;

        url.query_pairs_mut().append_pair("artist", artist);

        if let Some(c) = count {
            url.query_pairs_mut().append_pair("count", &c.to_string());
        }

        let req = self.client.get(url).query(&self.build_auth_query());
        let res = req.send().await?.json::<SubsonicResponse<TopSongs<'static>>>().await?;

        let mut top_songs = res.into_answer()?;
        top_songs.tag_origin(self.id(), IdType::Song);
        if let Some(songs) = top_songs.song.as_mut() {
            songs.iter_mut().for_each(|song| {
                song.suffix =
                    Some(self.tagged_suffix(song.suffix.as_deref().unwrap_or_default()).into());
                song.starred = None;
                song.played = None;
            });
        }

        Ok(top_songs)
    }

    async fn search<'a>(&self, query: &SearchRequest<'a>) -> Result<SearchResult<'a>> {
        let url = self.base_url.join("/rest/search3")?;

        let mut req =
            self.client.get(url).query(&self.build_auth_query()).query(&[("query", &query.query)]);

        if let Some(count) = query.artist_count {
            req = req.query(&[("artistCount", count)]);
        }
        if let Some(count) = query.album_count {
            req = req.query(&[("albumCount", count)]);
        }
        if let Some(count) = query.song_count {
            req = req.query(&[("songCount", count)]);
        }

        let res = req.send().await?.json::<SubsonicResponse<SearchResult<'static>>>().await?;

        let mut search_results = res
            .into_answer()
            .inspect_err(|err| log::error!("Search failed on federated server: {err}"))?;
        search_results.tag_origin(self.id(), IdType::Generic);

        Ok(search_results)
    }

    async fn get_cover_art(&self, id: &Id<'_>, size: Option<u32>) -> Result<reqwest::Response> {
        let url = self.base_url.join("/rest/getCoverArt")?;

        let mut req =
            self.client.get(url).query(&self.build_auth_query()).query(&[("id", id.key())]);

        if let Some(s) = size {
            req = req.query(&[("size", s)]);
        }

        let res = req.send().await?;

        if !res.status().is_success() {
            return Err(RemoteError::Status(res.status()).into());
        }

        let content_type =
            res.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("");

        if content_type.starts_with("application/json") || content_type.starts_with("text/") {
            return Err(FederatedError::NotAnImage(content_type.to_string()).into());
        }

        return Ok(res);
    }

    async fn stream(&self, req: &StreamMedia) -> Result<reqwest::Response> {
        let url = self.base_url.join("/rest/stream")?;

        let mut http_req = self
            .client
            .get(url)
            .query(&self.build_auth_query())
            .query(&[("id", req.request.id.key())]);

        if let Some(max_bit_rate) = req.request.max_bit_rate {
            http_req = http_req.query(&[("maxBitRate", max_bit_rate)]);
        }
        if let Some(ref format) = req.request.format {
            http_req = http_req.query(&[("format", format)]);
        }
        if let Some(time_offset) = req.request.time_offset {
            http_req = http_req.query(&[("timeOffset", time_offset)]);
        }
        if let Some(ref size) = req.request.size {
            http_req = http_req.query(&[("size", size)]);
        }
        if let Some(estimate) = req.request.estimate_content_length {
            http_req = http_req.query(&[("estimateContentLength", estimate.to_string())]);
        }
        if let Some(converted) = req.request.converted {
            http_req = http_req.query(&[("converted", converted.to_string())]);
        }

        if let Some(ref range) = req.range_header {
            http_req = http_req.header(RANGE, range);
        }

        let res = http_req.send().await?;

        if !res.status().is_success() {
            return Err(RemoteError::Status(res.status()).into());
        }

        Ok(res)
    }
}
