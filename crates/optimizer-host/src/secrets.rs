use std::collections::HashMap;
use std::fmt;
use std::sync::RwLock;

use zeroize::Zeroize;

const SECRET_PREFIX: &str = "secret://";
const MAX_REFERENCE_BYTES: usize = 512;
const MAX_SECRET_BYTES: usize = 2_560;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SecretReference(String);

impl SecretReference {
    pub fn parse(value: impl Into<String>) -> Result<Self, SecretStoreError> {
        let value = value.into();
        let Some(path) = value.strip_prefix(SECRET_PREFIX) else {
            return Err(SecretStoreError::InvalidReference);
        };
        if value.len() > MAX_REFERENCE_BYTES
            || path.is_empty()
            || !path.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'~' | b'/' | b'-')
            })
        {
            return Err(SecretStoreError::InvalidReference);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SecretReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

pub struct SecretValue(String);

impl SecretValue {
    pub fn new(value: impl Into<String>) -> Result<Self, SecretStoreError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(SecretStoreError::EmptySecret);
        }
        if value.len() > MAX_SECRET_BYTES {
            return Err(SecretStoreError::SecretTooLarge {
                maximum: MAX_SECRET_BYTES,
            });
        }
        Ok(Self(value))
    }

    pub fn expose_secret(&self) -> &str {
        &self.0
    }

    fn duplicate(&self) -> Result<Self, SecretStoreError> {
        Self::new(self.expose_secret())
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue([REDACTED])")
    }
}

impl Drop for SecretValue {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Debug)]
pub enum SecretStoreError {
    InvalidReference,
    EmptySecret,
    SecretTooLarge { maximum: usize },
    InvalidNamespace,
    InvalidEncoding,
    StateUnavailable,
    Platform { operation: &'static str, code: u32 },
}

impl SecretStoreError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidReference => "INVALID_SECRET_REFERENCE",
            Self::EmptySecret => "EMPTY_SECRET",
            Self::SecretTooLarge { .. } => "SECRET_TOO_LARGE",
            Self::InvalidNamespace => "INVALID_SECRET_NAMESPACE",
            Self::InvalidEncoding => "INVALID_SECRET_ENCODING",
            Self::StateUnavailable => "SECRET_STORE_UNAVAILABLE",
            Self::Platform { .. } => "SECRET_STORE_PLATFORM_ERROR",
        }
    }
}

impl fmt::Display for SecretStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidReference => formatter.write_str("invalid secret:// reference"),
            Self::EmptySecret => formatter.write_str("secret must not be empty"),
            Self::SecretTooLarge { maximum } => {
                write!(
                    formatter,
                    "secret exceeds the {maximum}-byte platform limit"
                )
            }
            Self::InvalidNamespace => formatter.write_str("invalid secret store namespace"),
            Self::InvalidEncoding => formatter.write_str("stored secret is not valid UTF-8"),
            Self::StateUnavailable => formatter.write_str("secret store state is unavailable"),
            Self::Platform { operation, code } => {
                write!(
                    formatter,
                    "platform secret store {operation} failed with OS code {code}"
                )
            }
        }
    }
}

impl std::error::Error for SecretStoreError {}

pub trait SecretStore: Send + Sync {
    fn put(&self, reference: &SecretReference, secret: SecretValue)
    -> Result<(), SecretStoreError>;

    fn resolve(&self, reference: &SecretReference)
    -> Result<Option<SecretValue>, SecretStoreError>;

    fn delete(&self, reference: &SecretReference) -> Result<bool, SecretStoreError>;

    fn contains(&self, reference: &SecretReference) -> Result<bool, SecretStoreError> {
        Ok(self.resolve(reference)?.is_some())
    }
}

#[derive(Default)]
pub struct MemorySecretStore {
    values: RwLock<HashMap<SecretReference, SecretValue>>,
}

impl SecretStore for MemorySecretStore {
    fn put(
        &self,
        reference: &SecretReference,
        secret: SecretValue,
    ) -> Result<(), SecretStoreError> {
        self.values
            .write()
            .map_err(|_| SecretStoreError::StateUnavailable)?
            .insert(reference.clone(), secret);
        Ok(())
    }

    fn resolve(
        &self,
        reference: &SecretReference,
    ) -> Result<Option<SecretValue>, SecretStoreError> {
        self.values
            .read()
            .map_err(|_| SecretStoreError::StateUnavailable)?
            .get(reference)
            .map(SecretValue::duplicate)
            .transpose()
    }

    fn delete(&self, reference: &SecretReference) -> Result<bool, SecretStoreError> {
        Ok(self
            .values
            .write()
            .map_err(|_| SecretStoreError::StateUnavailable)?
            .remove(reference)
            .is_some())
    }
}

