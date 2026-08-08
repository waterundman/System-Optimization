use std::fmt;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

const USER_CONFIG_SCHEMA_VERSION: u32 = 1;
const MAX_CONFIG_BYTES: u64 = 16 * 1024;
const MAX_LOCALE_LEN: usize = 32;
const DEFAULT_LOCALE: &str = "zh-CN";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct UserConfigFile {
    schema_version: u32,
    locale: String,
}

#[derive(Debug)]
pub enum UserConfigError {
    PathMustBeAbsolute(PathBuf),
    InvalidConfig(&'static str),
    InvalidLocale(String),
    Io {
        operation: &'static str,
        source: std::io::Error,
    },
    Json(serde_json::Error),
}

impl UserConfigError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidLocale(_) => "USER_LOCALE_INVALID",
            Self::PathMustBeAbsolute(_)
            | Self::InvalidConfig(_)
            | Self::Io { .. }
            | Self::Json(_) => "USER_CONFIG_UNAVAILABLE",
        }
    }

    pub fn public_message(&self) -> String {
        match self {
            Self::PathMustBeAbsolute(path) => {
                format!(
                    "user config path must be absolute: {}",
                    path.display()
                )
            }
            Self::InvalidConfig(message) => {
                format!("invalid user config: {message}")
            }
            Self::InvalidLocale(locale) => {
                format!("locale '{locale}' is not a supported BCP-47 tag")
            }
            Self::Io { operation, source } => {
                format!("failed to {operation} user config: {source}")
            }
            Self::Json(error) => {
                format!("invalid user config JSON: {error}")
            }
        }
    }
}

impl fmt::Display for UserConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.public_message())
    }
}

impl std::error::Error for UserConfigError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json(error) => Some(error),
            Self::PathMustBeAbsolute(_)
            | Self::InvalidConfig(_)
            | Self::InvalidLocale(_) => None,
        }
    }
}

/// Host-owned user configuration registry. Persists the active UI locale
/// (and future user-level preferences) to a JSON file outside any project
/// package. The WebView never reads or writes this file directly — it goes
/// through the `set_user_locale` / `get_user_locale` Tauri commands so the
/// host stays the authoritative security boundary for user preferences.
#[derive(Debug, Clone)]
pub struct UserConfigRegistry {
    path: Option<PathBuf>,
    locale: String,
}

impl Default for UserConfigRegistry {
    fn default() -> Self {
        Self::memory()
    }
}

impl UserConfigRegistry {
    pub fn memory() -> Self {
        Self {
            path: None,
            locale: DEFAULT_LOCALE.into(),
        }
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, UserConfigError> {
        let path = path.as_ref();
        if !path.is_absolute() {
            return Err(UserConfigError::PathMustBeAbsolute(path.to_path_buf()));
        }
        let parent = path.parent().ok_or(UserConfigError::InvalidConfig(
            "config path has no parent directory",
        ))?;
        fs::create_dir_all(parent).map_err(|source| UserConfigError::Io {
            operation: "create parent directory for",
            source,
        })?;
        let locale = load_config(path)?;
        Ok(Self {
            path: Some(path.to_path_buf()),
            locale,
        })
    }

    pub fn locale(&self) -> &str {
        &self.locale
    }

    pub fn set_locale(&mut self, locale: &str) -> Result<String, UserConfigError> {
        validate_locale(locale)?;
        let previous = self.locale.clone();
        self.locale = locale.to_owned();
        if let Err(error) = self.persist() {
            self.locale = previous;
            return Err(error);
        }
        Ok(self.locale.clone())
    }

    fn persist(&self) -> Result<(), UserConfigError> {
        let Some(path) = self.path.as_ref() else {
            return Ok(());
        };
        let payload = serde_json::to_vec_pretty(&UserConfigFile {
            schema_version: USER_CONFIG_SCHEMA_VERSION,
            locale: self.locale.clone(),
        })
        .map_err(UserConfigError::Json)?;
        if payload.len() as u64 > MAX_CONFIG_BYTES {
            return Err(UserConfigError::InvalidConfig(
                "config exceeds size limit",
            ));
        }
        let parent = path.parent().ok_or(UserConfigError::InvalidConfig(
            "config path has no parent directory",
        ))?;
        let temporary = parent.join(format!(".user-config-{}.tmp", Uuid::new_v4().simple()));
        if let Err(source) = fs::write(&temporary, payload) {
            let _ = fs::remove_file(&temporary);
            return Err(UserConfigError::Io {
                operation: "write temporary",
                source,
            });
        }
        if let Err(source) = OpenOptions::new()
            .write(true)
            .open(&temporary)
            .and_then(|file| file.sync_all())
        {
            let _ = fs::remove_file(&temporary);
            return Err(UserConfigError::Io {
                operation: "flush temporary",
                source,
            });
        }
        replace_config(path, &temporary)
    }
}

fn load_config(path: &Path) -> Result<String, UserConfigError> {
    let backup = backup_path(path);
    if path.exists() && !path.is_file() {
        return Err(UserConfigError::InvalidConfig(
            "config path is not a file",
        ));
    }
    let source = if path.is_file() {
        path
    } else if backup.is_file() {
        backup.as_path()
    } else {
        return Ok(DEFAULT_LOCALE.into());
    };
    let metadata = fs::metadata(source).map_err(|source| UserConfigError::Io {
        operation: "inspect",
        source,
    })?;
    if metadata.len() == 0 || metadata.len() > MAX_CONFIG_BYTES {
        return Err(UserConfigError::InvalidConfig(
            "config size is invalid",
        ));
    }
    let payload = fs::read(source).map_err(|source| UserConfigError::Io {
        operation: "read",
        source,
    })?;
    let file: UserConfigFile =
        serde_json::from_slice(&payload).map_err(UserConfigError::Json)?;
    if file.schema_version != USER_CONFIG_SCHEMA_VERSION {
        return Err(UserConfigError::InvalidConfig(
            "unsupported schema version",
        ));
    }
    validate_locale(&file.locale)?;
    Ok(file.locale)
}

/// Validate that `locale` is a safe BCP-47-style tag (e.g. `zh-CN`, `en-US`).
/// Accepts 2-3 lowercase ASCII letters, a hyphen, and 2-3 uppercase ASCII
/// letters. This keeps the value within a known shape before it touches disk
/// or the frontend `setLocale` call.
fn validate_locale(locale: &str) -> Result<(), UserConfigError> {
    if locale.len() > MAX_LOCALE_LEN {
        return Err(UserConfigError::InvalidLocale(locale.into()));
    }
    let parts = locale.split('-').collect::<Vec<_>>();
    if parts.len() != 2 {
        return Err(UserConfigError::InvalidLocale(locale.into()));
    }
    let language = parts[0];
    let region = parts[1];
    let language_valid = (2..=3).contains(&language.len())
        && language.bytes().all(|byte| byte.is_ascii_lowercase());
    let region_valid = (2..=3).contains(&region.len())
        && region.bytes().all(|byte| byte.is_ascii_uppercase());
    if !language_valid || !region_valid {
        return Err(UserConfigError::InvalidLocale(locale.into()));
    }
    Ok(())
}

fn backup_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("user-config.json");
    path.with_file_name(format!("{file_name}.bak"))
}

