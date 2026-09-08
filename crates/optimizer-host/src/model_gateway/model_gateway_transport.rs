use std::collections::BTreeMap;
#[cfg(windows)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicU8, Ordering};

use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelHttpResponseHead {
    pub status: u16,
    pub headers: BTreeMap<String, String>,
}

impl ModelHttpResponseHead {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

pub trait ModelResponseSink {
    fn begin(&mut self, head: ModelHttpResponseHead) -> Result<(), ModelGatewayError>;
    fn chunk(&mut self, chunk: &[u8]) -> Result<(), ModelGatewayError>;
}

pub trait ModelTransport: Send + Sync {
    fn execute(
        &self,
        request: &PreparedModelRequest,
        secret: Option<&SecretValue>,
        cancellation: &ModelCancellation,
        sink: &mut dyn ModelResponseSink,
    ) -> Result<(), ModelGatewayError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedModelRequest {
    pub(crate) request_id: String,
    pub(crate) provider_id: ModelProviderId,
    pub(crate) host: String,
    pub(crate) path: String,
    pub(crate) port: u16,
    pub(crate) use_tls: bool,
    pub(crate) use_system_proxy: bool,
    pub(crate) method: ModelHttpMethod,
    pub(crate) body: Vec<u8>,
    pub(crate) timeout_ms: u32,
    pub(crate) max_response_bytes: usize,
}

impl PreparedModelRequest {
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    pub fn provider_id(&self) -> ModelProviderId {
        self.provider_id
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn use_tls(&self) -> bool {
        self.use_tls
    }

    pub fn use_system_proxy(&self) -> bool {
        self.use_system_proxy
    }

    pub fn method(&self) -> &'static str {
        self.method.as_str()
    }

    pub fn body(&self) -> &[u8] {
        &self.body
    }

    pub fn timeout_ms(&self) -> u32 {
        self.timeout_ms
    }

    pub fn max_response_bytes(&self) -> usize {
        self.max_response_bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModelHttpMethod {
    Get,
    Post,
}

impl ModelHttpMethod {
    fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

#[derive(Debug)]
pub struct ModelCancellation {
    state: AtomicU8,
    #[cfg(windows)]
    native_request_handle: AtomicUsize,
}

impl ModelCancellation {
    pub(crate) fn new() -> Self {
        Self {
            state: AtomicU8::new(CONTROL_ACTIVE),
            #[cfg(windows)]
            native_request_handle: AtomicUsize::new(0),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(
            self.state.load(Ordering::Acquire),
            CONTROL_CANCELLED | CONTROL_TIMED_OUT
        )
    }

    pub fn cancellation_error(&self, provider_id: ModelProviderId) -> Option<ModelGatewayError> {
        match self.state.load(Ordering::Acquire) {
            CONTROL_CANCELLED => Some(
                ModelGatewayError::new(
                    "PROVIDER_CANCELLED",
                    format!("{} request was cancelled", provider_id.label()),
                )
                .for_provider(provider_id),
            ),
            CONTROL_TIMED_OUT => {
                let mut error = ModelGatewayError::new(
                    "PROVIDER_TIMEOUT",
                    format!("{} request timed out", provider_id.label()),
                )
                .for_provider(provider_id);
                error.retriable = true;
                Some(error)
            }
            _ => None,
        }
    }

    pub(crate) fn cancel(&self) -> bool {
        let changed = self
            .state
            .compare_exchange(
                CONTROL_ACTIVE,
                CONTROL_CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok();
        if changed {
            self.close_native_request();
        }
        changed
    }

    pub(crate) fn time_out(&self) {
        if self
            .state
            .compare_exchange(
                CONTROL_ACTIVE,
                CONTROL_TIMED_OUT,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            self.close_native_request();
        }
    }

    pub(crate) fn complete(&self) {
        let _ = self.state.compare_exchange(
            CONTROL_ACTIVE,
            CONTROL_COMPLETE,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        self.close_native_request();
    }

    #[cfg(windows)]
    pub(crate) fn install_native_request(
        &self,
        handle: *mut core::ffi::c_void,
        provider_id: ModelProviderId,
    ) -> Result<(), ModelGatewayError> {
        use windows_sys::Win32::Networking::WinHttp::WinHttpCloseHandle;

        let raw = handle as usize;
        if self
            .native_request_handle
            .compare_exchange(0, raw, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            unsafe { WinHttpCloseHandle(handle) };
            return Err(ModelGatewayError::new(
                "PROVIDER_PROTOCOL",
                "native request handle was registered twice",
            )
            .for_provider(provider_id));
        }
        if let Some(error) = self.cancellation_error(provider_id) {
            self.close_native_request();
            return Err(error);
        }
        Ok(())
    }

    #[cfg(windows)]
    pub(crate) fn close_native_request(&self) {
        use windows_sys::Win32::Networking::WinHttp::WinHttpCloseHandle;

        let raw = self.native_request_handle.swap(0, Ordering::AcqRel);
        if raw != 0 {
            unsafe { WinHttpCloseHandle(raw as *mut core::ffi::c_void) };
        }
    }

    #[cfg(not(windows))]
    pub(crate) fn close_native_request(&self) {}
}

pub(crate) struct ActiveRequestGuard {
    pub(crate) active: Arc<Mutex<HashMap<String, Arc<ModelCancellation>>>>,
    pub(crate) request_id: String,
    pub(crate) control: Arc<ModelCancellation>,
}

impl Drop for ActiveRequestGuard {
    fn drop(&mut self) {
        self.control.complete();
        // Best-effort cleanup: errors cannot be propagated from `Drop`. A
        // poisoned lock means another holder panicked; the registry entry
        // may leak but is keyed by unique ids and therefore harmless.
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.request_id);
        } else {
            eprintln!(
                "[optimizer-host] model_gateway: ActiveRequestGuard could not unregister {} (poisoned mutex)",
                self.request_id
            );
        }
    }
}
