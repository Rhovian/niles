use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use camino::{Utf8Path, Utf8PathBuf};
use openssl::{
    ec::{EcGroup, EcKey},
    nid::Nid,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
};
use web_push::{
    ContentEncoding, SubscriptionInfo, VapidSignatureBuilder, WebPushMessage, WebPushMessageBuilder,
};

const PUSH_TTL_SECONDS: u32 = 300;

#[derive(Serialize, Deserialize)]
struct Key {
    pem: String,
    subject: String,
}
#[derive(Serialize, Deserialize)]
struct Device {
    user: String,
    subscription: Subscription,
}
pub(crate) struct Store {
    root: Utf8PathBuf,
    key: Key,
}

fn private_dir(path: &Utf8Path) -> Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_dir() && meta.permissions().mode() & 0o077 == 0,
        "remote directory must be private and not a symlink"
    );
    Ok(())
}
fn read(path: &Utf8Path) -> Result<Vec<u8>> {
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() && meta.permissions().mode() & 0o777 == 0o600,
        "remote file must have mode 0600"
    );
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}
impl Store {
    pub(super) fn create(root: Utf8PathBuf, subject: &str) -> Result<Self> {
        let uri = url::Url::parse(subject)?;
        ensure!(
            matches!(uri.scheme(), "mailto" | "https"),
            "VAPID subject must be mailto: or https:"
        );
        private_dir(&root)?;
        private_dir(&root.join("subscriptions"))?;
        let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1)?;
        let pem = String::from_utf8(EcKey::generate(&group)?.private_key_to_pem()?)?;
        let key = Key {
            pem,
            subject: subject.to_owned(),
        };
        let bytes = serde_json::to_vec(&key)?;
        // create_new preserves existing device registrations' signing identity on repeated init.
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(root.join("vapid.json"))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        Ok(Self { root, key })
    }
    pub(crate) fn load_optional() -> Result<Option<Self>> {
        let root = crate::util::home_dir()?.join(".niles/remote");
        match fs::symlink_metadata(root.join("vapid.json")) {
            Ok(_) => Ok(Some(Self::load(root)?)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
    pub(super) fn load(root: Utf8PathBuf) -> Result<Self> {
        private_dir(&root)?;
        private_dir(&root.join("subscriptions"))?;
        let key = serde_json::from_slice(&read(&root.join("vapid.json"))?)?;
        let store = Self { root, key };
        store.public_key()?;
        Ok(store)
    }
    pub(super) fn public_key(&self) -> Result<String> {
        Ok(URL_SAFE_NO_PAD.encode(
            VapidSignatureBuilder::from_pem_no_sub(self.key.pem.as_bytes())?.get_public_key(),
        ))
    }
    pub(super) fn save(&self, user: String, subscription: Subscription) -> Result<()> {
        let path = self.path(&subscription);
        // tempfile 3.27 persist atomically replaces; readers never see partially written JSON.
        let mut temp = tempfile::NamedTempFile::new_in(self.root.join("subscriptions"))?;
        temp.write_all(&serde_json::to_vec(&Device { user, subscription })?)?;
        temp.as_file().sync_all()?;
        temp.persist(path)?;
        Ok(())
    }
    fn path(&self, subscription: &Subscription) -> Utf8PathBuf {
        self.root.join("subscriptions").join(format!(
            "{}.json",
            URL_SAFE_NO_PAD.encode(openssl::sha::sha256(subscription.0.endpoint.as_bytes()))
        ))
    }
    pub(super) fn subscriptions(&self, users: &[String]) -> Result<Vec<Subscription>> {
        let mut subscriptions = Vec::new();
        for entry in fs::read_dir(self.root.join("subscriptions"))? {
            let path = Utf8PathBuf::from_path_buf(entry?.path())
                .map_err(|_| anyhow::anyhow!("non-UTF8 subscription path"))?;
            if path.extension() == Some("json") {
                let device: Device = serde_json::from_slice(&read(&path)?)?;
                if users.contains(&device.user) {
                    subscriptions.push(device.subscription);
                }
            }
        }
        Ok(subscriptions)
    }
    pub(super) fn prune(&self, subscription: &Subscription) -> Result<()> {
        Ok(fs::remove_file(self.path(subscription))?)
    }
    pub(super) fn message(
        &self,
        subscription: &Subscription,
        payload: &[u8],
    ) -> Result<WebPushMessage> {
        let mut signature =
            VapidSignatureBuilder::from_pem(self.key.pem.as_bytes(), &subscription.0)?;
        signature.add_claim("sub", self.key.subject.clone());
        let mut builder = WebPushMessageBuilder::new(&subscription.0);
        builder.set_payload(ContentEncoding::Aes128Gcm, payload);
        builder.set_vapid_signature(signature.build()?);
        builder.set_ttl(PUSH_TTL_SECONDS);
        Ok(builder.build()?)
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(try_from = "SubscriptionInfo", into = "SubscriptionInfo")]
pub(crate) struct Subscription(pub(super) SubscriptionInfo);
impl From<Subscription> for SubscriptionInfo {
    fn from(value: Subscription) -> Self {
        value.0
    }
}
impl TryFrom<SubscriptionInfo> for Subscription {
    type Error = anyhow::Error;
    fn try_from(info: SubscriptionInfo) -> Result<Self> {
        let url = url::Url::parse(&info.endpoint)?;
        ensure!(
            url.scheme() == "https"
                && url.port_or_known_default() == Some(443)
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none(),
            "invalid push endpoint"
        );
        ensure!(
            matches!(
                url.host_str(),
                Some(
                    "fcm.googleapis.com"
                        | "updates.push.services.mozilla.com"
                        | "web.push.apple.com"
                )
            ),
            "unsupported push service"
        );
        let mut builder = WebPushMessageBuilder::new(&info);
        builder.set_payload(ContentEncoding::Aes128Gcm, b"validation");
        builder
            .build()
            .context("invalid subscription encryption keys")?;
        Ok(Self(info))
    }
}
