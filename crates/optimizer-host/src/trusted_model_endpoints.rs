use std::collections::HashSet;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

const REGISTRY_SCHEMA_VERSION: u32 = 1;
const ENDPOINT_SCHEMA_VERSION: u32 = 1;
const MAX_ENDPOINTS: usize = 32;
const MAX_REGISTRY_BYTES: u64 = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenAICompatibleMaxOutputTokenField {
    MaxTokens,
    MaxCompletionTokens,
}

impl OpenAICompatibleMaxOutputTokenField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MaxTokens => "max_tokens",
            Self::MaxCompletionTokens => "max_completion_tokens",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrustedModelEndpointCapabilities {
    pub json_object: bool,
    pub stream_usage: bool,
    pub max_output_token_field: OpenAICompatibleMaxOutputTokenField,
}

impl Default for TrustedModelEndpointCapabilities {
    fn default() -> Self {
        Self {
            json_object: false,
            stream_usage: false,
            max_output_token_field: OpenAICompatibleMaxOutputTokenField::MaxTokens,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrustedModelEndpoint {
    pub schema_version: u32,
    pub id: String,
    pub label: String,
    pub hostname: String,
    pub base_path: String,
    pub base_url: String,
    pub credential_ref: String,
    pub capabilities: TrustedModelEndpointCapabilities,
    pub revision: u64,
    pub created_at: String,
    pub updated_at: String,
}

impl TrustedModelEndpoint {
    pub fn chat_completions_path(&self) -> String {
        join_endpoint_path(&self.base_path, "chat/completions")
    }

    pub fn models_path(&self) -> String {
        join_endpoint_path(&self.base_path, "models")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisterTrustedModelEndpoint {
    pub schema_version: u32,
    pub label: String,
    pub hostname: String,
    pub base_path: String,
    pub confirmed_origin: String,
    #[serde(default)]
    pub capabilities: TrustedModelEndpointCapabilities,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TrustedModelEndpointFile {
    schema_version: u32,
    endpoints: Vec<TrustedModelEndpoint>,
}

#[derive(Debug)]
pub enum TrustedModelEndpointError {
    PathMustBeAbsolute(PathBuf),
    InvalidRegistry(&'static str),
    InvalidEndpoint(String),
    TrustConfirmationMismatch,
    NotFound,
    Io {
        operation: &'static str,
        source: std::io::Error,
    },
    Json(serde_json::Error),
    Time(time::error::Format),
}

impl TrustedModelEndpointError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidEndpoint(_) => "INVALID_TRUSTED_MODEL_ENDPOINT",
            Self::TrustConfirmationMismatch => "MODEL_ENDPOINT_TRUST_REQUIRED",
            Self::NotFound => "TRUSTED_MODEL_ENDPOINT_NOT_FOUND",
            Self::PathMustBeAbsolute(_)
            | Self::InvalidRegistry(_)
            | Self::Io { .. }
            | Self::Json(_)
            | Self::Time(_) => "TRUSTED_MODEL_ENDPOINTS_UNAVAILABLE",
        }
    }

    pub fn public_message(&self) -> String {
        match self {
            Self::InvalidEndpoint(message) => message.clone(),
            Self::TrustConfirmationMismatch => {
                "The confirmed HTTPS origin does not match the endpoint hostname".into()
            }
            Self::NotFound => "The trusted model endpoint is no longer registered".into(),
            _ => "Trusted model endpoints are unavailable".into(),
        }
    }
}

impl fmt::Display for TrustedModelEndpointError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PathMustBeAbsolute(path) => write!(
                formatter,
                "trusted model endpoint registry path must be absolute: {}",
                path.display()
            ),
            Self::InvalidRegistry(message) => {
                write!(
                    formatter,
                    "invalid trusted model endpoint registry: {message}"
                )
            }
            Self::InvalidEndpoint(message) => {
                write!(formatter, "invalid model endpoint: {message}")
            }
            Self::TrustConfirmationMismatch => {
                write!(
                    formatter,
                    "model endpoint trust confirmation does not match"
                )
            }
            Self::NotFound => write!(formatter, "trusted model endpoint is not registered"),
            Self::Io { operation, source } => write!(
                formatter,
                "failed to {operation} trusted model endpoint registry: {source}"
            ),
            Self::Json(error) => {
                write!(
                    formatter,
                    "invalid trusted model endpoint registry JSON: {error}"
                )
            }
            Self::Time(error) => write!(formatter, "failed to format endpoint time: {error}"),
        }
    }
}

impl std::error::Error for TrustedModelEndpointError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json(error) => Some(error),
            Self::Time(error) => Some(error),
            Self::PathMustBeAbsolute(_)
            | Self::InvalidRegistry(_)
            | Self::InvalidEndpoint(_)
            | Self::TrustConfirmationMismatch
            | Self::NotFound => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TrustedModelEndpointRegistry {
    path: Option<PathBuf>,
    endpoints: Vec<TrustedModelEndpoint>,
}

impl Default for TrustedModelEndpointRegistry {
    fn default() -> Self {
        Self::memory()
    }
}

impl TrustedModelEndpointRegistry {
    pub fn memory() -> Self {
        Self {
            path: None,
            endpoints: Vec::new(),
        }
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, TrustedModelEndpointError> {
        let path = path.as_ref();
        if !path.is_absolute() {
            return Err(TrustedModelEndpointError::PathMustBeAbsolute(
                path.to_path_buf(),
            ));
        }
        let parent = path
            .parent()
            .ok_or(TrustedModelEndpointError::InvalidRegistry(
                "registry has no parent directory",
            ))?;
        fs::create_dir_all(parent).map_err(|source| TrustedModelEndpointError::Io {
            operation: "create parent directory for",
            source,
        })?;
        let endpoints = load_registry(path)?;
        validate_entries(&endpoints)?;
        Ok(Self {
            path: Some(path.to_path_buf()),
            endpoints,
        })
    }

    pub fn list(&self) -> Vec<TrustedModelEndpoint> {
        self.endpoints.clone()
    }

    pub fn get(
        &self,
        endpoint_id: &str,
    ) -> Result<TrustedModelEndpoint, TrustedModelEndpointError> {
        self.endpoints
            .iter()
            .find(|endpoint| endpoint.id == endpoint_id)
            .cloned()
            .ok_or(TrustedModelEndpointError::NotFound)
    }

    pub fn contains_credential_reference(&self, reference: &str) -> bool {
        self.endpoints
            .iter()
            .any(|endpoint| endpoint.credential_ref == reference)
    }

    pub fn register(
        &mut self,
        input: RegisterTrustedModelEndpoint,
    ) -> Result<TrustedModelEndpoint, TrustedModelEndpointError> {
        if input.schema_version != ENDPOINT_SCHEMA_VERSION {
            return Err(TrustedModelEndpointError::InvalidEndpoint(
                "schemaVersion must equal 1".into(),
            ));
        }
        if self.endpoints.len() >= MAX_ENDPOINTS {
            return Err(TrustedModelEndpointError::InvalidEndpoint(format!(
                "at most {MAX_ENDPOINTS} trusted endpoints may be registered"
            )));
        }
        let label = input.label.trim();
        if label.is_empty() || label.len() > 128 || label.chars().any(char::is_control) {
            return Err(TrustedModelEndpointError::InvalidEndpoint(
                "label must contain 1-128 printable characters".into(),
            ));
        }
        let hostname = canonical_hostname(&input.hostname)?;
        let base_path = canonical_base_path(&input.base_path)?;
        let origin = format!("https://{hostname}");
        if input.confirmed_origin != origin {
            return Err(TrustedModelEndpointError::TrustConfirmationMismatch);
        }
        if self
            .endpoints
            .iter()
            .any(|endpoint| endpoint.hostname == hostname && endpoint.base_path == base_path)
        {
            return Err(TrustedModelEndpointError::InvalidEndpoint(
                "this HTTPS base URL is already registered".into(),
            ));
        }
        let now = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(TrustedModelEndpointError::Time)?;
        let id = format!("endpoint-{}", Uuid::new_v4().simple());
        let base_url = if base_path == "/" {
            origin
        } else {
            format!("{origin}{base_path}")
        };
        let endpoint = TrustedModelEndpoint {
            schema_version: ENDPOINT_SCHEMA_VERSION,
            credential_ref: format!("secret://providers/openai-compatible/{id}"),
            id,
            label: label.to_string(),
            hostname,
            base_path,
            base_url,
            capabilities: input.capabilities,
            revision: 1,
            created_at: now.clone(),
            updated_at: now,
        };
        validate_endpoint(&endpoint)?;
        let previous = self.endpoints.clone();
        self.endpoints.push(endpoint.clone());
        self.endpoints.sort_by(|left, right| {
            left.label
                .to_ascii_lowercase()
                .cmp(&right.label.to_ascii_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        if let Err(error) = self.persist() {
            self.endpoints = previous;
            return Err(error);
        }
        Ok(endpoint)
    }

    pub fn remove(
        &mut self,
        endpoint_id: &str,
    ) -> Result<TrustedModelEndpoint, TrustedModelEndpointError> {
        let index = self
            .endpoints
            .iter()
            .position(|endpoint| endpoint.id == endpoint_id)
            .ok_or(TrustedModelEndpointError::NotFound)?;
        let previous = self.endpoints.clone();
        let endpoint = self.endpoints.remove(index);
        if let Err(error) = self.persist() {
            self.endpoints = previous;
            return Err(error);
        }
        Ok(endpoint)
    }

    fn persist(&self) -> Result<(), TrustedModelEndpointError> {
        let Some(path) = self.path.as_ref() else {
            return Ok(());
        };
        let payload = serde_json::to_vec_pretty(&TrustedModelEndpointFile {
            schema_version: REGISTRY_SCHEMA_VERSION,
            endpoints: self.endpoints.clone(),
        })
        .map_err(TrustedModelEndpointError::Json)?;
        if payload.len() as u64 > MAX_REGISTRY_BYTES {
            return Err(TrustedModelEndpointError::InvalidRegistry(
                "registry exceeds size limit",
            ));
        }
        let parent = path
            .parent()
            .ok_or(TrustedModelEndpointError::InvalidRegistry(
                "registry has no parent directory",
            ))?;
        let temporary = parent.join(format!(
            ".trusted-model-endpoints-{}.tmp",
            Uuid::new_v4().simple()
        ));
        if let Err(source) = fs::write(&temporary, payload) {
            let _ = fs::remove_file(&temporary);
            return Err(TrustedModelEndpointError::Io {
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
            return Err(TrustedModelEndpointError::Io {
                operation: "flush temporary",
                source,
            });
        }
        replace_registry(path, &temporary)
    }
}

fn canonical_hostname(value: &str) -> Result<String, TrustedModelEndpointError> {
    let hostname = value.trim().to_ascii_lowercase();
    if hostname.is_empty()
        || hostname.len() > 253
        || hostname.starts_with('.')
        || hostname.ends_with('.')
        || !hostname.contains('.')
        || IpAddr::from_str(&hostname).is_ok()
    {
        return Err(TrustedModelEndpointError::InvalidEndpoint(
            "hostname must be a public ASCII DNS name with at least two labels".into(),
        ));
    }
    for label in hostname.split('.') {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(TrustedModelEndpointError::InvalidEndpoint(
                "hostname contains an invalid DNS label".into(),
            ));
        }
    }
    const RESERVED_SUFFIXES: [&str; 12] = [
        "localhost",
        "local",
        "internal",
        "home",
        "lan",
        "localdomain",
        "test",
        "example",
        "invalid",
        "onion",
        "arpa",
        "corp",
    ];
    if RESERVED_SUFFIXES
        .iter()
        .any(|suffix| hostname == *suffix || hostname.ends_with(&format!(".{suffix}")))
    {
        return Err(TrustedModelEndpointError::InvalidEndpoint(
            "hostname uses a local, private or reserved DNS suffix".into(),
        ));
    }
    Ok(hostname)
}

fn canonical_base_path(value: &str) -> Result<String, TrustedModelEndpointError> {
    let value = value.trim();
    if value.is_empty() || value == "/" {
        return Ok("/".into());
    }
    if value.len() > 256
        || !value.starts_with('/')
        || value.contains("//")
        || value.contains(['?', '#', '%', '\\', '@', ':'])
        || value.chars().any(char::is_whitespace)
    {
        return Err(TrustedModelEndpointError::InvalidEndpoint(
            "basePath must be an absolute path without credentials, escapes, query or fragment"
                .into(),
        ));
    }
    let canonical = value.trim_end_matches('/');
    for segment in canonical.trim_start_matches('/').split('/') {
        if segment.is_empty()
            || matches!(segment, "." | "..")
            || !segment.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~')
            })
        {
            return Err(TrustedModelEndpointError::InvalidEndpoint(
                "basePath contains an unsafe path segment".into(),
            ));
        }
    }
    Ok(canonical.to_string())
}

fn join_endpoint_path(base_path: &str, suffix: &str) -> String {
    if base_path == "/" {
        format!("/{suffix}")
    } else {
        format!("{base_path}/{suffix}")
    }
}

fn load_registry(path: &Path) -> Result<Vec<TrustedModelEndpoint>, TrustedModelEndpointError> {
    let backup = backup_path(path);
    if path.exists() && !path.is_file() {
        return Err(TrustedModelEndpointError::InvalidRegistry(
            "registry path is not a file",
        ));
    }
    let source = if path.is_file() {
        path
    } else if backup.is_file() {
        backup.as_path()
    } else {
        return Ok(Vec::new());
    };
    let metadata = fs::metadata(source).map_err(|source| TrustedModelEndpointError::Io {
        operation: "inspect",
        source,
    })?;
    if metadata.len() == 0 || metadata.len() > MAX_REGISTRY_BYTES {
        return Err(TrustedModelEndpointError::InvalidRegistry(
            "registry size is invalid",
        ));
    }
    let payload = fs::read(source).map_err(|source| TrustedModelEndpointError::Io {
        operation: "read",
        source,
    })?;
    let file: TrustedModelEndpointFile =
        serde_json::from_slice(&payload).map_err(TrustedModelEndpointError::Json)?;
    if file.schema_version != REGISTRY_SCHEMA_VERSION {
        return Err(TrustedModelEndpointError::InvalidRegistry(
            "unsupported schema version",
        ));
    }
    Ok(file.endpoints)
}

fn validate_entries(entries: &[TrustedModelEndpoint]) -> Result<(), TrustedModelEndpointError> {
    if entries.len() > MAX_ENDPOINTS {
        return Err(TrustedModelEndpointError::InvalidRegistry(
            "too many entries",
        ));
    }
    let mut ids = HashSet::new();
    let mut locations = HashSet::new();
    let mut references = HashSet::new();
    for endpoint in entries {
        validate_endpoint(endpoint)?;
        if !ids.insert(endpoint.id.as_str()) {
            return Err(TrustedModelEndpointError::InvalidRegistry(
                "duplicate endpoint ID",
            ));
        }
        if !locations.insert((endpoint.hostname.as_str(), endpoint.base_path.as_str())) {
            return Err(TrustedModelEndpointError::InvalidRegistry(
                "duplicate endpoint location",
            ));
        }
        if !references.insert(endpoint.credential_ref.as_str()) {
            return Err(TrustedModelEndpointError::InvalidRegistry(
                "duplicate credential reference",
            ));
        }
    }
    Ok(())
}

fn validate_endpoint(endpoint: &TrustedModelEndpoint) -> Result<(), TrustedModelEndpointError> {
    if endpoint.schema_version != ENDPOINT_SCHEMA_VERSION
        || !safe_endpoint_id(&endpoint.id)
        || endpoint.revision != 1
    {
        return Err(TrustedModelEndpointError::InvalidRegistry(
            "invalid endpoint identity or revision",
        ));
    }
    if endpoint.label.trim().is_empty()
        || endpoint.label.len() > 128
        || endpoint.label.chars().any(char::is_control)
    {
        return Err(TrustedModelEndpointError::InvalidRegistry(
            "invalid endpoint label",
        ));
    }
    let hostname = canonical_hostname(&endpoint.hostname)
        .map_err(|_| TrustedModelEndpointError::InvalidRegistry("invalid endpoint hostname"))?;
    let base_path = canonical_base_path(&endpoint.base_path)
        .map_err(|_| TrustedModelEndpointError::InvalidRegistry("invalid endpoint base path"))?;
    if hostname != endpoint.hostname || base_path != endpoint.base_path {
        return Err(TrustedModelEndpointError::InvalidRegistry(
            "endpoint location is not canonical",
        ));
    }
    let expected_base_url = if base_path == "/" {
        format!("https://{hostname}")
    } else {
        format!("https://{hostname}{base_path}")
    };
    let expected_reference = format!("secret://providers/openai-compatible/{}", endpoint.id);
    if endpoint.base_url != expected_base_url || endpoint.credential_ref != expected_reference {
        return Err(TrustedModelEndpointError::InvalidRegistry(
            "endpoint derived fields do not match its identity",
        ));
    }
    if OffsetDateTime::parse(&endpoint.created_at, &Rfc3339).is_err()
        || OffsetDateTime::parse(&endpoint.updated_at, &Rfc3339).is_err()
    {
        return Err(TrustedModelEndpointError::InvalidRegistry(
            "invalid endpoint timestamp",
        ));
    }
    Ok(())
}

fn safe_endpoint_id(value: &str) -> bool {
    value.starts_with("endpoint-")
        && value.len() == "endpoint-".len() + 32
        && value["endpoint-".len()..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn backup_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("trusted-model-endpoints.json");
    path.with_file_name(format!("{file_name}.bak"))
}

fn replace_registry(path: &Path, temporary: &Path) -> Result<(), TrustedModelEndpointError> {
    let backup = backup_path(path);
    if backup.exists()
        && let Err(source) = fs::remove_file(&backup)
    {
        let _ = fs::remove_file(temporary);
        return Err(TrustedModelEndpointError::Io {
            operation: "remove stale backup for",
            source,
        });
    }
    if path.exists()
        && let Err(source) = fs::rename(path, &backup)
    {
        let _ = fs::remove_file(temporary);
        return Err(TrustedModelEndpointError::Io {
            operation: "back up",
            source,
        });
    }
    if let Err(source) = fs::rename(temporary, path) {
        if backup.exists() {
            let _ = fs::rename(&backup, path);
        }
        let _ = fs::remove_file(temporary);
        return Err(TrustedModelEndpointError::Io {
            operation: "publish",
            source,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn input() -> RegisterTrustedModelEndpoint {
        RegisterTrustedModelEndpoint {
            schema_version: 1,
            label: "Acme Gateway".into(),
            hostname: "API.ACME.AI".into(),
            base_path: "/openai/v1/".into(),
            confirmed_origin: "https://api.acme.ai".into(),
            capabilities: TrustedModelEndpointCapabilities::default(),
        }
    }

    #[test]
    fn registration_canonicalizes_and_binds_derived_fields() {
        let mut registry = TrustedModelEndpointRegistry::memory();
        let endpoint = registry.register(input()).unwrap();
        assert_eq!(endpoint.hostname, "api.acme.ai");
        assert_eq!(endpoint.base_path, "/openai/v1");
        assert_eq!(endpoint.base_url, "https://api.acme.ai/openai/v1");
        assert_eq!(
            endpoint.chat_completions_path(),
            "/openai/v1/chat/completions"
        );
        assert_eq!(endpoint.models_path(), "/openai/v1/models");
        assert!(endpoint.credential_ref.ends_with(&endpoint.id));
        assert!(registry.contains_credential_reference(&endpoint.credential_ref));
    }

    #[test]
    fn registration_rejects_local_targets_and_mismatched_confirmation() {
        for hostname in [
            "127.0.0.1",
            "localhost",
            "model.internal",
            "model.local",
            "model.example",
            "singlelabel",
            "bad_name.example.com",
        ] {
            let mut candidate = input();
            candidate.hostname = hostname.into();
            candidate.confirmed_origin = format!("https://{hostname}");
            assert!(
                TrustedModelEndpointRegistry::memory()
                    .register(candidate)
                    .is_err()
            );
        }
        let mut candidate = input();
        candidate.confirmed_origin = "https://attacker.example.net".into();
        assert!(matches!(
            TrustedModelEndpointRegistry::memory().register(candidate),
            Err(TrustedModelEndpointError::TrustConfirmationMismatch)
        ));

        for base_path in [
            "v1",
            "//v1",
            "/v1/..",
            "/v1/%2e%2e",
            "/v1?tenant=other",
            "/v1#fragment",
            "/v1\\escape",
            "/v1:443",
        ] {
            let mut candidate = input();
            candidate.base_path = base_path.into();
            assert!(
                TrustedModelEndpointRegistry::memory()
                    .register(candidate)
                    .is_err()
            );
        }
    }

    #[test]
    fn registry_persists_strict_entries_and_removal() {
        let root = std::env::temp_dir().join(format!(
            "optimizer-trusted-endpoints-{}",
            Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("trusted-model-endpoints.json");
        let endpoint = {
            let mut registry = TrustedModelEndpointRegistry::open(&path).unwrap();
            registry.register(input()).unwrap()
        };
        let mut reopened = TrustedModelEndpointRegistry::open(&path).unwrap();
        assert_eq!(reopened.get(&endpoint.id).unwrap(), endpoint);
        assert_eq!(reopened.remove(&endpoint.id).unwrap().id, endpoint.id);
        assert!(reopened.list().is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn registry_rejects_tampering_and_recovers_a_missing_primary_from_backup() {
        let root = std::env::temp_dir().join(format!(
            "optimizer-trusted-endpoints-recovery-{}",
            Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("trusted-model-endpoints.json");
        let first = {
            let mut registry = TrustedModelEndpointRegistry::open(&path).unwrap();
            registry.register(input()).unwrap()
        };
        {
            let mut registry = TrustedModelEndpointRegistry::open(&path).unwrap();
            let mut second = input();
            second.label = "Second Gateway".into();
            second.hostname = "api.second.ai".into();
            second.confirmed_origin = "https://api.second.ai".into();
            registry.register(second).unwrap();
        }
        let valid = fs::read_to_string(&path).unwrap();
        fs::write(
            &path,
            valid.replace(
                "https://api.acme.ai/openai/v1",
                "https://attacker.example.net/openai/v1",
            ),
        )
        .unwrap();
        assert!(TrustedModelEndpointRegistry::open(&path).is_err());

        fs::remove_file(&path).unwrap();
        let recovered = TrustedModelEndpointRegistry::open(&path).unwrap();
        assert_eq!(recovered.list(), [first]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn registry_enforces_the_endpoint_count_limit() {
        let mut registry = TrustedModelEndpointRegistry::memory();
        for index in 0..MAX_ENDPOINTS {
            let hostname = format!("api-{index}.acme.ai");
            let mut candidate = input();
            candidate.label = format!("Gateway {index}");
            candidate.hostname = hostname.clone();
            candidate.confirmed_origin = format!("https://{hostname}");
            registry.register(candidate).unwrap();
        }
        let mut overflow = input();
        overflow.label = "Overflow".into();
        overflow.hostname = "overflow.acme.ai".into();
        overflow.confirmed_origin = "https://overflow.acme.ai".into();
        assert!(registry.register(overflow).is_err());
    }
}
