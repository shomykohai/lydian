use std::collections::HashMap;
use std::fmt::Display;

use anyhow::Result;
use axum::Json;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use mime_type::Audio;
use reqwest::Response;
use serde::de::Error;
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use serde_json::{Map, Value};
use serde_repr::{Deserialize_repr, Serialize_repr};
use thiserror::Error as TError;

use crate::models::CowStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubsonicRespFormat {
    Json,
    Xml,
}

#[derive(Default, Clone, Copy, PartialEq, Eq, Debug, Hash, Serialize)]
pub enum SubsonicStreamFormat {
    #[default]
    Raw,
    Mp3,
    Ogg,
    Flac,
    Aac,
}

impl<'de> Deserialize<'de> for SubsonicStreamFormat {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        log::info!("Deserializing SubsonicStreamFormat from string: {}", s);
        match s.as_str() {
            "raw" => Ok(Self::Raw),
            "mp3" => Ok(Self::Mp3),
            "opus" | "ogg" => Ok(Self::Ogg),
            "flac" => Ok(Self::Flac),
            "aac" => Ok(Self::Aac),
            /* Seems like subsonic servers default to raw format */
            _ => Ok(Self::Raw),
        }
    }
}

impl Display for SubsonicStreamFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Raw => "raw",
            Self::Mp3 => "mp3",
            Self::Ogg => "ogg",
            Self::Flac => "flac",
            Self::Aac => "aac",
        };
        write!(f, "{}", s)
    }
}

impl From<Audio> for SubsonicStreamFormat {
    fn from(audio: Audio) -> Self {
        match audio {
            Audio::Mpeg => Self::Mp3,
            Audio::Ogg => Self::Ogg,
            Audio::Flac => Self::Flac,
            Audio::Aac => Self::Aac,
            _ => Self::Raw,
        }
    }
}

impl SubsonicStreamFormat {
    pub const fn mime_type(&self) -> &'static str {
        match self {
            Self::Raw => "audio/mpeg",
            Self::Mp3 => "audio/mpeg",
            Self::Ogg => "audio/ogg",
            Self::Flac => "audio/flac",
            Self::Aac => "audio/aac",
        }
    }
}

#[derive(Debug)]
pub struct SubsonicStatus(pub StatusCode);

impl From<StatusCode> for SubsonicStatus {
    fn from(status: StatusCode) -> Self {
        Self(status)
    }
}

impl From<SubsonicStatus> for StatusCode {
    fn from(val: SubsonicStatus) -> Self {
        val.0
    }
}

impl Serialize for SubsonicStatus {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if self.0.is_success() {
            serializer.serialize_str("ok")
        } else {
            serializer.serialize_str("failed")
        }
    }
}

impl<'de> Deserialize<'de> for SubsonicStatus {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "ok" => Ok(Self(StatusCode::OK)),
            "failed" => Ok(Self(StatusCode::INTERNAL_SERVER_ERROR)),
            _ => Err(serde::de::Error::custom(format!("Invalid status: {}", s))),
        }
    }
}

pub trait SubsonicEntity {
    const FIELD_NAME: &'static str;
}

impl SubsonicEntity for () {
    const FIELD_NAME: &'static str = "";
}

#[derive(TError, Debug, Serialize_repr, Deserialize_repr)]
#[repr(u32)]
pub enum SubsonicErrorCode {
    #[error("A generic error.")]
    Generic = 0,
    #[error("Required parameter is missing.")]
    RequiredParameterMissing = 10,
    #[error("Incompatible Subsonic REST protocol version. Client must upgrade.")]
    ClientUpgradeRequired = 20,
    #[error("Incompatible Subsonic REST protocol version. Server must upgrade.")]
    ServerUpgradeRequired = 30,
    #[error("Wrong username or password.")]
    WrongCredentials = 40,
    #[error("Token authentication not supported for LDAP users.")]
    TokenAuthNotSupportedForLdap = 41,
    #[error("Provided authentication mechanism not supported.")]
    AuthMechanismNotSupported = 42,
    #[error("Multiple conflicting authentication mechanisms provided.")]
    MultipleAuthMechanisms = 43,
    #[error("Invalid API key.")]
    InvalidApiKey = 44,
    #[error("User is not authorized for the given operation.")]
    NotAuthorized = 50,
    #[error(
        "The trial period for the Subsonic server is over. Please upgrade to Subsonic Premium. Visit subsonic.org for details."
    )]
    TrialPeriodOver = 60,
    #[error("The requested data was not found.")]
    NotFound = 70,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SubsonicError<'a> {
    pub code: SubsonicErrorCode,
    pub message: Option<CowStr<'a>>,
    pub help_url: Option<CowStr<'a>>,
}

