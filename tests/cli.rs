use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

struct CliSandbox {
    _temp_dir: TempDir,
    home_dir: PathBuf,
    xdg_config_home: PathBuf,
    xdg_cache_home: PathBuf,
}

impl CliSandbox {
    fn new() -> Self {
        let temp_dir = TempDir::new().unwrap();
        let home_dir = temp_dir.path().join("home");
        let xdg_config_home = temp_dir.path().join("xdg-config");
        let xdg_cache_home = temp_dir.path().join("xdg-cache");

        for dir in [&home_dir, &xdg_config_home, &xdg_cache_home] {
            fs::create_dir_all(dir).unwrap();
        }

        Self {
            _temp_dir: temp_dir,
            home_dir,
            xdg_config_home,
            xdg_cache_home,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::cargo_bin("tt").unwrap();
        command
            .env("HOME", &self.home_dir)
            .env("XDG_CONFIG_HOME", &self.xdg_config_home)
            .env("XDG_CACHE_HOME", &self.xdg_cache_home)
            .env_remove("TICKTICK_CLIENT_ID")
            .env_remove("TICKTICK_CLIENT_SECRET")
            .env_remove("TICKTICK_REDIRECT_URI");
        command
    }

    fn config_dir(&self) -> PathBuf {
        #[cfg(target_os = "macos")]
        {
            self.home_dir
                .join("Library/Application Support/ticktick-cli")
        }

        #[cfg(not(target_os = "macos"))]
        {
            self.xdg_config_home.join("ticktick-cli")
        }
    }

    fn cache_dir(&self) -> PathBuf {
        #[cfg(target_os = "macos")]
        {
            self.home_dir.join("Library/Caches/ticktick-cli")
        }

        #[cfg(not(target_os = "macos"))]
        {
            self.xdg_cache_home.join("ticktick-cli")
        }
    }

    fn config_file(&self) -> PathBuf {
        self.config_dir().join("config.toml")
    }

    fn projects_cache_file(&self) -> PathBuf {
        self.cache_dir().join("projects.json")
    }

    fn task_projects_cache_file(&self) -> PathBuf {
        self.cache_dir().join("task-projects.json")
    }

    fn write_config(&self, expires_at: i64) {
        fs::create_dir_all(self.config_dir()).unwrap();
        fs::write(
            self.config_file(),
            format!(
                concat!(
                    "access_token = \"12345678abcdefgh\"\n",
                    "refresh_token = \"refresh-token\"\n",
                    "expires_at = {}\n"
                ),
                expires_at
            ),
        )
        .unwrap();
    }

    fn write_cache_files(&self) {
        fs::create_dir_all(self.cache_dir()).unwrap();
        fs::write(
            self.projects_cache_file(),
            r#"{"updated_at":4102444800,"projects":[]}"#,
        )
        .unwrap();
        fs::write(self.task_projects_cache_file(), r#"{"tasks":{}}"#).unwrap();
    }
}

#[test]
fn help_lists_core_commands() {
    let sandbox = CliSandbox::new();

    sandbox.command().arg("--help").assert().success().stdout(
        predicate::str::contains("Usage: tt <COMMAND>")
            .and(predicate::str::contains("login"))
            .and(predicate::str::contains("projects")),
    );
}

#[test]
fn status_reports_missing_auth_in_isolated_environment() {
    let sandbox = CliSandbox::new();

    sandbox.command().arg("status").assert().success().stdout(
        predicate::str::contains("Status: Not authenticated").and(predicate::str::contains(
            "Run 'tt auth login' to authenticate.",
        )),
    );
}

#[test]
fn status_masks_saved_access_token() {
    let sandbox = CliSandbox::new();
    sandbox.write_config(4_102_444_800);

    sandbox.command().arg("status").assert().success().stdout(
        predicate::str::contains("Status: Authenticated").and(predicate::str::contains(
            "Access Token: 12345678...abcdefgh",
        )),
    );
}

#[test]
fn logout_clears_saved_config_and_cache_files() {
    let sandbox = CliSandbox::new();
    sandbox.write_config(4_102_444_800);
    sandbox.write_cache_files();

    sandbox
        .command()
        .arg("logout")
        .assert()
        .success()
        .stdout(predicate::str::contains("Successfully logged out."));

    assert!(!sandbox.config_file().exists());
    assert!(!sandbox.projects_cache_file().exists());
    assert!(!sandbox.task_projects_cache_file().exists());
}

#[test]
fn list_requires_authentication_before_network_requests() {
    let sandbox = CliSandbox::new();

    sandbox
        .command()
        .arg("ls")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Not authenticated. Run 'tt auth login' first.",
        ));
}

#[test]
fn login_fails_fast_when_client_id_is_missing() {
    let sandbox = CliSandbox::new();

    sandbox
        .command()
        .arg("login")
        .assert()
        .failure()
        .stdout(predicate::str::contains("TickTick CLI Authentication"))
        .stderr(predicate::str::contains("Missing TICKTICK_CLIENT_ID"));
}
