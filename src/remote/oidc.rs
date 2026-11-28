use crate::config::user::{Access, FileGrant, FileIntrospection};
use anyhow::{Context, Result, ensure};
use isahc::config::{Configurable, RedirectPolicy};
use isahc::{Body, HttpClient, Response};
use jsonwebtoken::dangerous::insecure_decode;
use jsonwebtoken::get_current_timestamp;
use jsonwebtoken::jwk::{AlgorithmParameters, Jwk};
use jsonwebtoken::{AlgorithmFamily, DecodingKey, Validation, decode};
use serde::de::{DeserializeOwned, Error};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use url::Url;

use super::blocking;
use opaque::Opaque;

mod opaque;
pub(super) use opaque::{TokenGrant, TokenResponse};

const KEYS_TTL: Duration = Duration::from_secs(300);
const MAX_KEY_AGE: Duration = Duration::from_secs(900);
const RETRY_DELAY: Duration = Duration::from_secs(10);
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_DOCUMENT_BYTES: u64 = 256 * 1024;

pub(super) struct Verifier {
    issuers: BTreeMap<String, Issuer>,
}

#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    jwks_uri: Option<String>,
    introspection_endpoint: Option<String>,
    token_endpoint: Option<String>,
}

struct Issuer {
    grants: Vec<FileGrant>,
    verification: Verification,
}

enum Verification {
    Jwt(JwtKeys),
    Introspection(Arc<Opaque>),
}

struct JwtKeys {
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
}

#[derive(Deserialize)]
struct SignedIdentity {
    iss: String,
    sub: String,
    exp: u64,
    #[serde(rename = "aud", deserialize_with = "sole_audience")]
    _audience: (),
}

// jsonwebtoken matches a configured audience against a set of token audiences.
// https://docs.rs/jsonwebtoken/10.3.0/src/jsonwebtoken/validation.rs.html
fn sole_audience<'de, D: Deserializer<'de>>(deserializer: D) -> Result<(), D::Error> {
    let audience = Value::deserialize(deserializer)?;
    if audience.is_string()
        || audience
            .as_array()
            .is_some_and(|entries| entries.len() == 1)
    {
        Ok(())
    } else {
        Err(D::Error::custom("expected a sole token audience"))
    }
}

#[derive(Clone, Deserialize, Eq, PartialEq, serde::Serialize)]
pub(super) struct Principal {
    issuer: String,
    subject: String,
}

pub(super) struct Grants {
    pub(super) principal: Principal,
    pub(super) expires: u64,
    grants: Vec<FileGrant>,
}

impl Grants {
    pub(super) fn is_empty(&self) -> bool {
        self.grants.is_empty()
    }

    pub(super) fn allows(&self, project: &str, access: Access) -> bool {
        self.grants.iter().any(|grant| {
            grant.project == project && (grant.access == Access::Operate || grant.access == access)
        })
    }
}

impl Verifier {
    pub(super) async fn load(
        config: BTreeMap<String, Vec<FileGrant>>,
        introspection: Option<FileIntrospection>,
    ) -> Result<Self> {
        if let Some(settings) = &introspection {
            ensure!(
                config.contains_key(&settings.issuer),
                "remote.introspection.issuer must name a trusted issuer"
            );
        }
        let client = HttpClient::builder()
            .timeout(FETCH_TIMEOUT)
            .redirect_policy(RedirectPolicy::None)
            .build()?;
        blocking(move || {
            let mut issuers = BTreeMap::new();
            for (name, grants) in config {
                let verification = match introspection
                    .as_ref()
                    .filter(|settings| settings.issuer == name)
                {
                    Some(settings) => Opaque::discover(settings, client.clone())
                        .map(Arc::new)
                        .map(Verification::Introspection),
                    None => JwtKeys::discover(&name, client.clone()).map(Verification::Jwt),
                }
                .with_context(|| format!("load OIDC issuer {name}"))?;
                issuers.insert(
                    name,
                    Issuer {
                        grants,
                        verification,
                    },
                );
            }
            Ok(Self { issuers })
        })
        .await
    }

    pub(super) fn trusts(&self, issuer: &str) -> bool {
        self.issuers.contains_key(issuer)
    }

    pub(super) fn can_read(&self, principal: &Principal, project: &str) -> bool {
        self.grants(principal).any(|grant| grant.project == project)
    }

    fn grants<'a>(&'a self, principal: &'a Principal) -> impl Iterator<Item = &'a FileGrant> {
        self.issuers
            .get(&principal.issuer)
            .map(|issuer| &issuer.grants)
            .into_iter()
            .flatten()
            .filter(|grant| grant.subject == principal.subject)
    }

