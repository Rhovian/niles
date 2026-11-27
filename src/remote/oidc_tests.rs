use super::*;
use crate::config::user::{FileConfig, FileMcp};
use crate::remote::auth::Auth;
use crate::remote::mcp::{ENDPOINT, LeadTools, METADATA, router};
use crate::session;
use crate::store::atomic_write;
use crate::test_support::{home_dir, temp_dir, write_executable_script};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode, header};
use axum::response::Response;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use camino::{Utf8Path, Utf8PathBuf};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode, get_current_timestamp};
use openssl::rsa::Rsa;
use serde_json::{Value, json};
use std::env;
use std::fs;
use std::iter::once;
use std::os::unix::fs::symlink;
use std::process::Command;
use std::sync::OnceLock;
use tower::ServiceExt;

pub(in crate::remote) const WEB_RESOURCE: &str = "https://niles.example.test/h/desk/api";
pub(in crate::remote) fn web_auth(resource: &str) -> Auth {
    Auth::new(web_verifier(), https_url(resource).unwrap(), None).unwrap()
}
pub(in crate::remote) fn web_verifier() -> Arc<Verifier> {
    let grants =
        [("alice", Access::Operate), ("bob", Access::Read)].map(|(subject, access)| FileGrant {
            subject: subject.into(),
            project: "project".into(),
            access,
        });
    Arc::new(Verifier {
        issuers: BTreeMap::from([(INTERACTIVE.into(), web_key().issuer(grants.into()))]),
    })
}
pub(in crate::remote) fn authorization(subject: &str, audience: &str) -> String {
    authorization_until(
        subject,
        audience,
        get_current_timestamp().saturating_add(300),
    )
}
pub(in crate::remote) fn authorization_until(subject: &str, audience: &str, exp: u64) -> String {
    format!(
        "Bearer {}",
        web_key().sign(&json!({
            "iss": INTERACTIVE, "sub": subject, "aud": audience, "exp": exp,
        }))
    )
}
pub(in crate::remote) fn principal(subject: &str) -> Principal {
    Principal {
        issuer: INTERACTIVE.into(),
        subject: subject.into(),
    }
}

fn web_key() -> &'static SigningKey {
    static KEY: OnceLock<SigningKey> = OnceLock::new();
    KEY.get_or_init(SigningKey::new)
}

const RESOURCE: &str = "https://mcp.example.test/mcp";
const INTERACTIVE: &str = "https://login.example.test";
const WORKLOAD: &str = "https://token.actions.githubusercontent.com";
const JOB: &str = "repo:owner/repo:ref:refs/heads/main";

struct SigningKey {
    key: Jwk,
    private: EncodingKey,
}

impl SigningKey {
    fn new() -> Self {
        let rsa = Rsa::generate(2048).unwrap();
        Self {
            key: serde_json::from_value(json!({
                "kty": "RSA", "kid": "signing", "use": "sig", "alg": "RS256",
                "n": URL_SAFE_NO_PAD.encode(rsa.n().to_vec()),
                "e": URL_SAFE_NO_PAD.encode(rsa.e().to_vec()),
            }))
            .unwrap(),
            private: EncodingKey::from_rsa_der(&rsa.private_key_to_der().unwrap()),
        }
    }

    fn sign(&self, claims: &Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some("signing".into());
        encode(&header, claims, &self.private).unwrap()
    }

    fn issuer(&self, grants: Vec<FileGrant>) -> Issuer {
        Issuer {
            grants,
            verification: Verification::Jwt(self.jwt_keys()),
        }
    }

    fn jwt_keys(&self) -> JwtKeys {
        JwtKeys {
            jwks_uri: Url::parse("https://unused.example.test").unwrap(),
            client: HttpClient::new().unwrap(),
            cache: Arc::new(Mutex::new(KeyCache {
                keys: Arc::from([self.key.clone()]),
                loaded_at: Instant::now(),
                attempted_at: Instant::now(),
            })),
        }
    }
}

async fn post(app: &Router, token: Option<&str>, method: &str, params: Value) -> Response {
    let mut request = Request::post(ENDPOINT)
        .header(header::HOST, "mcp.example.test")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(token) = token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let request = request
        .body(Body::from(
            json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string(),
        ))
        .unwrap();
    app.clone().oneshot(request).await.unwrap()
}

