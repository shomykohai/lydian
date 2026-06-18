use std::sync::Arc;
use std::task::{Context, Poll};

use axum::extract::Request;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures::future::BoxFuture;
use log::{error, trace};
use tower::{Layer, Service};

use crate::AppState;
use crate::models::SubsonicResponse;

#[derive(Clone)]
pub struct AuthMiddlewareLayer {
    state: Arc<AppState>,
}

impl AuthMiddlewareLayer {
    pub const fn new(state: Arc<AppState>) -> Self {
        Self { state }
    }
}

impl<S> Layer<S> for AuthMiddlewareLayer {
    type Service = AuthMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        AuthMiddleware { inner, state: self.state.clone() }
    }
}

#[derive(Clone)]
pub struct AuthMiddleware<S> {
    inner: S,
    state: Arc<AppState>,
}

impl<S> Service<Request> for AuthMiddleware<S>
where
    S: Service<Request, Response = Response> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;
    type Response = Response;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request) -> Self::Future {
        // todo: return subsonic response instead
        let clone = self.inner.clone();
        let mut inner = std::mem::replace(&mut self.inner, clone);
        let state = self.state.clone();

        Box::pin(async move {
            let query_str = req.uri().query().unwrap_or("");

            let params: Vec<(String, String)> =
                serde_urlencoded::from_str(query_str).unwrap_or_default();

            let has_auth = params.iter().any(|(k, _)| k == "u")
                && params.iter().any(|(k, _)| k == "p" || k == "t");

            if !has_auth {
                trace!("Someone tried to authenticate without providing subsonic credentials!!");
                return Ok((
                    StatusCode::UNAUTHORIZED,
                    "Missing Subsonic authentication parameters",
                )
                    .into_response());
            }

            let new_query = params
                .into_iter()
                .filter(|(k, _)| matches!(k.as_str(), "c" | "u" | "t" | "p" | "v" | "s"))
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<String>>()
                .join("&");

            let upstream_url = format!(
                "{}/rest/ping.view?{}&f=json",
                state.config.subsonic.url.trim_end_matches('/'),
                new_query
            );

            match state.http_client.get(&upstream_url).send().await {
                Ok(resp) => {
                    let sub_resp = match SubsonicResponse::<()>::from_response(resp).await {
                        Ok(data) => data,
                        Err(err) => {
                            error!("Failed to parse response from authentication server: {err}");

                            return Ok((
                                StatusCode::INTERNAL_SERVER_ERROR,
                                "Invalid response from authentication server",
                            )
                                .into_response());
                        }
                    };

                    if sub_resp.is_err() {
                        trace!("Someone tried to authenticate with invalid credentials");
                        return Ok((StatusCode::UNAUTHORIZED, "Invalid Subsonic credentials")
                            .into_response());
                    }

                    inner.call(req).await
                }
                Err(_) => {
                    error!("Failed to reach upstream authentication server at {}", upstream_url);
                    Ok((StatusCode::BAD_GATEWAY, "Upstream authentication server unreachable")
                        .into_response())
                }
            }
        })
    }
}
