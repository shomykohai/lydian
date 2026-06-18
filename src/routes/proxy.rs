use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use futures::TryStreamExt;
use log::trace;
use tower_real_ip::RealIp;

use crate::Error;
use crate::middleware::SubsonicClient;
use crate::models::{SubsonicErrorCode, SubsonicRespFormat};

pub async fn subsonic_proxy<'a>(
    format: SubsonicRespFormat,
    subsonic_client: Arc<SubsonicClient<'a>>,
    req: Request,
) -> Response {
    let real_ip = req.extensions().get::<RealIp>().copied();

    let (mut parts, body) = req.into_parts();

    if let Some(RealIp(ip)) = real_ip
        && let Ok(header_val) = HeaderValue::from_str(&ip.to_string())
    {
        parts.headers.insert("X-Real-IP", header_val.clone());
        parts.headers.insert("X-Forwarded-For", header_val);
    }

    let method = parts.method;
    let uri = parts.uri;
    let headers = parts.headers;

    trace!("{}: {}", method, uri.path());

    subsonic_client
        .forward_request(method, uri.path(), uri.query(), headers, Some(body))
        .await
        .map_or_else(
            |_| (StatusCode::BAD_GATEWAY, "Upstream server is offline").into_response(),
            |resp| {
                let mut builder = Response::builder().status(resp.status());

                for (name, value) in resp.headers() {
                    builder = builder.header(name, value);
                }

                let stream = resp.bytes_stream().map_err(|e| Error::Internal(e.to_string()));

                builder.body(Body::from_stream(stream)).unwrap_or_else(|_| {
                    subsonic_client.build_error(format, SubsonicErrorCode::Generic)
                })
            },
        )
}