#[cfg(windows)]
mod windows_store {
    use std::ptr;
    use std::slice;

    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_NOT_FOUND, GetLastError};
    use windows_sys::Win32::Security::Credentials::{
        CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree,
        CredReadW, CredWriteW,
    };

    use super::{SecretReference, SecretStore, SecretStoreError, SecretValue};

    pub struct WindowsCredentialStore {
        namespace: String,
    }

    impl WindowsCredentialStore {
        pub fn new(namespace: impl Into<String>) -> Result<Self, SecretStoreError> {
            let namespace = namespace.into();
            if namespace.is_empty()
                || namespace.len() > 64
                || !namespace
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
            {
                return Err(SecretStoreError::InvalidNamespace);
            }
            Ok(Self { namespace })
        }

        fn target(&self, reference: &SecretReference) -> Vec<u16> {
            format!("{}:{}", self.namespace, reference.as_str())
                .encode_utf16()
                .chain([0])
                .collect()
        }
    }

    impl SecretStore for WindowsCredentialStore {
        fn put(
            &self,
            reference: &SecretReference,
            secret: SecretValue,
        ) -> Result<(), SecretStoreError> {
            let mut target = self.target(reference);
            let mut username: Vec<u16> = "Optimizer System".encode_utf16().chain([0]).collect();
            let blob = secret.expose_secret().as_bytes();
            let credential = CREDENTIALW {
                Type: CRED_TYPE_GENERIC,
                TargetName: target.as_mut_ptr(),
                CredentialBlobSize: blob.len() as u32,
                CredentialBlob: blob.as_ptr().cast_mut(),
                Persist: CRED_PERSIST_LOCAL_MACHINE,
                UserName: username.as_mut_ptr(),
                ..Default::default()
            };
            if unsafe { CredWriteW(&credential, 0) } == 0 {
                return Err(platform_error("write"));
            }
            Ok(())
        }

        fn resolve(
            &self,
            reference: &SecretReference,
        ) -> Result<Option<SecretValue>, SecretStoreError> {
            let target = self.target(reference);
            let mut raw = ptr::null_mut();
            if unsafe { CredReadW(target.as_ptr(), CRED_TYPE_GENERIC, 0, &mut raw) } == 0 {
                let code = unsafe { GetLastError() };
                if code == ERROR_NOT_FOUND || code == ERROR_FILE_NOT_FOUND {
                    return Ok(None);
                }
                return Err(SecretStoreError::Platform {
                    operation: "read",
                    code,
                });
            }
            if raw.is_null() {
                return Err(SecretStoreError::Platform {
                    operation: "read",
                    code: 0,
                });
            }
            let credential = CredentialGuard(raw);
            let record = unsafe { &*credential.0 };
            if record.CredentialBlobSize == 0 || record.CredentialBlob.is_null() {
                return Err(SecretStoreError::InvalidEncoding);
            }
            let blob = unsafe {
                slice::from_raw_parts(record.CredentialBlob, record.CredentialBlobSize as usize)
            };
            let value =
                String::from_utf8(blob.to_vec()).map_err(|_| SecretStoreError::InvalidEncoding)?;
            Ok(Some(SecretValue::new(value)?))
        }

        fn delete(&self, reference: &SecretReference) -> Result<bool, SecretStoreError> {
            let target = self.target(reference);
            if unsafe { CredDeleteW(target.as_ptr(), CRED_TYPE_GENERIC, 0) } == 0 {
                let code = unsafe { GetLastError() };
                if code == ERROR_NOT_FOUND || code == ERROR_FILE_NOT_FOUND {
                    return Ok(false);
                }
                return Err(SecretStoreError::Platform {
                    operation: "delete",
                    code,
                });
            }
            Ok(true)
        }
    }

    struct CredentialGuard(*mut CREDENTIALW);

    impl Drop for CredentialGuard {
        fn drop(&mut self) {
            unsafe { CredFree(self.0.cast()) };
        }
    }

    fn platform_error(operation: &'static str) -> SecretStoreError {
        SecretStoreError::Platform {
            operation,
            code: unsafe { GetLastError() },
        }
    }
}

#[cfg(windows)]
pub use windows_store::WindowsCredentialStore;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_opaque_references_using_the_protocol_character_set() {
        let reference = SecretReference::parse("secret://providers/deepseek/default").unwrap();
        assert_eq!(reference.as_str(), "secret://providers/deepseek/default");
        for invalid in [
            "providers/deepseek/default",
            "secret://",
            "secret://providers/deep seek",
            "secret://providers/deepseek?key=value",
        ] {
            assert!(matches!(
                SecretReference::parse(invalid),
                Err(SecretStoreError::InvalidReference)
            ));
        }
    }

    #[test]
    fn memory_store_round_trips_without_exposing_secrets_through_debug() {
        let store = MemorySecretStore::default();
        let reference = SecretReference::parse("secret://providers/qwen/default").unwrap();
        let secret = SecretValue::new("test-secret-value").unwrap();
        assert_eq!(format!("{secret:?}"), "SecretValue([REDACTED])");
        store.put(&reference, secret).unwrap();
        assert!(store.contains(&reference).unwrap());
        let resolved = store.resolve(&reference).unwrap().unwrap();
        assert_eq!(resolved.expose_secret(), "test-secret-value");
        assert!(store.delete(&reference).unwrap());
        assert!(!store.contains(&reference).unwrap());
    }

    #[test]
    fn rejects_empty_and_platform_oversized_values() {
        assert!(matches!(
            SecretValue::new("  "),
            Err(SecretStoreError::EmptySecret)
        ));
        assert!(matches!(
            SecretValue::new("x".repeat(MAX_SECRET_BYTES + 1)),
            Err(SecretStoreError::SecretTooLarge { .. })
        ));
    }

    #[cfg(windows)]
    #[test]
    fn validates_windows_credential_namespaces_without_touching_the_os_store() {
        assert!(WindowsCredentialStore::new("OptimizerSystem").is_ok());
        assert!(matches!(
            WindowsCredentialStore::new("Optimizer System"),
            Err(SecretStoreError::InvalidNamespace)
        ));
    }
}
