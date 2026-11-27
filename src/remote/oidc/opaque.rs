use super::{
    Discovery, FileIntrospection, HttpClient, Principal, get_current_timestamp, https_url,
    read_document,
};
use anyhow::{Context, Result, ensure};
use isahc::{Body, Request, Response, http::StatusCode};
use serde::{Deserialize, Serialize};
use std::{env, slice};
use url::{Url, form_urlencoded};

pub(super) struct Opaque {
    client: HttpClient,
    introspection_endpoint: Url,
    token_endpoint: Url,
    client_id: String,
    client_secret: String,
}

#[derive(Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub(in crate::remote) enum TokenGrant {
    Code { code: String, code_verifier: String },
    Refresh { refresh_token: String },
}

#[derive(Deserialize, Serialize)]
pub(in crate::remote) struct TokenResponse {
    access_token: String,
    token_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_in: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
}

#[derive(Deserialize)]
struct Introspection {
    active: bool,
    #[serde(flatten)]
    claims: Option<Claims>,
}

#[derive(Deserialize)]
struct Claims {
    iss: String,
    sub: String,
    exp: u64,
    nbf: Option<u64>,
    client_id: String,
    token_type: Option<String>,
    aud: Audiences,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Audiences {
    One(String),
    Many(Vec<String>),
}

impl Audiences {
    fn is_bound(&self, resource: &str, client: &str) -> bool {
        let aud = match self {
            Self::One(aud) => slice::from_ref(aud),
            Self::Many(aud) => aud.as_slice(),
        };
        aud.iter().any(|value| value == resource)
            && aud.iter().all(|value| value == resource || value == client)
    }
}

impl Opaque {
    pub(super) fn discover(settings: &FileIntrospection, client: HttpClient) -> Result<Self> {
        let client_secret = env::var(&settings.client_secret_env).with_context(|| {
            format!(
                "load introspection client secret from {}",
                settings.client_secret_env
            )
        })?;
        ensure!(
            !settings.client_id.trim().is_empty() && !client_secret.is_empty(),
            "empty OAuth client credentials"
        );
        let discovery = Discovery::load(&settings.issuer, &client)?;
        Ok(Self {
            introspection_endpoint: https_url(
                &discovery
                    .introspection_endpoint
                    .context("missing introspection_endpoint")?,
            )?,
            token_endpoint: https_url(
                &discovery.token_endpoint.context("missing token_endpoint")?,
            )?,
            client_id: settings.client_id.clone(),
            client_secret,
            client,
        })
    }

    // RFC 7662 uses a form POST. url's serializer encodes the pairs; isahc sends the request.
    // https://www.rfc-editor.org/rfc/rfc7662.html#section-2.1
    fn post(&self, endpoint: &Url, fields: &[(&str, &str)]) -> Result<Response<Body>> {
        let body = form_urlencoded::Serializer::new(String::new())
            .extend_pairs(fields)
            .append_pair("client_id", &self.client_id)
            .append_pair("client_secret", &self.client_secret)
            .finish();
        let request = Request::post(endpoint.as_str())
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)?;
        Ok(self.client.send(request)?)
    }

    pub(super) fn verify(
        &self,
        issuer: &str,
        resource: &str,
        token: &str,
    ) -> Result<Option<(Principal, u64)>> {
        let response: Introspection = read_document(self.post(
            &self.introspection_endpoint,
            &[("token", token), ("token_type_hint", "access_token")],
        )?)?;
        let Some(claims) = response.claims.filter(|_| response.active) else {
            return Ok(None);
        };
        let now = get_current_timestamp();
        if claims.iss != issuer
            || claims.client_id != self.client_id
            || claims.sub.trim().is_empty()
            || claims.exp <= now
            || claims.nbf.is_some_and(|nbf| nbf > now)
            || claims
                .token_type
                .as_deref()
                .is_some_and(|kind| !kind.eq_ignore_ascii_case("bearer"))
            || !claims.aud.is_bound(resource, &claims.client_id)
        {
            return Ok(None);
        }
        Ok(Some((
            Principal {
                issuer: claims.iss,
                subject: claims.sub,
            },
            claims.exp,
        )))
    }

    pub(super) fn exchange(
        &self,
        resource: &Url,
        grant: TokenGrant,
    ) -> Result<Option<TokenResponse>> {
        let mut fields = vec![("resource", resource.as_str())];
        let redirect = resource.join("/")?;
        match &grant {
            TokenGrant::Code {
                code,
                code_verifier,
            } => fields.extend([
                ("grant_type", "authorization_code"),
                ("code", code),
                ("code_verifier", code_verifier),
                ("redirect_uri", redirect.as_str()),
            ]),
            TokenGrant::Refresh { refresh_token } => fields.extend([
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ]),
        }
        let response = self.post(&self.token_endpoint, &fields)?;
        if response.status() == StatusCode::BAD_REQUEST {
            return Ok(None);
        }
        Ok(Some(read_document(response)?))
    }
}
