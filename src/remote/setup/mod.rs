use std::{collections::BTreeMap, process::Command};

use anyhow::{Context, Result, ensure};
use camino::Utf8Path;
use serde::{Deserialize, de::IgnoredAny};

mod preflight;
mod service;
#[cfg(test)]
mod tests;

const RECOVERY: &str = "Setup may have left completed steps in place. Inspect tailscale serve status and the service first. To remove setup's mounts/service, run: niles remote uninstall\nThen rerun the same setup command.";

pub(crate) fn uninstall() -> Result<()> {
    let serve: Option<Serve> =
        serde_json::from_str(&command("tailscale", &["serve", "status", "--json"])?)?;
    if let Some(serve) = &serve {
        serve.check_schema()?;
    }
    service::uninstall(&crate::util::home_dir()?)?;
    // The CLI's per-handler removal preserves other mounts but errors on absent ones.
    // https://github.com/tailscale/tailscale/blob/v1.96.2/cmd/tailscale/cli/serve_v2.go#L1339-L1396
    for mount in ["/api", "/"] {
        if serve.as_ref().is_some_and(|serve| {
            serve
                .web
                .iter()
                .any(|(host, web)| host.ends_with(":443") && web.handlers.contains_key(mount))
        }) {
            command(
                "tailscale",
                &[
                    "serve",
                    "--bg",
                    "--https=443",
                    &format!("--set-path={mount}"),
                    "off",
                ],
            )?;
        }
    }
    println!("Remote service and Serve mounts removed; push state retained.");
    Ok(())
}

pub(crate) fn doctor() -> Result<()> {
    preflight::check()?;
    Ok(())
}

pub(crate) fn run(
    web_dir: &Utf8Path,
    subject: Option<&str>,
    port: std::num::NonZeroU16,
) -> Result<()> {
    let plan = Plan::prepare(web_dir, port)?;
    plan.apply(subject).context(RECOVERY)?;
    // Host self-resolution/identity can fail on a valid setup; verification is advisory by policy.
    match plan.verify() {
        Ok(()) => println!("Verified {}/api/sessions", plan.origin),
        Err(error) => eprintln!(
            "warning: verification failed: {error:#}\nVerify from an allowed tailnet device: curl -i {}/api/sessions",
            plan.origin
        ),
    }
    println!(
        "Open {}/ on your phone; install/Add to Home Screen and enable notifications.\n\
        Manual: join host/phone to the tailnet, enable MagicDNS/HTTPS, restrict TCP 443 with ACLs/grants.\n\
        macOS: open-source tailscaled. Linux without login: loginctl enable-linger $USER. Service: {}",
        plan.origin, plan.service.path
    );
    Ok(())
}

