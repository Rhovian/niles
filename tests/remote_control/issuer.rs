use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode, get_current_timestamp};
use openssl::asn1::Asn1Time;
use openssl::hash::MessageDigest;
use openssl::pkey::PKey;
use openssl::rsa::Rsa;
use openssl::ssl::{SslAcceptor, SslMethod};
use openssl::x509::{X509, X509NameBuilder, extension::SubjectAlternativeName};
use std::net::SocketAddr;
use std::thread;

const CLIENT_ID: &str = "phone-client";
const CLIENT_SECRET: &str = "fixture!+secret";
pub(super) const RESOURCE: &str = "https://niles.example.test/h/desk/api";

pub(super) struct Issuer {
    cert: PathBuf,
    key: EncodingKey,
    url: String,
    opaque: bool,
    address: SocketAddr,
    _root: tempfile::TempDir,
}

impl Issuer {
    pub(super) fn start() -> Self {
        Self::launch(false)
    }

    pub(super) fn opaque() -> Self {
        Self::launch(true)
    }

    fn launch(opaque: bool) -> Self {
        let root = tempfile::tempdir().unwrap();
        let cert = root.path().join("issuer.pem");
        let rsa = Rsa::generate(2048).unwrap();
        let jwks = json!({"keys": [{
            "kty": "RSA", "alg": "RS256",
            "n": URL_SAFE_NO_PAD.encode(rsa.n().to_vec()),
            "e": URL_SAFE_NO_PAD.encode(rsa.e().to_vec()),
        }]})
        .to_string();
        let key = EncodingKey::from_rsa_der(&rsa.private_key_to_der().unwrap());
        let tls_key = PKey::from_rsa(rsa).unwrap();
        let mut name = X509NameBuilder::new().unwrap();
        name.append_entry_by_text("CN", "localhost").unwrap();
        let name = name.build();
        let mut certificate = X509::builder().unwrap();
        certificate.set_version(2).unwrap();
        certificate.set_subject_name(&name).unwrap();
        certificate.set_issuer_name(&name).unwrap();
        certificate.set_pubkey(&tls_key).unwrap();
        certificate
            .set_not_before(&Asn1Time::days_from_now(0).unwrap())
            .unwrap();
        certificate
            .set_not_after(&Asn1Time::days_from_now(1).unwrap())
            .unwrap();
        let san = SubjectAlternativeName::new()
            .ip("127.0.0.1")
            .build(&certificate.x509v3_context(None, None))
            .unwrap();
        certificate.append_extension(san).unwrap();
        certificate.sign(&tls_key, MessageDigest::sha256()).unwrap();
        let certificate = certificate.build();
        atomic_write(&cert, certificate.to_pem().unwrap()).unwrap();
        let mut tls = SslAcceptor::mozilla_intermediate(SslMethod::tls()).unwrap();
        tls.set_private_key(&tls_key).unwrap();
        tls.set_certificate(&certificate).unwrap();
        let tls = tls.build();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let url = format!("https://{address}");
        let upstream = url.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let stream = stream.unwrap();
                // Drop wakes the listener with an empty connection.
                if stream.peek(&mut [0]).unwrap() == 0 {
                    break;
                }
                let mut tls = tls.accept(stream).unwrap();
                let mut request = BufReader::new(&mut tls);
                let mut line = String::new();
                request.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap().to_owned();
                let mut length = 0;
                loop {
                    line.clear();
                    assert_ne!(request.read_line(&mut line).unwrap(), 0);
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut data = vec![0; length];
                request.read_exact(&mut data).unwrap();
                let mut status = "200 OK";
                let body = match path.as_str() {
                    "/.well-known/openid-configuration" if opaque => json!({
                        "issuer": upstream, "introspection_endpoint": format!("{upstream}/introspect"),
                        "token_endpoint": format!("{upstream}/token"),
                    }).to_string(),
                    "/mismatch/.well-known/openid-configuration" => json!({"issuer": upstream, "jwks_uri": format!("{upstream}/keys")}).to_string(),
                    "/insecure/.well-known/openid-configuration" => json!({"issuer": format!("{upstream}/insecure"), "jwks_uri": "http://issuer.example/keys"}).to_string(),
                    "/keys" if !opaque => jwks.clone(),
                    "/token" | "/introspect" if opaque => opaque_response(&upstream, &path, &data, &mut status).to_string(),
                    _ => {
                        assert_eq!(path, "/.well-known/openid-configuration");
                        json!({"issuer": upstream, "jwks_uri": format!("{upstream}/keys")}).to_string()
                    }
                };
                write!(tls, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                tls.shutdown().unwrap();
            }
        });
        Self {
            url,
            cert,
            key,
            opaque,
            address,
            _root: root,
        }
    }

    pub(super) fn config(&self) -> String {
        let grants = ["api", "other", "unknown", "missing"]
            .map(|project| {
                format!("      - {{subject: alice, project: {project}, access: operate}}\n")
            })
            .concat();
        let intro = if self.opaque {
            format!(
                "  introspection: {{issuer: '{}', client_id: '{CLIENT_ID}', client_secret_env: NILES_TEST_IDP_SECRET}}\n",
                self.url
            )
        } else {
            String::new()
        };
        format!(
            "  web: {{bind: 127.0.0.1, resource: '{RESOURCE}'}}\n  issuers:\n    '{issuer}':\n{grants}{intro}",
            issuer = self.url
        )
    }

    pub(super) fn config_with_issuer_path(&self, path: &str) -> String {
        format!(
            "remote:\n{}",
            self.config()
                .replace(&self.url, &format!("{}/{path}", self.url))
        )
    }

    pub(super) fn trust(&self, command: &mut std::process::Command) {
        command
            .env("SSL_CERT_FILE", &self.cert)
            .env("NO_PROXY", "127.0.0.1")
            .env("NILES_TEST_IDP_SECRET", CLIENT_SECRET);
    }

    pub(super) fn authorization(&self) -> String {
        let token = encode(&Header::new(Algorithm::RS256), &json!({
            "iss": self.url, "sub": "alice", "aud": RESOURCE, "exp": get_current_timestamp().saturating_add(300),
        }), &self.key).unwrap();
        format!("Authorization: Bearer {token}\r\n")
    }
}