    pub(super) async fn verify(&self, audience: &str, token: &str) -> Result<Option<Grants>> {
        let verified = if let Ok(unverified) = insecure_decode::<Identity>(token) {
            self.verify_jwt(audience, token, unverified).await?
        } else if let Some((issuer, opaque)) = self.introspection() {
            let (issuer, opaque) = (issuer.to_owned(), opaque.clone());
            let audience = audience.to_owned();
            let token = token.to_owned();
            blocking(move || opaque.verify(&issuer, &audience, &token)).await?
        } else {
            None
        };
        Ok(verified.map(|(principal, expires)| Grants {
            grants: self.grants(&principal).cloned().collect(),
            principal,
            expires,
        }))
    }

    fn introspection(&self) -> Option<(&str, &Arc<Opaque>)> {
        self.issuers
            .iter()
            .find_map(|(issuer, entry)| match &entry.verification {
                Verification::Introspection(opaque) => Some((issuer.as_str(), opaque)),
                Verification::Jwt(_) => None,
            })
    }

    pub(super) async fn exchange(
        &self,
        resource: Url,
        grant: TokenGrant,
    ) -> Result<Option<TokenResponse>> {
        let Some((_, opaque)) = self.introspection() else {
            return Ok(None);
        };
        let opaque = opaque.clone();
        blocking(move || opaque.exchange(&resource, grant)).await
    }

    async fn verify_jwt(
        &self,
        audience: &str,
        token: &str,
        unverified: jsonwebtoken::TokenData<Identity>,
    ) -> Result<Option<(Principal, u64)>> {
        // The unverified issuer can select a configured key set only.
        let Some(Issuer {
            verification: Verification::Jwt(keys),
            ..
        }) = self.issuers.get(&unverified.claims.iss)
        else {
            return Ok(None);
        };
        let header = unverified.header;
        if AlgorithmFamily::Hmac.algorithms().contains(&header.alg) {
            return Ok(None);
        }
        let keys = keys.keys().await?;
        let mut validation = Validation::new(header.alg);
        validation.set_audience(&[audience]);
        validation.set_required_spec_claims(&["iss", "sub", "aud", "exp"]);
        validation.validate_nbf = true;
        let Some(identity) = keys.iter().find_map(|key| {
            if header.kid.is_some() && header.kid != key.common.key_id {
                return None;
            }
            let Ok(key) = DecodingKey::from_jwk(key) else {
                return None;
            };
            match decode::<SignedIdentity>(token, &key, &validation) {
                Ok(signed) => Some(signed.claims),
                Err(_) => None,
            }
        }) else {
            return Ok(None);
        };
        let principal = Principal {
            issuer: identity.iss,
            subject: identity.sub,
        };
        Ok(Some((principal, identity.exp)))
    }
}

impl Discovery {
    fn load(issuer: &str, client: &HttpClient) -> Result<Self> {
        ensure!(
            https_url(issuer)?.query().is_none(),
            "OIDC issuer has a query"
        );
        let discovery: Self = fetch(
            client,
            &format!(
                "{}/.well-known/openid-configuration",
                issuer.trim_end_matches('/')
            ),
        )?;
        ensure!(discovery.issuer == issuer, "OIDC discovery issuer mismatch");
        Ok(discovery)
    }

    fn keys_url(self) -> Result<Url> {
        https_url(
            &self
                .jwks_uri
                .context("OIDC discovery is missing jwks_uri")?,
        )
    }
}

// jsonwebtoken's JwkSet rejects the whole document if it cannot parse an individual key.
// https://docs.rs/jsonwebtoken/10.3.0/jsonwebtoken/jwk/struct.JwkSet.html
#[derive(Deserialize)]
struct KeyDocument {
    keys: Vec<Value>,
}

fn fetch_keys(client: &HttpClient, uri: &Url) -> Result<Arc<[Jwk]>> {
    let document: KeyDocument = fetch(client, uri.as_str())?;
    let keys: Vec<Jwk> = document
        .keys
        .into_iter()
        .filter_map(|raw| {
            let Ok(key) = serde_json::from_value::<Jwk>(raw) else {
                return None;
            };
            (!matches!(key.algorithm, AlgorithmParameters::OctetKey(_))
                && DecodingKey::from_jwk(&key).is_ok())
            .then_some(key)
        })
        .collect();
    ensure!(
        !keys.is_empty(),
        "OIDC JWKS at {uri} has no supported public signing keys"
    );
    Ok(keys.into())
}

impl JwtKeys {
    fn discover(name: &str, client: HttpClient) -> Result<Self> {
        let jwks_uri = Discovery::load(name, &client)?.keys_url()?;
        let keys = fetch_keys(&client, &jwks_uri)?;
        let now = Instant::now();
        Ok(Self {
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
    read_document(client.get(uri)?)
}

fn read_document<T: DeserializeOwned>(response: Response<Body>) -> Result<T> {
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
pub(super) mod tests;