struct Plan {
    mounts: [(&'static str, String); 2],
    origin: String,
    service: service::Service,
}
impl Plan {
    fn prepare(web_dir: &Utf8Path, port: std::num::NonZeroU16) -> Result<Self> {
        let web_dir = web_dir
            .canonicalize_utf8()
            .with_context(|| format!("resolve web directory {web_dir}"))?;
        ensure!(
            web_dir.join("index.html").is_file(),
            "web directory must contain index.html"
        );
        let origin = preflight::check()?;
        let serve: Option<Serve> =
            serde_json::from_str(&command("tailscale", &["serve", "status", "--json"])?)?;
        if let Some(serve) = serve {
            serve.check()?;
        }
        let service = service::Service::new(&crate::util::home_dir()?, port.get())?;
        ensure!(
            !service.path.try_exists()?,
            "service already exists at {}; inspect it before recovery:\n{}",
            service.path,
            RECOVERY
        );
        Ok(Self {
            mounts: [
                ("/", web_dir.into_string()),
                ("/api", format!("http://127.0.0.1:{port}/api")),
            ],
            origin,
            service,
        })
    }

    fn apply(&self, subject: Option<&str>) -> Result<()> {
        if let Some(subject) = subject
            && super::push::Store::load_optional()?.is_none()
        {
            super::push::init(subject)?;
        }
        self.service.install()?;
        // The Serve CLI accepts one mount per invocation; it owns TCP/Funnel conflict handling.
        // https://github.com/tailscale/tailscale/blob/v1.96.2/cmd/tailscale/cli/serve_v2.go
        for (mount, target) in &self.mounts {
            let status = Command::new("tailscale")
                .args([
                    "serve",
                    "--bg",
                    "--https=443",
                    &format!("--set-path={mount}"),
                    target.as_str(),
                ])
                .status()
                .context("run tailscale serve")?;
            ensure!(status.success(), "tailscale serve {mount} failed: {status}");
        }
        Ok(())
    }

    fn verify(&self) -> Result<()> {
        // curl owns bounded startup retries: https://curl.se/docs/manpage.html#--retry-connrefused
        let output = command(
            "curl",
            &[
                "-fsS",
                "-m10",
                "--retry",
                "5",
                "--retry-connrefused",
                "--write-out",
                "\n%{http_code}\n%header{cache-control}",
                &format!("{}/api/sessions", self.origin),
            ],
        )?;
        verify(&output)
    }
}

pub(super) fn command(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .stderr(std::process::Stdio::inherit())
        .output()
        .with_context(|| format!("run {program}; install it and ensure it is on PATH"))?;
    ensure!(
        output.status.success(),
        "{program} {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stdout)
    );
    Ok(String::from_utf8(output.stdout)?)
}

fn check_version(text: &str) -> Result<()> {
    // CLI has no minimum-version predicate: https://github.com/tailscale/tailscale/blob/v1.96.2/cmd/tailscale/cli/version.go
    let version = text.lines().next().context("missing Tailscale version")?;
    let numbers = version
        .split('.')
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .context("unsupported Tailscale version: expected major.minor.patch")?;
    ensure!(
        numbers.len() == 3,
        "Tailscale version must be major.minor.patch"
    );
    ensure!(
        numbers.as_slice() >= [1, 96, 2].as_slice(),
        "requires Tailscale >=1.96.2; update from https://tailscale.com/download (macOS: brew upgrade tailscale; sudo brew services restart tailscale)"
    );
    Ok(())
}

// Known top-level fields are explicit so renamed/new schema fields fail closed.
// https://github.com/tailscale/tailscale/blob/v1.96.2/ipn/serve.go
// Empty config is null: https://github.com/tailscale/tailscale/blob/v1.96.2/cmd/tailscale/cli/serve_legacy.go#L577
#[derive(Deserialize)]
struct Serve {
    #[serde(rename = "Web", default)]
    web: BTreeMap<String, Web>,
    #[serde(flatten)]
    other: BTreeMap<String, IgnoredAny>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Web {
    handlers: BTreeMap<String, IgnoredAny>,
}
impl Serve {
    fn check_schema(&self) -> Result<()> {
        ensure!(
            self.other.keys().all(|name| matches!(
                name.as_str(),
                "TCP" | "AllowFunnel" | "Foreground" | "Services"
            )),
            "unsupported Tailscale Serve schema"
        );
        Ok(())
    }

    fn check(&self) -> Result<()> {
        self.check_schema()?;
        ensure!(
            !self.web.iter().any(|(host, web)| host.ends_with(":443")
                && web
                    .handlers
                    .keys()
                    .any(|path| path == "/" || path == "/api" || path.starts_with("/api/"))),
            "Serve / or /api mapping already exists; refusing to overwrite it"
        );
        Ok(())
    }
}

// curl 7.84+ parses the final response's status/header; no application HTTP parser is needed.
// https://curl.se/docs/manpage.html#--write-out
fn verify(output: &str) -> Result<()> {
    let (payload, cache) = output
        .rsplit_once('\n')
        .context("missing curl Cache-Control output")?;
    let (body, status) = payload
        .rsplit_once('\n')
        .context("missing curl status output")?;
    ensure!(status == "200", "verification requires HTTP 200");
    ensure!(
        cache.eq_ignore_ascii_case("no-store"),
        "verification requires Cache-Control: no-store"
    );
    #[derive(Deserialize)]
    struct Sessions {
        api_version: u32,
        #[serde(rename = "sessions")]
        _sessions: Vec<IgnoredAny>,
    }
    let response: Sessions = serde_json::from_str(body)?;
    ensure!(
        response.api_version == super::API_VERSION,
        "unexpected API version"
    );
    Ok(())
}
