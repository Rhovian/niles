use std::{
    collections::BTreeMap,
    net::{Ipv4Addr, SocketAddrV4, UdpSocket},
};

use anyhow::{Context, Result, ensure};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use super::{ApiError, AppState, blocking};
use crate::config::user::FileWakeHost;

pub(super) struct Target {
    mac: [u8; 6],
    broadcast: Ipv4Addr,
}
pub(super) type Hosts = BTreeMap<String, Target>;

pub(super) fn configured(hosts: BTreeMap<String, FileWakeHost>) -> Result<Hosts> {
    hosts
        .into_iter()
        .map(|(id, host)| {
            ensure!(
                !id.is_empty()
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                    && id.as_bytes()[0].is_ascii_alphanumeric(),
                "invalid wake host ID: {id}"
            );
            // std parses hexadecimal bytes, but offers no MAC address parser.
            // https://doc.rust-lang.org/1.89.0/std/primitive.u8.html#method.from_str_radix
            let bytes = host
                .mac
                .split(':')
                .map(|part| {
                    ensure!(
                        part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_hexdigit()),
                        "MAC bytes must have two hexadecimal digits"
                    );
                    Ok(u8::from_str_radix(part, 16)?)
                })
                .collect::<Result<Vec<_>>>()?;
            let mac = bytes
                .try_into()
                .map_err(|_| anyhow::anyhow!("MAC must contain six bytes"))?;
            Ok((
                id,
                Target {
                    mac,
                    broadcast: host.broadcast,
                },
            ))
        })
        .collect()
}

impl Target {
    fn send(&self, destination: SocketAddrV4) -> Result<()> {
        let mut packet = [0xff; 102];
        for bytes in packet[6..].chunks_exact_mut(6) {
            bytes.copy_from_slice(&self.mac);
        }
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
        socket.set_broadcast(true)?;
        ensure!(
            socket.send_to(&packet, destination)? == packet.len(),
            "incomplete wake packet"
        );
        Ok(())
    }
}

pub(super) async fn wake(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(()): Json<()>,
) -> Result<StatusCode, ApiError> {
    blocking(move || {
        let Some(target) = state.wake_hosts.get(&id) else {
            return Ok(StatusCode::NOT_FOUND);
        };
        target
            .send(SocketAddrV4::new(target.broadcast, 9))
            .context("sending wake packet")?;
        Ok(StatusCode::NO_CONTENT)
    })
    .await
    .map_err(Into::into)
}

#[cfg(test)]
mod tests;