fn replace_config(path: &Path, temporary: &Path) -> Result<(), UserConfigError> {
    let backup = backup_path(path);
    if backup.exists()
        && let Err(source) = fs::remove_file(&backup)
    {
        let _ = fs::remove_file(temporary);
        return Err(UserConfigError::Io {
            operation: "remove stale backup for",
            source,
        });
    }
    if path.exists() {
        fs::rename(path, &backup).map_err(|source| UserConfigError::Io {
            operation: "stage previous",
            source,
        })?;
    }
    if let Err(source) = fs::rename(temporary, path) {
        if backup.exists() {
            let _ = fs::rename(&backup, path);
        }
        let _ = fs::remove_file(temporary);
        return Err(UserConfigError::Io {
            operation: "publish",
            source,
        });
    }
    if backup.exists() {
        let _ = fs::remove_file(&backup);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "optimizer-user-config-{}-{nonce}-{}",
                std::process::id(),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn memory_registry_defaults_to_zh_cn() {
        let registry = UserConfigRegistry::memory();
        assert_eq!(registry.locale(), "zh-CN");
    }

    #[test]
    fn set_locale_persists_and_reloads() {
        let temp = TempDirectory::new();
        let config_path = temp.0.join("user-config.json");
        let mut registry = UserConfigRegistry::open(&config_path).unwrap();
        assert_eq!(registry.locale(), "zh-CN");
        let saved = registry.set_locale("en-US").unwrap();
        assert_eq!(saved, "en-US");
        assert_eq!(registry.locale(), "en-US");

        let reloaded = UserConfigRegistry::open(&config_path).unwrap();
        assert_eq!(reloaded.locale(), "en-US");
    }

    #[test]
    fn set_locale_rejects_invalid_tags() {
        let mut registry = UserConfigRegistry::memory();
        let invalid = ["", "zh", "zh_CN", "zh-cn", "ZH-CN", "zh-CN-extra", "1234"];
        for locale in invalid {
            let error = registry.set_locale(locale).unwrap_err();
            assert_eq!(error.code(), "USER_LOCALE_INVALID");
        }
        assert_eq!(registry.locale(), "zh-CN");
    }

    #[test]
    fn set_locale_rolls_back_on_persistence_failure() {
        let temp = TempDirectory::new();
        let config_path = temp.0.join("user-config.json");
        let mut registry = UserConfigRegistry::open(&config_path).unwrap();
        registry.set_locale("en-US").unwrap();
        // Simulate a persistence failure by pointing the path at a directory
        // that was removed after open.
        let bad_path = temp.0.join("vanish").join("user-config.json");
        registry.path = Some(bad_path.clone());
        let error = registry.set_locale("ja-JP").unwrap_err();
        assert_eq!(error.code(), "USER_CONFIG_UNAVAILABLE");
        // The in-memory locale must roll back to the last persisted value.
        assert_eq!(registry.locale(), "en-US");
    }

    #[test]
    fn rejects_tampered_config_files() {
        let temp = TempDirectory::new();
        let path = temp.0.join("user-config.json");
        fs::write(&path, br#"{"schemaVersion":1,"locale":"en-US","extra":true}"#).unwrap();
        assert!(matches!(
            UserConfigRegistry::open(&path),
            Err(UserConfigError::Json(_))
        ));
        fs::write(&path, br#"{"schemaVersion":99,"locale":"en-US"}"#).unwrap();
        assert!(matches!(
            UserConfigRegistry::open(path),
            Err(UserConfigError::InvalidConfig(_))
        ));
    }
}
