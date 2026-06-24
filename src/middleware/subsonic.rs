use std::borrow::Cow;
use std::convert::Infallible;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Result, bail};
use axum::body::Body;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, Request};
use axum::response::{IntoResponse, Response as AxumResponse};
use futures::TryStreamExt;
use log::{debug, error, info};
use quick_xml::Writer;
use quick_xml::events::{BytesDecl, Event};
use rand::RngExt;
use rand::distr::Alphanumeric;
use reqwest::header::{
    ACCEPT_RANGES,
    CONTENT_LENGTH,
    CONTENT_RANGE,
    CONTENT_TYPE,
    X_CONTENT_TYPE_OPTIONS,
};
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use tower::ServiceExt;
use tower_http::services::fs::ServeFile;
use url::Url;

use crate::models::{
    SubsonicEntity,
    SubsonicError,
    SubsonicErrorCode,
    SubsonicRespFormat,
    SubsonicResponse,
    SubsonicStreamFormat,
};
use crate::providers::StreamRequest;
use crate::services::{ServiceManager, TranscodeRequest};
use crate::utils::request::json_to_xml;
use crate::{Error, SUBSONIC_API_VERSION, VERSION};

impl<S> FromRequestParts<S> for SubsonicRespFormat
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let query = parts.uri.query().unwrap_or_default();
        if query.contains("f=json") { Ok(Self::Json) } else { Ok(Self::Xml) }
    }
}

pub struct SubsonicClient<'a> {
    client: Client,
    base_url: Url,
    username: Option<Cow<'a, str>>,
    password: Option<Cow<'a, str>>,
    name: Cow<'a, str>,
    version: Cow<'a, str>,
    open_subsonic: bool,
    services: Arc<ServiceManager>,
}

impl<'a> SubsonicClient<'a> {
    pub fn new(
        client: Client,
        base_url: &str,
        username: Option<String>,
        password: Option<String>,
        services: Arc<ServiceManager>,
    ) -> Result<Self> {
        let base_url = Url::parse(base_url)?;

        if !matches!(base_url.scheme(), "http" | "https") {
            bail!("Invalid URL scheme: {}", base_url.scheme());
        }

        /* TODO: Support (and prefer) token (API Keys actually) authentication instead */
        let username = username.map(Cow::Owned);
        let password = password.map(Cow::Owned);

        /*
         * Before initialization (and if the backend server is down),
         * the client will be unable to communicate with the server.
         * If we fail the initial ping, we can tell the client who we are
         * to let it know we're stalling.
         */
        Ok(Self {
            client,
            base_url,
            username,
            password,
            name: "Lydian".into(),
            version: VERSION.into(),
            open_subsonic: true,
            services,
        })
    }

    /*
     * Initializes the proxy client by pinging the origin server.
     * This must be called before any other method!!
     */
    pub async fn init(&mut self) -> Result<()> {
        /* We send a ping request to the server to check if it's alive
         * and gather some basic info for future usage.
         */
        let response = self.get("/rest/ping")?.send().await?;

        if response.status().is_success() {
            let subsonic_response = response.json::<SubsonicResponse<()>>().await?;

            /* Clone here is necessary, or we'd be borrowing data from the respones
             * which would be dropped at the end of this function!
             */
            self.name = subsonic_response.server_name.clone();
            self.version = subsonic_response.server_version.clone();
            self.open_subsonic = subsonic_response.open_subsonic;

            info!("Connected to Subsonic server: {} - {}", self.name, self.version);
            info!("OpenSubsonic support: {}", self.open_subsonic);

            Ok(())
        } else {
            bail!("Ping failed with status: {}", response.status());
        }
    }

    pub async fn forward_request(
        &self,
        method: Method,
        endpoint: &str,
        query: Option<&str>,
        headers: HeaderMap,
        body: Option<Body>,
    ) -> Result<Response> {
        let mut url = self.base_url.join(endpoint)?;
        url.set_query(query);

        let mut request = self.client.request(method.clone(), url);

        request = request.headers(headers);

        if let Some(b) = body {
            request = request.body(reqwest::Body::wrap_stream(b.into_data_stream()));
        }

        let response = request.send().await?;
        Ok(response)
    }

