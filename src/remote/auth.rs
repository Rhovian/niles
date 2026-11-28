use super::{
    ApiError,
    oidc::{TokenGrant, Verifier},
};
use anyhow::Result;
use axum::Json;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::sync::Arc;
use url::Url;

#[derive(Clone)]
pub(super) struct Auth {
    verifier: Arc<Verifier>,
    resource: Url,
    challenge: HeaderValue,
    invalid_token: HeaderValue,
}

impl Auth {
    pub(super) fn new(
        verifier: Arc<Verifier>,
        resource: Url,
        metadata: Option<&str>,
    ) -> Result<Self> {
        let challenge = match metadata {
            Some(url) => format!("Bearer resource_metadata=\"{url}\""),
            None => "Bearer".to_owned(),
        };
        let separator = if metadata.is_some() { ", " } else { " " };
        Ok(Self {
            verifier,
            resource,
            challenge: challenge.parse()?,
            invalid_token: format!("{challenge}{separator}error=\"invalid_token\"").parse()?,
        })
    }
}

pub(super) async fn authenticate(
    State(auth): State<Auth>,
    mut request: Request,
    next: Next,
) -> Response {
    let mut response = match bearer(request.headers()) {
        Some(token) => match auth.verifier.verify(auth.resource.as_str(), token).await {
            Ok(Some(grants)) if !grants.is_empty() => {
                request.headers_mut().remove(header::AUTHORIZATION);
                request.extensions_mut().insert(Arc::new(grants));
                next.run(request).await
            }
            Ok(Some(_)) => StatusCode::FORBIDDEN.into_response(),
            Ok(None) => challenge(auth.invalid_token),
            Err(error) => ApiError(error).into_response(),
        },
        None if request.headers().contains_key(header::AUTHORIZATION) => {
            challenge(auth.invalid_token)
        }
        None => challenge(auth.challenge),
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub(super) async fn exchange(
    State(auth): State<Auth>,
    Json(grant): Json<TokenGrant>,
) -> Result<Response, ApiError> {
    let Some(token) = auth.verifier.exchange(auth.resource, grant).await? else {
        return Ok(StatusCode::BAD_REQUEST.into_response());
    };
    Ok(([(header::CACHE_CONTROL, "no-store")], Json(token)).into_response())
}

fn challenge(value: HeaderValue) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, value)],
    )
        .into_response()
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let Ok(value) = values.next()?.to_str() else {
        return None;
    };
    let (scheme, token) = value.split_once(' ')?;
    (values.next().is_none() && scheme.eq_ignore_ascii_case("bearer") && !token.trim().is_empty())
        .then_some(token.trim_start_matches(' '))
}