async fn body(response: Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 128 * 1024).await.unwrap()).unwrap()
}

// The subprocess can select a fixture tmux without changing PATH in other tests.
// Follow src/tmux/home/tests.rs, which also tests through process-local tmux settings.
fn fixture_process() -> Option<Utf8PathBuf> {
    const FIXTURES: &str = "NILES_MCP_TEST_FIXTURES";
    if let Some(path) = env::var_os(FIXTURES) {
        return Some(Utf8PathBuf::from_path_buf(path.into()).unwrap());
    }
    let root = temp_dir("mcp-auth");
    write_executable_script(
        &root.join("tmux"),
        r#"#!/bin/sh
[ "$1" != -u ] || shift
case "$1" in
  has-session) if [ ! -f "$NILES_MCP_TEST_FIXTURES/${3#=}/live" ]; then
    echo "can't find session" >&2; exit 1
  fi ;;
  display) project=${4#=}; printf '%s/%s\n' "$NILES_MCP_TEST_FIXTURES" "${project%:}" ;;
  list-windows) case "$5" in
    *pane_dead*) printf 'niles\t0\n' ;;
    *) printf '0\tniles\n' ;;
  esac ;;
  *) exit 1 ;;
esac
"#,
    );
    let path = env::var_os("PATH").unwrap();
    let path = env::join_paths(once(root.as_std_path().to_owned()).chain(env::split_paths(&path)))
        .unwrap();
    let test = format!(
        "{0}::mcp_verifies_bearers_and_enforces_issuer_project_and_access_grants",
        module_path!().split_once("::").unwrap().1
    );
    let output = Command::new(env::current_exe().unwrap())
        .args(["--exact", &test, "--nocapture"])
        .env("PATH", path)
        .env(FIXTURES, root.as_str())
        .output()
        .unwrap();
    assert!(
        output.status.success() && String::from_utf8_lossy(&output.stdout).contains("1 passed"),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    None
}

fn register_projects(root: &Utf8Path) {
    let registry = home_dir().join(".niles/projects");
    fs::create_dir_all(&registry).unwrap();
    for project in ["mcp-project", "another-project", "stopped"] {
        let workspace = root.join(project);
        fs::create_dir(&workspace).unwrap();
        symlink(&workspace, registry.join(project)).unwrap();
        if project == "stopped" {
            continue;
        }
        atomic_write(workspace.join("live"), "").unwrap();
        let dir = session::sessions_dir(&workspace).unwrap().join("session");
        fs::create_dir_all(&dir).unwrap();
        let transcript = dir.join("transcript");
        fs::create_dir(&transcript).unwrap();
        atomic_write(
            transcript.join("001.jsonl"),
            format!(
                "{}\n",
                json!({
                    "type":"message", "message":{"role":"assistant", "content":[
                        {"type":"text", "text":format!("x{}", "é".repeat(16 * 1024))}
                    ]}
                })
            ),
        )
        .unwrap();
        atomic_write(dir.join("session.json"), json!({
            "id": "session", "agent": "pi", "entry_skill": "lead",
            "created_at": chrono::Utc::now(), "workspace": workspace, "brief": dir.join("brief.md"),
            "session_link": {"family":"pi","session_dir":transcript},
        }).to_string()).unwrap();
    }
}

async fn call(app: &Router, token: &str, name: &str, arguments: Value) -> Response {
    post(
        app,
        Some(token),
        "tools/call",
        json!({"name": name, "arguments": arguments}),
    )
    .await
}