    /* Wrapper for GET requests.
     * This is so we can easily add some common parameters when performing
     * requests proxy to server.
     * This is NOT the same as forward_request.
     * Forward request will just take the incoming request and end it as is
     * to the server, while this is specifically for the proxy!!
     */
    pub fn get(&self, endpoint: &str) -> Result<RequestBuilder> {
        let url = self.base_url.join(endpoint)?;

        /* Salt must be at least 6 characters, letters and number, and different each request */
        let salt: String =
            rand::rng().sample_iter(&Alphanumeric).take(12).map(char::from).collect();

        let mut context = md5::Context::new();
        context.consume(self.password.as_deref().unwrap_or_default().as_bytes());
        context.consume(salt.as_bytes());
        let digest = context.finalize();
        let token = format!("{:x}", digest);

        let params = [
            ("u", self.username.as_deref().unwrap_or_default()),
            ("t", &token),
            ("s", &salt),
            ("f", "json"),
            ("v", SUBSONIC_API_VERSION),
            ("c", "lydian"),
        ];

        Ok(self.client.get(url).query(&params))
    }

    /*
     * Performs a request to the Subsonic origin server, and
     * returns the response as a SubsonicResponse<T>.
     */
    pub async fn request<T>(
        &self,
        method: Method,
        path: &str,
        query: Option<&str>,
        headers: HeaderMap,
        body: Option<Body>,
    ) -> Result<SubsonicResponse<'_, T>>
    where
        T: DeserializeOwned + SubsonicEntity,
    {
        let upstream_resp = self.forward_request(method, path, query, headers, body).await?;
        let parsed = SubsonicResponse::from_response(upstream_resp).await?;
        Ok(parsed)
    }

    /*
     * Builds a Subsonic response, with the given status code and answer.
     * The answer must implement the SubsonicEntity trait, which defines
     * the name of the field in the response.
     */
    pub fn build_response<T: serde::Serialize + SubsonicEntity>(
        &self,
        format: SubsonicRespFormat,
        status: StatusCode,
        answer: Option<T>,
    ) -> AxumResponse {
        let raw_response = SubsonicResponse {
            status: status.into(),
            version: Cow::Owned(self.version.clone().into_owned()),
            server_name: Cow::Owned(self.name.clone().into_owned()),
            server_version: Cow::Owned(self.version.clone().into_owned()),
            open_subsonic: self.open_subsonic,
            answer,
            error: None,
        };

        self.render_payload(format, raw_response)
    }

    /*
     * Builds a Subsonic error response, with the given error code.
     */
    pub fn build_error(
        &self,
        format: SubsonicRespFormat,
        error: SubsonicErrorCode,
    ) -> AxumResponse {
        let message = error.to_string();
        let error_payload =
            SubsonicError { code: error, message: Some(message.into()), help_url: None };

        let raw_response = SubsonicResponse::<()> {
            status: StatusCode::INTERNAL_SERVER_ERROR.into(),
            version: Cow::Owned(self.version.clone().into_owned()),
            server_name: Cow::Owned(self.name.clone().into_owned()),
            server_version: Cow::Owned(self.version.clone().into_owned()),
            open_subsonic: self.open_subsonic,
            answer: None,
            error: Some(error_payload),
        };

        self.render_payload(format, raw_response)
    }

    /*
     * Tries to perform a request to the origin subsonic server,
     * and returns the response as a SubsonicResponse<T> if successful.
     * If the request fails, returns None.
     * Works like try_media, but for api endpoints with data.
     */
    pub async fn try_req<T>(
        &self,
        format: SubsonicRespFormat,
        parts: &Parts,
        body: Body,
    ) -> Option<AxumResponse>
    where
        T: serde::de::DeserializeOwned + serde::Serialize + SubsonicEntity,
    {
        if let Ok(resp) = self
            .request::<T>(
                parts.method.clone(),
                parts.uri.path(),
                parts.uri.query(),
                parts.headers.clone(),
                Some(body),
            )
            .await
            && !resp.is_err()
        {
            let clean_response = SubsonicResponse {
                status: resp.status,
                version: Cow::Owned(resp.version.into_owned()),
                server_name: Cow::Owned(resp.server_name.into_owned()),
                server_version: Cow::Owned(resp.server_version.into_owned()),
                open_subsonic: resp.open_subsonic,
                answer: resp.answer,
                error: None,
            };
            return Some(self.render_payload(format, clean_response));
        }
        None
    }

