use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use gigawatts::service::{Manager, Service};

fn service(manager: Manager, home: &Path) -> Service {
    Service {
        manager,
        home: home.to_path_buf(),
        executable: PathBuf::from("/home/me/my tools/gigawatts"),
        path: "/home/me/.local/bin:/usr/bin:/bin".to_string(),
        uid: 501,
    }
}

struct Commands {
    ran: Vec<String>,
    failing: Option<&'static str>,
}

impl Commands {
    fn new() -> Commands {
        Commands {
            ran: Vec::new(),
            failing: None,
        }
    }

    fn run(&mut self, args: &[&str]) -> io::Result<()> {
        let command = args.join(" ");
        self.ran.push(command.clone());
        if self
            .failing
            .is_some_and(|failing| command.starts_with(failing))
        {
            return Err(io::Error::other("exited with status 5"));
        }
        Ok(())
    }
}

#[test]
fn the_systemd_unit_starts_the_engine_in_the_foreground_at_login() {
    let home = tempfile::tempdir().unwrap();

    let unit = service(Manager::Systemd, home.path()).contents();

    assert_eq!(
        unit,
        format!(
            r#"[Unit]
Description=Gigawatts engine, which keeps Claude Code usage history current

[Service]
ExecStart="/home/me/my tools/gigawatts" engine --foreground
Environment="PATH=/home/me/.local/bin:/usr/bin:/bin"
StandardError=append:{}/.local/share/gigawatts/engine.log

[Install]
WantedBy=default.target
"#,
            home.path().display()
        )
    );
}

#[test]
fn the_launchd_agent_starts_the_engine_in_the_foreground_at_login() {
    let home = tempfile::tempdir().unwrap();
    let service = service(Manager::Launchd, home.path());

    let plist = service.contents();

    let log = home.path().join(".local/share/gigawatts/engine.log");
    assert_eq!(
        plist,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>gigawatts.engine</string>
	<key>ProgramArguments</key>
	<array>
		<string>/home/me/my tools/gigawatts</string>
		<string>engine</string>
		<string>--foreground</string>
	</array>
	<key>EnvironmentVariables</key>
	<dict>
		<key>PATH</key>
		<string>/home/me/.local/bin:/usr/bin:/bin</string>
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
            log.display()
        )
    );
}

#[test]
fn installing_with_systemd_writes_the_unit_and_enables_it_now() {
    let home = tempfile::tempdir().unwrap();
    let service = service(Manager::Systemd, home.path());
    let mut commands = Commands::new();

    service.install(&mut |args| commands.run(args)).unwrap();

    let unit = home.path().join(".config/systemd/user/gigawatts.service");
    assert_eq!(service.file(), unit);
    assert_eq!(fs::read_to_string(&unit).unwrap(), service.contents());
    assert_eq!(
        commands.ran,
        [
            "systemctl --user daemon-reload",
            "systemctl --user enable --now gigawatts.service",
        ]
    );
}

#[test]
fn installing_with_launchd_replaces_any_loaded_agent() {
    let home = tempfile::tempdir().unwrap();
    let service = service(Manager::Launchd, home.path());
    let mut commands = Commands::new();
    commands.failing = Some("launchctl bootout");

    service.install(&mut |args| commands.run(args)).unwrap();

    let plist = home
        .path()
        .join("Library/LaunchAgents/gigawatts.engine.plist");
    assert_eq!(service.file(), plist);
    assert_eq!(fs::read_to_string(&plist).unwrap(), service.contents());
    assert_eq!(
        commands.ran,
        [
            "launchctl bootout gui/501/gigawatts.engine".to_string(),
            format!("launchctl bootstrap gui/501 {}", plist.display()),
        ]
    );
}

#[test]
fn a_failing_install_command_is_reported() {
    let home = tempfile::tempdir().unwrap();
    let service = service(Manager::Systemd, home.path());
    let mut commands = Commands::new();
    commands.failing = Some("systemctl --user enable");

    let error = service.install(&mut |args| commands.run(args)).unwrap_err();

    assert_eq!(
        error.to_string(),
        "`systemctl --user enable --now gigawatts.service` failed: exited with status 5"
    );
}

#[test]
fn uninstalling_with_systemd_disables_the_unit_and_removes_it() {
    let home = tempfile::tempdir().unwrap();
    let service = service(Manager::Systemd, home.path());
    service.install(&mut |_| Ok(())).unwrap();
    let mut commands = Commands::new();

    let removed = service.uninstall(&mut |args| commands.run(args)).unwrap();

    assert!(removed);
    assert!(!service.file().exists());
    assert_eq!(
        commands.ran,
        [
            "systemctl --user disable --now gigawatts.service",
            "systemctl --user daemon-reload",
        ]
    );
}

#[test]
fn uninstalling_with_launchd_unloads_the_agent_and_removes_it() {
    let home = tempfile::tempdir().unwrap();
    let service = service(Manager::Launchd, home.path());
    service.install(&mut |_| Ok(())).unwrap();
    let mut commands = Commands::new();
    commands.failing = Some("launchctl bootout");

    let removed = service.uninstall(&mut |args| commands.run(args)).unwrap();

    assert!(removed);
    assert!(!service.file().exists());
    assert_eq!(commands.ran, ["launchctl bootout gui/501/gigawatts.engine"]);
}

#[test]
fn uninstalling_when_nothing_is_installed_changes_nothing() {
    let home = tempfile::tempdir().unwrap();
    let service = service(Manager::Systemd, home.path());
    let mut commands = Commands::new();

    let removed = service.uninstall(&mut |args| commands.run(args)).unwrap();

    assert!(!removed);
    assert!(commands.ran.is_empty());
}

#[test]
fn a_failed_install_leaves_no_unit_behind() {
    let home = tempfile::tempdir().unwrap();
    let service = service(Manager::Systemd, home.path());
    let mut commands = Commands::new();
    commands.failing = Some("systemctl --user enable");

    service.install(&mut |args| commands.run(args)).unwrap_err();

    assert!(!service.file().exists());
}

#[test]
fn uninstalling_keeps_the_unit_when_systemd_cant_disable_it() {
    let home = tempfile::tempdir().unwrap();
    let service = service(Manager::Systemd, home.path());
    service.install(&mut |_| Ok(())).unwrap();
    let mut commands = Commands::new();
    commands.failing = Some("systemctl --user disable");

    let error = service
        .uninstall(&mut |args| commands.run(args))
        .unwrap_err();

    assert!(service.file().exists());
    assert_eq!(
        error.to_string(),
        "`systemctl --user disable --now gigawatts.service` failed: exited with status 5"
    );
}

#[test]
fn a_failed_reinstall_leaves_a_unit_in_place() {
    let home = tempfile::tempdir().unwrap();
    let service = service(Manager::Systemd, home.path());
    service.install(&mut |_| Ok(())).unwrap();
    let mut commands = Commands::new();
    commands.failing = Some("systemctl --user enable");

    service.install(&mut |args| commands.run(args)).unwrap_err();

    assert_eq!(
        fs::read_to_string(service.file()).unwrap(),
        service.contents()
    );
}

#[test]
fn installing_the_launchd_agent_creates_the_folder_its_log_goes_to() {
    let home = tempfile::tempdir().unwrap();
    let service = service(Manager::Launchd, home.path());

    service.install(&mut |_| Ok(())).unwrap();

    assert!(home.path().join(".local/share/gigawatts").is_dir());
}
