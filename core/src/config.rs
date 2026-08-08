use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    pub humble_cookie: Option<String>,
    pub packt_token: Option<String>,
    pub manning_cookies: Option<String>,
    pub db_path: Option<PathBuf>,
}

impl Config {
    fn config_dir() -> Result<PathBuf> {
        let base = dirs::config_dir().context("could not determine OS config directory")?;
        Ok(base.join("library-inventory"))
    }

    fn config_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("config.toml"))
    }

    pub fn load() -> Result<Config> {
        Self::load_from(&Self::config_path()?)
    }

    pub fn save(&self) -> Result<()> {
        let dir = Self::config_dir()?;
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create config directory at {}", dir.display()))?;
        self.save_to(&Self::config_path()?)
    }

    fn load_from(path: &std::path::Path) -> Result<Config> {
        if !path.exists() {
            return Ok(Config::default());
        }
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config file at {}", path.display()))?;
        let config: Config = toml::from_str(&contents)
            .with_context(|| format!("failed to parse config file at {}", path.display()))?;
        Ok(config)
    }

    fn save_to(&self, path: &std::path::Path) -> Result<()> {
        let contents = toml::to_string_pretty(self)?;
        std::fs::write(path, contents)
            .with_context(|| format!("failed to write config file at {}", path.display()))?;

        // The config file holds session cookies/API tokens in plaintext, so
        // restrict it to owner-read/write only.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let permissions = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(path, permissions)
                .with_context(|| format!("failed to set permissions on {}", path.display()))?;
        }

        Ok(())
    }

    pub fn resolve_db_path(&self) -> PathBuf {
        if let Some(path) = &self.db_path {
            return path.clone();
        }
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("library-inventory")
            .join("library.db")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");

        let config = Config {
            humble_cookie: Some("abc123".to_string()),
            packt_token: Some("jwt.token.here".to_string()),
            manning_cookies: None,
            db_path: Some(PathBuf::from("/tmp/library.db")),
        };

        config.save_to(&config_path).unwrap();
        let parsed = Config::load_from(&config_path).unwrap();

        assert_eq!(parsed.humble_cookie, config.humble_cookie);
        assert_eq!(parsed.packt_token, config.packt_token);
        assert_eq!(parsed.manning_cookies, config.manning_cookies);
        assert_eq!(parsed.db_path, config.db_path);
    }

    #[test]
    fn load_from_missing_path_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let missing_path = dir.path().join("does-not-exist.toml");
        let config = Config::load_from(&missing_path).unwrap();
        assert!(config.humble_cookie.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn save_sets_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.toml");
        let config = Config::default();
        config.save_to(&config_path).unwrap();

        let mode = std::fs::metadata(&config_path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn resolve_db_path_uses_configured_override() {
        let config = Config {
            db_path: Some(PathBuf::from("/custom/path/library.db")),
            ..Default::default()
        };
        assert_eq!(config.resolve_db_path(), PathBuf::from("/custom/path/library.db"));
    }

    #[test]
    fn resolve_db_path_falls_back_to_data_dir() {
        let config = Config::default();
        let resolved = config.resolve_db_path();
        assert!(resolved.ends_with("library-inventory/library.db"));
    }

    #[test]
    fn default_config_has_no_secrets() {
        let config = Config::default();
        assert!(config.humble_cookie.is_none());
        assert!(config.packt_token.is_none());
        assert!(config.manning_cookies.is_none());
    }
}
