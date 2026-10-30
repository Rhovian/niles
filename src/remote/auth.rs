use std::{
    net::{Ipv4Addr, SocketAddr},
    process::Command,
    sync::Arc,
};

use anyhow::{Context, Result, ensure};
use serde::Deserialize;

type Lookup = Arc<dyn Fn(SocketAddr) -> Result<String> + Send + Sync>;

#[derive(Clone)]
pub(super) struct Relay {
    pub(super) node: String,
    pub(super) lookup: Lookup,
}

impl Relay {
    pub(super) fn new(node: Option<String>) -> Result<Self> {
        let node =
            node.context("remote.relay_node must be the relay's stable Tailscale node ID")?;
        ensure!(
            !node.is_empty() && node.trim() == node && !node.chars().any(char::is_control),
            "remote.relay_node is invalid"
        );
        Ok(Self {
            node,
            lookup: Arc::new(whois),
        })
    }

    pub(super) fn accepts(&self, peer: SocketAddr) -> Result<bool> {
        Ok((self.lookup)(peer)? == self.node)
    }
}

fn command(args: &[&str]) -> Result<String> {
    let output = Command::new("tailscale")
        .args(args)
        .output()
        .context("run tailscale")?;
    ensure!(
        output.status.success(),
        "tailscale {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8(output.stdout)?)
}

pub(super) fn tailnet_ip() -> Result<Ipv4Addr> {
    // The CLI selects the local IPv4; Rust parses its text representation.
    // https://github.com/tailscale/tailscale/blob/v1.96.2/cmd/tailscale/cli/ip.go
    let ip = command(&["ip", "-4"])?;
    ip.trim().parse().context("parse tailscale ip -4")
}

// WhoIs authenticates the TCP peer, never an HTTP forwarding header.
// The CLI emits JSON but offers no stable-node-ID-only output:
// https://github.com/tailscale/tailscale/blob/v1.96.2/cmd/tailscale/cli/whois.go
fn whois(peer: SocketAddr) -> Result<String> {
    #[derive(Deserialize)]
    struct WhoIs {
        #[serde(rename = "Node")]
        node: Node,
    }
    #[derive(Deserialize)]
    struct Node {
        #[serde(rename = "StableID")]
        stable_id: String,
    }
    let identity: WhoIs = serde_json::from_str(&command(&[
        "whois",
        "--json",
        "--proto=tcp",
        &peer.to_string(),
    ])?)?;
    ensure!(
        !identity.node.stable_id.is_empty(),
        "WhoIs returned an empty node ID"
    );
    Ok(identity.node.stable_id)
}