impl Drop for Issuer {
    fn drop(&mut self) {
        drop(TcpStream::connect(self.address));
    }
}

fn opaque_response(issuer: &str, path: &str, data: &[u8], status: &mut &str) -> serde_json::Value {
    let form: std::collections::BTreeMap<_, _> = url::form_urlencoded::parse(data).collect();
    assert_eq!(form["client_id"], CLIENT_ID);
    assert_eq!(form["client_secret"], CLIENT_SECRET);
    if path == "/token" {
        assert_eq!(form["resource"], RESOURCE);
        let (credential, valid, replacement) = if form["grant_type"] == "authorization_code" {
            assert_eq!(form["redirect_uri"], "https://niles.example.test/");
            assert_eq!(form["code_verifier"], "v".repeat(43));
            (&form["code"], "valid-code", "first-refresh")
        } else {
            assert_eq!(form["grant_type"], "refresh_token");
            (&form["refresh_token"], "first-refresh", "rotated-refresh")
        };
        if credential != valid {
            *status = "400 Bad Request";
            return json!({"error":"invalid_grant"});
        }
        return json!({"access_token":"valid-opaque", "token_type":"Bearer", "expires_in":300, "refresh_token":replacement, "id_token":"not-for-browser"});
    }
    let mut claims = json!({
        "active": true, "iss": issuer, "sub": "alice", "aud": [CLIENT_ID, RESOURCE],
        "client_id": CLIENT_ID, "exp": get_current_timestamp().saturating_add(300),
        "token_type": "Bearer",
    });
    match form["token"].as_ref() {
        "valid-opaque" => return claims,
        "single-audience" => claims["aud"] = json!(RESOURCE),
        "wrong-resource" => {
            claims["aud"] = json!([CLIENT_ID, "https://niles.example.test/h/other/api"])
        }
        "two-resources" => {
            claims["aud"] = json!([
                CLIENT_ID,
                RESOURCE,
                "https://niles.example.test/h/other/api"
            ])
        }
        "wrong-client" => {
            claims["client_id"] = json!("another-host-client");
            claims["aud"] = json!(RESOURCE);
        }
        "foreign-issuer" => claims["iss"] = json!("https://other.example.test"),
        "unknown-subject" => claims["sub"] = json!("other-user"),
        "future" => claims["nbf"] = json!(get_current_timestamp().saturating_add(300)),
        "wrong-type" => claims["token_type"] = json!("DPoP"),
        "empty-subject" => claims["sub"] = json!(""),
        "expired" => claims["exp"] = json!(get_current_timestamp().saturating_sub(1)),
        _ => return json!({"active": false}),
    }
    claims
}
