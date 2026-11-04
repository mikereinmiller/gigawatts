use std::fmt;
use std::io;
use std::path::PathBuf;

use crate::engine::Paths;

const SYSTEMD_UNIT: &str = "gigawatts.service";
const LAUNCHD_LABEL: &str = "gigawatts.engine";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manager {
    Systemd,
    Launchd,
}

impl Manager {
    pub fn for_this_os() -> Manager {
        if cfg!(target_os = "macos") {
            Manager::Launchd
        } else {
            Manager::Systemd
        }
    }
}

pub struct Service {
    pub manager: Manager,
    pub home: PathBuf,
    pub executable: PathBuf,
    pub path: String,
    pub uid: u32,
}

#[derive(Debug)]
pub enum Error {
    File(PathBuf, io::Error),
    Command(String, io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::File(path, error) => write!(f, "{}: {error}", path.display()),
            Error::Command(command, error) => write!(f, "`{command}` failed: {error}"),
        }
    }
}

impl std::error::Error for Error {}

type Run<'a> = dyn FnMut(&[&str]) -> io::Result<()> + 'a;

impl Service {
    pub fn file(&self) -> PathBuf {
        match self.manager {
            Manager::Systemd => self
                .home
                .join(".config")
                .join("systemd")
                .join("user")
                .join(SYSTEMD_UNIT),
            Manager::Launchd => self
                .home
                .join("Library")
                .join("LaunchAgents")
                .join(format!("{LAUNCHD_LABEL}.plist")),
        }
    }

    pub fn contents(&self) -> String {
        match self.manager {
            Manager::Systemd => self.systemd_unit(),
            Manager::Launchd => self.launchd_plist(),
        }
    }

    pub fn install(&self, run: &mut Run) -> Result<(), Error> {
        let file = self.file();
        if let Some(folder) = file.parent() {
            std::fs::create_dir_all(folder).map_err(|error| Error::File(file.clone(), error))?;
        }
        let log_folder = Paths::new(&self.home).data_dir();
        std::fs::create_dir_all(&log_folder).map_err(|error| Error::File(log_folder, error))?;
        let replacing = file.exists();
        std::fs::write(&file, self.contents()).map_err(|error| Error::File(file.clone(), error))?;
        let started = self.start(run, &file);
        if started.is_err() && !replacing {
            let _ = std::fs::remove_file(&file);
        }
        started
    }

    fn start(&self, run: &mut Run, file: &std::path::Path) -> Result<(), Error> {
        match self.manager {
            Manager::Systemd => {
                checked(run, &["systemctl", "--user", "daemon-reload"])?;
                checked(
                    run,
                    &["systemctl", "--user", "enable", "--now", SYSTEMD_UNIT],
                )
            }
            Manager::Launchd => {
                let _not_loaded_yet = run(&["launchctl", "bootout", &self.launchd_target()]);
                checked(
                    run,
                    &[
                        "launchctl",
                        "bootstrap",
                        &self.launchd_domain(),
                        &file.to_string_lossy(),
                    ],
                )
            }
        }
    }

    pub fn uninstall(&self, run: &mut Run) -> Result<bool, Error> {
        let file = self.file();
        if !file.exists() {
            return Ok(false);
        }
        let stopped = match self.manager {
            Manager::Systemd => checked(
                run,
                &["systemctl", "--user", "disable", "--now", SYSTEMD_UNIT],
            ),
            Manager::Launchd => {
                let _not_loaded = run(&["launchctl", "bootout", &self.launchd_target()]);
                Ok(())
            }
        };
        stopped?;
        std::fs::remove_file(&file).map_err(|error| Error::File(file.clone(), error))?;
        if self.manager == Manager::Systemd {
            checked(run, &["systemctl", "--user", "daemon-reload"])?;
        }
        Ok(true)
    }

    fn launchd_domain(&self) -> String {
        format!("gui/{}", self.uid)
    }

    fn launchd_target(&self) -> String {
        format!("{}/{LAUNCHD_LABEL}", self.launchd_domain())
    }

    fn systemd_unit(&self) -> String {
        format!(
            "[Unit]
Description=Gigawatts engine, which keeps Claude Code usage history current

[Service]
ExecStart={} engine --foreground
Environment={}
StandardError=append:{}

[Install]
WantedBy=default.target
",
            systemd_quoted(&self.executable.to_string_lossy()),
            systemd_quoted(&format!("PATH={}", self.path)),
            Paths::new(&self.home)
                .log()
                .to_string_lossy()
                .replace('%', "%%"),
        )
    }

    fn launchd_plist(&self) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{LAUNCHD_LABEL}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{}</string>
		<string>engine</string>
		<string>--foreground</string>
	</array>
	<key>EnvironmentVariables</key>
	<dict>
		<key>PATH</key>
		<string>{}</string>
	</dict>
	<key>RunAtLoad</key>
	<true/>
	<key>ProcessType</key>
	<string>Background</string>
	<key>StandardErrorPath</key>
	<string>{}</string>
</dict>
</plist>
"#,
            xml_escaped(&self.executable.to_string_lossy()),
            xml_escaped(&self.path),
            xml_escaped(&Paths::new(&self.home).log().to_string_lossy()),
        )
    }
}

fn checked(run: &mut Run, args: &[&str]) -> Result<(), Error> {
    run(args).map_err(|error| Error::Command(args.join(" "), error))
}

fn systemd_quoted(text: &str) -> String {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    format!("\"{escaped}\"")
}

fn xml_escaped(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
