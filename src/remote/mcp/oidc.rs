use crate::config::user::{FileMcp, FileMcpGrant, McpAccess};
use anyhow::{Context, Result, ensure};
use isahc::HttpClient;
use isahc::config::{Configurable, RedirectPolicy};
use jsonwebtoken::dangerous::insecure_decode;
use jsonwebtoken::jwk::{AlgorithmParameters, Jwk};
use jsonwebtoken::{AlgorithmFamily, DecodingKey, Validation, decode};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use url::Url;

use super::super::blocking;

const KEYS_TTL: Duration = Duration::from_secs(300);
const MAX_KEY_AGE: Duration = Duration::from_secs(900);
const RETRY_DELAY: Duration = Duration::from_secs(10);
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_DOCUMENT_BYTES: u64 = 256 * 1024;

pub(super) struct Verifier {
    audience: String,
    issuers: BTreeMap<String, Issuer>,
}

#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    jwks_uri: String,
}

struct Issuer {
    grants: Vec<FileMcpGrant>,
    jwks_uri: Url,
    client: HttpClient,
    cache: Arc<Mutex<KeyCache>>,
}

// jsonwebtoken verifies with a supplied key and has no HTTP or refresh policy.
// https://docs.rs/jsonwebtoken/10.3.0/jsonwebtoken/fn.decode.html
struct KeyCache {
    keys: Arc<[Jwk]>,
    loaded_at: Instant,
    attempted_at: Instant,
}

#[derive(Deserialize)]
struct Identity {
    iss: String,
    sub: String,
}

pub(super) struct Grants(Vec<FileMcpGrant>);

impl Grants {
    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(super) fn allows(&self, project: &str, access: McpAccess) -> bool {
        self.0.iter().any(|grant| {
            grant.project == project
                && (grant.access == McpAccess::Operate || grant.access == access)
        })
    }
}

impl Verifier {
    pub(super) async fn load(config: &FileMcp) -> Result<Self> {
        let client = HttpClient::builder()
            .timeout(FETCH_TIMEOUT)
            .redirect_policy(RedirectPolicy::None)
            .build()?;
        let mut issuers = BTreeMap::new();
        for (name, grants) in &config.issuers {
            let client = client.clone();
            let issuer_name = name.clone();
            let grants = grants.clone();
            let issuer = blocking(move || Issuer::discover(&issuer_name, client, grants))
                .await
                .with_context(|| format!("load OIDC issuer {name}"))?;
            issuers.insert(name.clone(), issuer);
        }
        Ok(Self {
            audience: config.resource.clone(),
            issuers,
        })
    }

    pub(super) async fn verify(&self, token: &str) -> Result<Option<Grants>> {
        // Only the verified claims below can select grants. The unverified issuer
        // selects an entry from the operator's allowlist, never a network destination.
        let Ok(unverified) = insecure_decode::<Identity>(token) else {
            return Ok(None);
        };
        let Some(issuer) = self.issuers.get(&unverified.claims.iss) else {
            return Ok(None);
        };
        let header = unverified.header;
        if AlgorithmFamily::Hmac.algorithms().contains(&header.alg) {
            return Ok(None);
        }
        let keys = issuer.keys().await?;
        let mut validation = Validation::new(header.alg);
        validation.set_audience(&[&self.audience]);
        validation.set_required_spec_claims(&["iss", "sub", "aud", "exp"]);
        validation.validate_nbf = true;
        let Some(identity) = keys.iter().find_map(|key| {
            if header.kid.is_some() && header.kid != key.common.key_id {
                return None;
            }
            let Ok(key) = DecodingKey::from_jwk(key) else {
                return None;
            };
            match decode::<Identity>(token, &key, &validation) {
                Ok(signed) => Some(signed.claims),
                Err(_) => None,
            }
        }) else {
            return Ok(None);
        };
        Ok(Some(Grants(
            issuer
                .grants
                .iter()
                .filter(|grant| grant.subject == identity.sub)
                .cloned()
                .collect(),
        )))
    }
}