    /*
     * Converts a query into a subsonic remote operation struct.
     */
    pub fn parse_query<'de, T>(&self, parts: &'de Parts) -> Result<T, SubsonicErrorCode>
    where
        T: serde::de::Deserialize<'de>,
    {
        serde_urlencoded::from_str::<T>(parts.uri.query().unwrap_or_default()).map_err(|e| {
            debug!(
                "Failed to parse query: {}, error: {}",
                parts.uri.query().unwrap_or_default(),
                e
            );
            SubsonicErrorCode::RequiredParameterMissing
        })
    }

    /*
     * Serves a local media file from disk,
     * thanks to tower, no need to handle range headers!
     */
    pub async fn stream_local_media(
        &self,
        format: SubsonicRespFormat,
        file_path: PathBuf,
        parts: Parts,
        stream_request: Option<&StreamRequest<'a>>,
    ) -> AxumResponse {
        if let Some(req) = stream_request {
            let is_raw = req.format.is_none() || req.format == Some(SubsonicStreamFormat::Raw);
            let needs_transcode =
                !is_raw && (req.format.is_some() || req.max_bit_rate.is_some_and(|b| b > 0));

            if needs_transcode {
                return self.transcode_local_media(format, file_path, req).await;
            }
        }

        debug!("Streaming local media: {}", file_path.display());

        let serve_file = ServeFile::new(&file_path);

        let mut request = Request::from_parts(parts, Body::empty());
        request.headers_mut().insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));

        serve_file.oneshot(request).await.map_or_else(
            |_| self.build_error(format, SubsonicErrorCode::Generic),
            |response| response.into_response(),
        )
    }

    pub async fn transcode_local_media(
        &self,
        subsonic_format: SubsonicRespFormat,
        file_path: PathBuf,
        req: &StreamRequest<'_>,
    ) -> AxumResponse {
        let target_format = req.format.unwrap_or(SubsonicStreamFormat::Mp3);

        let transcode_req = TranscodeRequest {
            id: req.id.to_string(),
            file_path,
            format: target_format,
            bitrate: req.max_bit_rate.unwrap_or(0),
            offset: req.time_offset.unwrap_or(0),
        };

        match self.services.transcode.clone().oneshot(transcode_req).await {
            Ok(body) => AxumResponse::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, target_format.mime_type())
                .header(X_CONTENT_TYPE_OPTIONS, "nosniff")
                .body(body)
                .unwrap_or_else(|_| self.build_error(subsonic_format, SubsonicErrorCode::Generic)),
            Err(e) => {
                error!("Transcoding stack execution error: {}", e);
                self.build_error(subsonic_format, SubsonicErrorCode::Generic)
            }
        }
    }

    /*
     * Stream a media response from a remote source
     */
    pub fn stream_media(&self, format: SubsonicRespFormat, resp: Response) -> AxumResponse {
        let content_type =
            resp.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_owned();

        if content_type.starts_with("application/json") || content_type.starts_with("text/") {
            return self.build_error(format, SubsonicErrorCode::Generic);
        }

        let status_code = resp.status();
        let upstream_headers = resp.headers();

        let mut response_builder = AxumResponse::builder().status(status_code);

        if !content_type.is_empty() {
            response_builder = response_builder.header(CONTENT_TYPE, content_type);
        }
        if let Some(c_len) = upstream_headers.get(CONTENT_LENGTH) {
            response_builder = response_builder.header(CONTENT_LENGTH, c_len);
        }
        if let Some(c_range) = upstream_headers.get(CONTENT_RANGE) {
            response_builder = response_builder.header(CONTENT_RANGE, c_range);
        }
        if let Some(acc_ranges) = upstream_headers.get(ACCEPT_RANGES) {
            response_builder = response_builder.header(ACCEPT_RANGES, acc_ranges);
        }

        response_builder = response_builder.header(X_CONTENT_TYPE_OPTIONS, "nosniff");

        let chunk_stream = resp.bytes_stream().map_err(|err| Error::Internal(err.to_string()));

        response_builder
            .body(Body::from_stream(chunk_stream))
            .unwrap_or_else(|_| self.build_error(format, SubsonicErrorCode::Generic))
    }

    /*
     * Tries to perform a request to the origin subsonic server.
     * If the response is a media stream, we return an Axum Response,
     * with the media stream as body.
     * If the response is an error, we return None, so the caller can
     * decide how to handle it.
     */
    pub async fn try_media(
        &self,
        format: SubsonicRespFormat,
        parts: &Parts,
        body: Body,
    ) -> Option<AxumResponse> {
        if let Ok(resp) = self
            .forward_request(
                parts.method.clone(),
                parts.uri.path(),
                parts.uri.query(),
                parts.headers.clone(),
                Some(body),
            )
            .await
        {
            let content_type =
                resp.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("");

            if !(content_type.starts_with("application/json")
                || content_type.starts_with("application/xml"))
                && !content_type.starts_with("text/")
            {
                return Some(self.stream_media(format, resp));
            }
        }
        None
    }

    /*
     * Renders the subsonic response into the requested format
     */
    fn render_payload<T>(
        &self,
        format: SubsonicRespFormat,
        response: SubsonicResponse<'_, T>,
    ) -> AxumResponse
    where
        T: serde::Serialize + SubsonicEntity,
    {
        fn to_val<V: serde::Serialize>(v: V) -> Value {
            serde_json::to_value(v).unwrap_or(Value::Null)
        }

        let mut response_map = Map::new();
        response_map.insert("status".into(), to_val(&response.status));
        response_map.insert("version".into(), to_val(&response.version));
        response_map.insert("type".into(), to_val(&response.server_name));
        response_map.insert("serverVersion".into(), to_val(&response.server_version));
        response_map.insert("openSubsonic".into(), to_val(response.open_subsonic));

        if let Some(ref answer) = response.answer {
            response_map.insert(T::FIELD_NAME.into(), to_val(answer));
        }

        if let Some(ref error) = response.error {
            response_map.insert("error".into(), to_val(error));
        }

        match format {
            SubsonicRespFormat::Json => {
                let mut root = Map::new();
                root.insert("subsonic-response".into(), Value::Object(response_map));
                (StatusCode::OK, axum::Json(Value::Object(root))).into_response()
            }
            SubsonicRespFormat::Xml => {
                let mut writer = Writer::new(Cursor::new(Vec::with_capacity(1024)));
                let _ = writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)));

                if json_to_xml(&mut writer, "subsonic-response", &Value::Object(response_map))
                    .is_err()
                {
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }

                let xml_bytes = writer.into_inner().into_inner();
                AxumResponse::builder()
                    .status(StatusCode::OK)
                    .header(CONTENT_TYPE, "application/xml; charset=utf-8")
                    .body(Body::from(xml_bytes))
                    .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
            }
        }
    }

    pub async fn scan(&self, format: SubsonicRespFormat) -> AxumResponse {
        debug!("Starting library scan...");

        let Ok(req_builder) = self.get("/rest/startScan") else {
            return self.build_error(format, SubsonicErrorCode::Generic);
        };

        let Ok(resp) = req_builder.send().await else {
            return self.build_error(format, SubsonicErrorCode::Generic);
        };

        if !resp.status().is_success() {
            debug!("Failed to start scan, status: {}", resp.status());
            return self.build_error(format, SubsonicErrorCode::Generic);
        }

        debug!("Scan started successfully");
        self.build_response(format, StatusCode::OK, None::<()>)
    }
}