#[derive(Debug)]
pub struct SubsonicResponse<'a, T> {
    pub status: SubsonicStatus,
    pub version: CowStr<'a>,
    pub server_name: CowStr<'a>,
    pub server_version: CowStr<'a>,
    pub open_subsonic: bool,
    pub error: Option<SubsonicError<'a>>,
    pub answer: Option<T>,
}

impl<'a, T> SubsonicResponse<'a, T>
where
    T: serde::de::DeserializeOwned + SubsonicEntity,
{
    pub async fn from_response(resp: Response) -> Result<Self, reqwest::Error> {
        let subsonic_response = resp.json::<SubsonicResponse<T>>().await?;

        Ok(subsonic_response)
    }

    pub const fn is_err(&self) -> bool {
        self.error.is_some()
    }
}

impl<'a, T> IntoResponse for SubsonicResponse<'a, T>
where
    T: serde::Serialize + SubsonicEntity,
{
    fn into_response(self) -> axum::response::Response {
        (StatusCode::OK, Json(self)).into_response()
    }
}

impl<'de, 'a, T> Deserialize<'de> for SubsonicResponse<'a, T>
where
    T: Deserialize<'de> + SubsonicEntity,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let mut root_map = HashMap::<&str, &'de RawValue>::deserialize(deserializer)?;
        let inner_raw = root_map
            .remove("subsonic-response")
            .ok_or_else(|| D::Error::missing_field("subsonic-response"))?;

        let mut fields_map = HashMap::<&str, &'de RawValue>::deserialize(
            &mut serde_json::Deserializer::from_str(inner_raw.get()),
        )
        .map_err(D::Error::custom)?;

        fn parse_field<'de, V, E>(
            map: &mut HashMap<&str, &'de RawValue>,
            key: &'static str,
        ) -> Result<V, E>
        where
            V: Deserialize<'de>,
            E: serde::de::Error,
        {
            let raw = map.remove(key).ok_or_else(|| E::missing_field(key))?;
            serde_json::from_str(raw.get()).map_err(E::custom)
        }

        let status = parse_field(&mut fields_map, "status")?;
        let version = parse_field(&mut fields_map, "version")?;
        let server_name = parse_field(&mut fields_map, "type")?;
        let server_version = parse_field(&mut fields_map, "serverVersion")?;
        let open_subsonic = parse_field(&mut fields_map, "openSubsonic")?;

        let answer = match fields_map.remove(T::FIELD_NAME) {
            Some(raw) => Some(serde_json::from_str(raw.get()).map_err(D::Error::custom)?),
            None => None,
        };

        let error = match fields_map.remove("error") {
            Some(raw) => Some(serde_json::from_str(raw.get()).map_err(D::Error::custom)?),
            None => None,
        };

        Ok(SubsonicResponse {
            status,
            version,
            server_name,
            server_version,
            open_subsonic,
            answer,
            error,
        })
    }
}

impl<'a, T> Serialize for SubsonicResponse<'a, T>
where
    T: Serialize + SubsonicEntity,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut response = Map::new();

        fn to_val<V, E>(v: V) -> Result<Value, E>
        where
            V: Serialize,
            E: serde::ser::Error,
        {
            serde_json::to_value(v).map_err(E::custom)
        }

        response.insert("status".into(), to_val(&self.status)?);
        response.insert("version".into(), to_val(&self.version)?);
        response.insert("type".into(), to_val(&self.server_name)?);
        response.insert("serverVersion".into(), to_val(&self.server_version)?);
        response.insert("openSubsonic".into(), to_val(self.open_subsonic)?);

        if let Some(ref answer) = self.answer {
            response.insert(T::FIELD_NAME.into(), to_val(answer)?);
        }

        if let Some(ref error) = self.error {
            response.insert("error".into(), to_val(error)?);
        }

        let mut root = serializer.serialize_map(Some(1))?;
        root.serialize_entry("subsonic-response", &Value::Object(response))?;
        root.end()
    }
}
