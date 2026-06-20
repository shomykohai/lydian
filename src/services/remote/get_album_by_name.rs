use std::borrow::Cow;

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use super::{RemoteError, RemoteOperation};
use crate::MusicProvider;
use crate::models::AlbumWithSongsID3;

#[derive(Debug, Deserialize)]
pub struct GetAlbumByName<'op> {
    pub name: &'op str,
}

#[async_trait]
impl<'op> RemoteOperation for GetAlbumByName<'op> {
    type Output = Vec<AlbumWithSongsID3<'op>>;

    fn target(&self) -> &str {
        ""
    }

    async fn execute(&self, provider: &dyn MusicProvider) -> Result<Self::Output> {
        /* We return a Vec because some routes might need to compare against multiple albums */
        let albums = provider.get_albums_by_name(self.name).await;

        if albums.is_empty() {
            return Err(RemoteError::EmptyDataset.into());
        }

        Ok(albums)
    }

    fn process(&self, items: Vec<Self::Output>) -> Result<Self::Output, RemoteError> {
        if items.is_empty() {
            return Err(RemoteError::EmptyDataset);
        }

        let flattened: Vec<AlbumWithSongsID3<'op>> = items.into_iter().flatten().collect();

        Ok(flattened)
    }

    fn cache_key(&self) -> Option<Cow<'op, str>> {
        Some(format!("get_album_by_name:{}", self.name).into())
    }

    fn deserialize(&self, value: Value) -> Option<Self::Output> {
        serde_json::from_value(value).ok()
    }

    fn serialize(&self, output: &Self::Output) -> Option<Value> {
        serde_json::to_value(output).ok()
    }
}
