use std::{fs, io::Write};

use anyhow::{Context, Result, bail, ensure};
use camino::{Utf8Path, Utf8PathBuf};

use super::command;

const SYSTEMD_UNIT: &str = "niles-remote.service";

pub(super) enum Manager {
    Launchd,
    Systemd,
}
pub(super) struct Service {
    pub(super) path: Utf8PathBuf,
    manager: Manager,
    pub(super) contents: String,
}
impl Service {
    pub(super) fn new(home: &Utf8Path, port: u16) -> Result<Self> {
        let manager = match std::env::consts::OS {
            "macos" => Manager::Launchd,
            "linux" => Manager::Systemd,
            os => bail!("remote setup has no service manager for {os}"),
        };
        let executable = crate::util::utf8_path(std::env::current_exe()?, "niles executable")?;
        let path_env = std::env::var("PATH").context("PATH is missing")?;
        Self::render(manager, home, &executable, &path_env, port)
    }

    // serde_json only renders JSON, not service-manager formats:
    // https://docs.rs/serde_json/1.0.151/serde_json/fn.to_string.html
    // Format boundaries: https://www.freedesktop.org/software/systemd/man/latest/systemd.syntax.html
    // https://www.freedesktop.org/software/systemd/man/latest/systemd.service.html#Command%20lines
    // https://www.manpagez.com/man/5/launchd.plist/
    pub(super) fn render(
        manager: Manager,
        home: &Utf8Path,
        executable: &Utf8Path,
        path_env: &str,
        port: u16,
    ) -> Result<Self> {
        let values = [home.as_str(), executable.as_str(), path_env];
        ensure!(
            values.iter().all(|s| !s.chars().any(char::is_control)),
            "service paths/environment contain control characters"
        );
        let (path, contents) = match manager {
            Manager::Launchd => {
                let [home, executable, path_env] = values.map(xml);
                (
                    Utf8PathBuf::from("Library/LaunchAgents/dev.niles.remote.plist"),
                    format!(
                        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<plist version=\"1.0\"><dict>\n\
                    <key>Label</key><string>dev.niles.remote</string>\n\
                    <key>ProgramArguments</key><array><string>{executable}</string><string>remote</string><string>--port</string><string>{port}</string></array>\n\
                    <key>EnvironmentVariables</key><dict><key>HOME</key><string>{home}</string><key>PATH</key><string>{path_env}</string></dict>\n\
                    <key>RunAtLoad</key><true/><key>KeepAlive</key><true/>\n\
                    <key>StandardErrorPath</key><string>{home}/.niles/remote-service.log</string>\n</dict></plist>\n"
                    ),
                )
            }
            Manager::Systemd => {
                let [home, executable, path_env] = values.map(systemd);
                (
                    Utf8PathBuf::from(".config/systemd/user/niles-remote.service"),
                    format!(
                        "[Unit]\nDescription=Niles remote API\n\n[Service]\n\
                    ExecStart=\"{}\" remote --port {port}\nEnvironment=\"HOME={home}\" \"PATH={path_env}\"\nRestart=always\nRestartSec=3\n\n[Install]\nWantedBy=default.target\n",
                        executable.replace('$', "$$")
                    ),
                )
            }
        };
        Ok(Self {
            path: home.join(path),
            manager,
            contents,
        })
    }

    pub(super) fn recovery(&self) -> String {
        let stop = match self.manager {
            Manager::Launchd => "launchctl bootout \"gui/$(id -u)/dev.niles.remote\"",
            Manager::Systemd => "systemctl --user disable --now niles-remote.service",
        };
        format!(
            "Setup may have left completed steps in place. Inspect tailscale serve status and this service first. To retry, remove only setup's mounts/service (skip absent entries):\ntailscale serve --bg --https=443 --set-path=/api off\ntailscale serve --bg --https=443 --set-path=/ off\n{stop}\nrm -- '{}'\nThen rerun the same setup command.",
            self.path.as_str().replace('\'', "'\\''")
        )
    }

    pub(super) fn install(&self) -> Result<()> {
        fs::create_dir_all(self.path.parent().context("service path has no parent")?)?;
        let mut file = fs::File::create_new(&self.path)?;
        let result = (|| {
            file.write_all(self.contents.as_bytes())?;
            self.start()
        })();
        if let Err(error) = result {
            fs::remove_file(&self.path)
                .with_context(|| format!("remove service file after {error:#}"))?;
            return Err(error);
        }
        Ok(())
    }

    fn start(&self) -> Result<()> {
        match self.manager {
            Manager::Launchd => {
                let domain = format!("gui/{}", command("id", &["-u"])?.trim().parse::<u32>()?);
                command("launchctl", &["bootstrap", &domain, self.path.as_str()])?;
            }
            Manager::Systemd => {
                command("systemctl", &["--user", "daemon-reload"])?;
                command("systemctl", &["--user", "enable", "--now", SYSTEMD_UNIT])?;
            }
        }
        Ok(())
    }
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn systemd(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
}