impl Discovery {
    fn keys_url(self, issuer: &str) -> Result<Url> {
        ensure!(self.issuer == issuer, "OIDC discovery issuer mismatch");
        https_url(&self.jwks_uri)
    }
}

// jsonwebtoken's JwkSet rejects the whole document on an unsupported key.
// https://docs.rs/jsonwebtoken/10.3.0/jsonwebtoken/jwk/struct.JwkSet.html
#[derive(Deserialize)]
struct KeyDocument {
    keys: Vec<Value>,
}

#[expect(
    clippy::disallowed_methods,
    reason = "unsupported individual keys do not invalidate the issuer's supported keys"
)]
fn fetch_keys(client: &HttpClient, uri: &Url) -> Result<Arc<[Jwk]>> {
    let document: KeyDocument = fetch(client, uri.as_str())?;
    let keys: Vec<Jwk> = document
        .keys
        .into_iter()
        .filter_map(|key| serde_json::from_value::<Jwk>(key).ok())
        .filter(|key| {
            !matches!(key.algorithm, AlgorithmParameters::OctetKey(_))
                && DecodingKey::from_jwk(key).is_ok()
        })
        .collect();
    ensure!(
        !keys.is_empty(),
        "OIDC JWKS at {uri} has no supported public signing keys"
    );
    Ok(keys.into())
}

impl Issuer {
    fn discover(name: &str, client: HttpClient, grants: Vec<FileMcpGrant>) -> Result<Self> {
        ensure!(
            https_url(name)?.query().is_none(),
            "OIDC issuer has a query"
        );
        let discovery: Discovery = fetch(
            &client,
            &format!(
                "{}/.well-known/openid-configuration",
                name.trim_end_matches('/')
            ),
        )?;
        let jwks_uri = discovery.keys_url(name)?;
        let keys = fetch_keys(&client, &jwks_uri)?;
        let now = Instant::now();
        Ok(Self {
            grants,
            jwks_uri,
            client,
            cache: Arc::new(Mutex::new(KeyCache {
                keys,
                loaded_at: now,
                attempted_at: now,
            })),
        })
    }

    async fn keys(&self) -> Result<Arc<[Jwk]>> {
        let mut cache = self.cache.clone().lock_owned().await;
        if cache.loaded_at.elapsed() < KEYS_TTL || cache.attempted_at.elapsed() < RETRY_DELAY {
            return cache.usable_keys();
        }
        let client = self.client.clone();
        let uri = self.jwks_uri.clone();
        // The refresh owns the lock and completes even if the HTTP caller disconnects.
        tokio::spawn(async move {
            cache.attempted_at = Instant::now();
            match blocking(move || fetch_keys(&client, &uri)).await {
                Ok(keys) => {
                    cache.keys = keys;
                    cache.loaded_at = Instant::now();
                }
                Err(error) => eprintln!("refresh OIDC signing keys: {error:#}"),
            }
            cache.usable_keys()
        })
        .await?
    }
}

impl KeyCache {
    fn usable_keys(&self) -> Result<Arc<[Jwk]>> {
        ensure!(
            self.loaded_at.elapsed() < MAX_KEY_AGE,
            "OIDC signing keys expired"
        );
        Ok(self.keys.clone())
    }
}

pub(super) fn https_url(value: &str) -> Result<Url> {
    let url = Url::parse(value).context("parse HTTPS URL")?;
    ensure!(
        url.scheme() == "https"
            && url.host().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "expected an HTTPS URL without user info or fragment"
    );
    Ok(url)
}

fn fetch<T: DeserializeOwned>(client: &HttpClient, uri: &str) -> Result<T> {
    let response = client.get(uri)?;
    ensure!(
        response.status().is_success(),
        "OIDC document returned HTTP {}",
        response.status()
    );
    Ok(serde_json::from_reader(
        response.into_body().take(MAX_DOCUMENT_BYTES),
    )?)
}

#[cfg(test)]
#[path = "oidc_tests.rs"]
mod tests;