#[tokio::test]
async fn mcp_verifies_bearers_and_enforces_issuer_project_and_access_grants() {
    let Some(root) = fixture_process() else {
        return;
    };
    register_projects(&root);
    let path = root.join("config.yaml");
    let yaml = format!(
        "remote:\n  mcp:\n    resource: {RESOURCE}\n    interactive_issuer: {INTERACTIVE}\n  issuers:\n    {INTERACTIVE}:\n      - {{subject: alice, project: mcp-project, access: read}}\n    {WORKLOAD}:\n"
    ) + &["mcp-project", "stopped", "unregistered"]
        .map(|project| {
            format!("      - {{subject: '{JOB}', project: {project}, access: operate}}\n")
        })
        .concat();
    atomic_write(&path, &yaml).unwrap();
    let config = FileConfig::load(&path).unwrap();
    config.save(&path).unwrap();
    assert!(!fs::read_to_string(&path).unwrap().contains("port:"));
    let issuers = config.remote.issuers;
    let config = config.remote.mcp.unwrap();
    let interactive = SigningKey::new();
    let workload = SigningKey::new();
    let fixture_app = |config: FileMcp| {
        let grants = |name: &str| issuers.get(name).unwrap().clone();
        let verifier = Verifier {
            issuers: BTreeMap::from([
                (INTERACTIVE.into(), interactive.issuer(grants(INTERACTIVE))),
                (WORKLOAD.into(), workload.issuer(grants(WORKLOAD))),
            ]),
        };
        router(Arc::new(verifier), config, LeadTools::new()).unwrap()
    };
    let app = fixture_app(config);
    atomic_write(
        &path,
        yaml.replace(&format!("    interactive_issuer: {INTERACTIVE}\n"), ""),
    )
    .unwrap();
    let restarted = fixture_app(FileConfig::load(&path).unwrap().remote.mcp.unwrap());

    for (router, authorization_servers) in [(&app, Some(json!([INTERACTIVE]))), (&restarted, None)]
    {
        let metadata = router
            .clone()
            .oneshot(Request::get(METADATA).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(metadata.status(), StatusCode::OK);
        let metadata = body(metadata).await;
        assert_eq!(metadata.get("resource"), Some(&json!(RESOURCE)));
        assert_eq!(
            metadata.get("authorization_servers"),
            authorization_servers.as_ref()
        );
    }

    let exp = get_current_timestamp().saturating_add(300);
    let claims = json!({"iss": WORKLOAD, "sub": JOB, "aud": [RESOURCE], "exp": exp});
    let mut invalid = vec![
        ("bad signature", Some(interactive.sign(&claims))),
        ("missing token", None),
        ("malformed token", Some("invalid".into())),
    ];
    for (label, claim, value) in [
        (
            "unknown issuer",
            "iss",
            json!("https://unknown.example.test"),
        ),
        (
            "wrong audience",
            "aud",
            json!("https://other.example.test/mcp"),
        ),
        ("expired", "exp", json!(1)),
        ("not yet valid", "nbf", json!(exp)),
    ] {
        let mut fields = claims.clone();
        let _previous = fields.as_object_mut().unwrap().insert(claim.into(), value);
        invalid.push((label, Some(workload.sign(&fields))));
    }
    let mut multiple = claims.clone();
    *multiple.get_mut("aud").unwrap() = json!([RESOURCE, WEB_RESOURCE]);
    invalid.push(("multiple audiences", Some(workload.sign(&multiple))));
    let mut no_audience = claims.clone();
    assert!(no_audience.as_object_mut().unwrap().remove("aud").is_some());
    invalid.push(("missing audience", Some(workload.sign(&no_audience))));
    for (label, token) in invalid {
        let response = post(&app, token.as_deref(), "tools/list", json!({})).await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{label}");
        let mut expected =
            "Bearer resource_metadata=\"https://mcp.example.test/.well-known/oauth-protected-resource/mcp\""
                .to_owned();
        if token.is_some() {
            expected.push_str(", error=\"invalid_token\"");
        }
        assert_eq!(
            response.headers()[header::WWW_AUTHENTICATE],
            expected,
            "{label}"
        );
    }

    let job = workload.sign(&claims);
    let mut claims = json!({"iss": INTERACTIVE, "sub": "alice", "aud": RESOURCE, "exp": exp});
    let reader = interactive.sign(&claims);
    *claims.get_mut("sub").unwrap() = json!(JOB);
    let same_subject_other_issuer = interactive.sign(&claims);

    let tools = post(&app, Some(&reader), "tools/list", json!({})).await;
    assert_eq!(tools.status(), StatusCode::OK);
    let result = body(tools).await;
    let names: Vec<_> = result
        .pointer("/result/tools")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool.get("name").unwrap().as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "close_worker",
            "list_projects",
            "list_sessions",
            "message_lead",
            "message_worker",
            "read_lead_events",
            "read_worker_events",
            "start_lead",
            "stop_lead"
        ]
    );

    let project = json!({"project": "mcp-project"});
    let worker = json!({"project": "mcp-project", "worker_id": "worker"});
    for (name, arguments) in [
        ("start_lead", project.clone()),
        ("stop_lead", project.clone()),
        (
            "message_lead",
            json!({"project": "mcp-project", "text": "hello"}),
        ),
        (
            "message_worker",
            json!({"project": "mcp-project", "worker_id": "worker", "text": "hello"}),
        ),
        ("close_worker", worker.clone()),
    ] {
        assert_tool_error(
            call(&app, &reader, name, arguments).await,
            "An operate grant is required for project mcp-project",
        )
        .await;
    }
    let response = call(
        &app,
        &same_subject_other_issuer,
        "start_lead",
        project.clone(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(!response.headers().contains_key(header::WWW_AUTHENTICATE));
    assert_tool_error(
        call(
            &app,
            &job,
            "read_lead_events",
            json!({"project": "another-project"}),
        )
        .await,
        "A read grant is required for project another-project",
    )
    .await;

    let mut skewed = json!({"iss": WORKLOAD, "sub": JOB, "aud": RESOURCE, "exp": exp});
    let _previous = skewed.as_object_mut().unwrap().insert(
        "nbf".into(),
        json!(get_current_timestamp().saturating_add(30)),
    );
    let result = call(&app, &workload.sign(&skewed), "start_lead", project.clone()).await;
    assert_eq!(result.status(), StatusCode::OK);
    assert_eq!(
        body(result).await.pointer("/result/content/0/text"),
        Some(&json!("No saved manifest; start once at the desk"))
    );

    for (name, collection, id_field, expected) in [
        (
            "list_projects",
            "/result/structuredContent/projects",
            "name",
            "mcp-project",
        ),
        (
            "list_sessions",
            "/result/structuredContent/sessions",
            "id",
            "mcp-project:session",
        ),
    ] {
        let result = call(&app, &reader, name, json!({})).await;
        assert_eq!(result.status(), StatusCode::OK, "{name}");
        let data = body(result).await;
        let items = data.pointer(collection).unwrap().as_array().unwrap();
        assert_eq!(items.len(), 1, "{name}: {data}");
        assert_eq!(items.first().unwrap().get(id_field), Some(&json!(expected)));
    }

    let result = call(&app, &job, "read_lead_events", project.clone()).await;
    assert_eq!(result.status(), StatusCode::OK);
    let data = body(result).await;
    assert_eq!(
        data.pointer("/result/structuredContent/status/id"),
        Some(&json!("mcp-project:session"))
    );
    let snapshot = data.pointer("/result/structuredContent/snapshot").unwrap();
    assert_eq!(snapshot.get("truncated"), Some(&json!(true)));
    let text = snapshot.pointer("/turns/0/text").unwrap().as_str().unwrap();
    assert_eq!(text, format!("x{}", "é".repeat(12_287)));

    for (name, args, error) in [
        (
            "start_lead",
            json!({"project": "unregistered"}),
            "Unregistered project",
        ),
        (
            "read_lead_events",
            json!({"project": "unregistered"}),
            "Unregistered project",
        ),
        (
            "read_lead_events",
            json!({"project": "stopped"}),
            "No running lead session",
        ),
        ("read_worker_events", worker, "Unknown worker"),
        (
            "message_lead",
            json!({"project": "mcp-project", "text": "hello"}),
            "Agent pane is unavailable",
        ),
    ] {
        assert_tool_error(call(&app, &job, name, args).await, error).await;
    }
}

async fn assert_tool_error(response: Response, message: &str) {
    assert_eq!(response.status(), StatusCode::OK, "{message}");
    let data = body(response).await;
    assert_eq!(
        data.pointer("/result/isError"),
        Some(&json!(true)),
        "{data}"
    );
    assert_eq!(
        data.pointer("/result/content/0/text"),
        Some(&json!(message))
    );
}

#[path = "key_tests.rs"]
mod keys;
