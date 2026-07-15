use super::model_gateway::{
    ModelCancellation, ModelGatewayError, ModelResponseSink, ModelTransport, PreparedModelRequest,
};
use crate::SecretValue;

#[derive(Debug, Default)]
pub struct NativeModelTransport;

impl NativeModelTransport {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(not(windows))]
impl ModelTransport for NativeModelTransport {
    fn execute(
        &self,
        request: &PreparedModelRequest,
        _secret: Option<&SecretValue>,
        _cancellation: &ModelCancellation,
        _sink: &mut dyn ModelResponseSink,
    ) -> Result<(), ModelGatewayError> {
        Err(ModelGatewayError::transport(
            format!(
                "{} native model transport is not implemented on this platform",
                request.provider_id()
            ),
            false,
        ))
    }
}

#[cfg(windows)]
mod windows {
    use std::collections::BTreeMap;
    use std::ffi::c_void;
    use std::ptr;

    use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, GetLastError};
    use windows_sys::Win32::Networking::WinHttp::{
        WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_FLAG_SECURE,
        WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_RAW_HEADERS_CRLF, WINHTTP_QUERY_STATUS_CODE,
        WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders,
        WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetTimeouts,
    };
    use zeroize::Zeroize;

    use super::*;
    impl ModelTransport for NativeModelTransport {
        fn execute(
            &self,
            request: &PreparedModelRequest,
            secret: Option<&SecretValue>,
            cancellation: &ModelCancellation,
            sink: &mut dyn ModelResponseSink,
        ) -> Result<(), ModelGatewayError> {
            if let Some(error) = cancellation.cancellation_error(request.provider_id()) {
                return Err(error);
            }
            let agent = wide_null("OptimizerSystem/0.1");
            let session = InternetHandle::new(
                unsafe {
                    WinHttpOpen(
                        agent.as_ptr(),
                        if request.use_tls() {
                            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY
                        } else {
                            WINHTTP_ACCESS_TYPE_NO_PROXY
                        },
                        ptr::null(),
                        ptr::null(),
                        0,
                    )
                },
                "open HTTP session",
                request,
                cancellation,
            )?;
            let timeout = i32::try_from(request.timeout_ms()).map_err(|_| {
                ModelGatewayError::transport("provider timeout exceeds platform range", false)
            })?;
            ensure_bool(
                unsafe { WinHttpSetTimeouts(session.raw(), timeout, timeout, timeout, timeout) },
                "configure HTTP timeouts",
                request,
                cancellation,
            )?;

            let host = wide_null(request.host());
            let connection = InternetHandle::new(
                unsafe { WinHttpConnect(session.raw(), host.as_ptr(), request.port(), 0) },
                "open provider connection",
                request,
                cancellation,
            )?;
            let verb = wide_null(request.method());
            let path = wide_null(request.path());
            let request_handle = unsafe {
                WinHttpOpenRequest(
                    connection.raw(),
                    verb.as_ptr(),
                    path.as_ptr(),
                    ptr::null(),
                    ptr::null(),
                    ptr::null(),
                    if request.use_tls() {
                        WINHTTP_FLAG_SECURE
                    } else {
                        0
                    },
                )
            };
            if request_handle.is_null() {
                return Err(last_error("open provider request", request, cancellation));
            }
            cancellation.install_native_request(request_handle, request.provider_id())?;
            let _request_registration = RequestRegistration(cancellation);

            let mut headers = Vec::new();
            if let Some(secret) = secret {
                headers.extend("Authorization: Bearer ".encode_utf16());
                headers.extend(secret.expose_secret().encode_utf16());
                headers.extend("\r\n".encode_utf16());
            }
            if !request.body().is_empty() {
                headers.extend("Content-Type: application/json\r\n".encode_utf16());
            }
            headers.extend("Accept: application/json, text/event-stream\r\n".encode_utf16());
            let header_length = u32::try_from(headers.len()).map_err(|_| {
                ModelGatewayError::transport("provider request headers are too large", false)
            })?;
            let body_length = u32::try_from(request.body().len()).map_err(|_| {
                ModelGatewayError::transport("provider request body is too large", false)
            })?;
            let body = if request.body().is_empty() {
                ptr::null_mut()
            } else {
                request.body().as_ptr().cast_mut().cast()
            };
            let send_result = unsafe {
                WinHttpSendRequest(
                    request_handle,
                    headers.as_ptr(),
                    header_length,
                    body,
                    body_length,
                    body_length,
                    0,
                )
            };
            headers.zeroize();
            ensure_bool(send_result, "send provider request", request, cancellation)?;
            ensure_bool(
                unsafe { WinHttpReceiveResponse(request_handle, ptr::null_mut()) },
                "receive provider response",
                request,
                cancellation,
            )?;

            let status = query_status(request_handle, request, cancellation)?;
            let headers = query_headers(request_handle, request, cancellation)?;
            sink.begin(super::super::model_gateway::ModelHttpResponseHead { status, headers })?;

            let mut total = 0_usize;
            let mut buffer = vec![0_u8; 16 * 1024];
            loop {
                if let Some(error) = cancellation.cancellation_error(request.provider_id()) {
                    return Err(error);
                }
                let mut bytes_read = 0_u32;
                ensure_bool(
                    unsafe {
                        WinHttpReadData(
                            request_handle,
                            buffer.as_mut_ptr().cast(),
                            buffer.len() as u32,
                            &mut bytes_read,
                        )
                    },
                    "read provider response",
                    request,
                    cancellation,
                )?;
                if bytes_read == 0 {
                    break;
                }
                let bytes_read = bytes_read as usize;
                total = total.saturating_add(bytes_read);
                if total > request.max_response_bytes() {
                    return Err(ModelGatewayError::transport(
                        "provider response exceeded the hard size limit",
                        false,
                    ));
                }
                sink.chunk(&buffer[..bytes_read])?;
            }
            Ok(())
        }
    }

    struct InternetHandle(*mut c_void);

    impl InternetHandle {
        fn new(
            raw: *mut c_void,
            operation: &'static str,
            request: &PreparedModelRequest,
            cancellation: &ModelCancellation,
        ) -> Result<Self, ModelGatewayError> {
            if raw.is_null() {
                Err(last_error(operation, request, cancellation))
            } else {
                Ok(Self(raw))
            }
        }

        fn raw(&self) -> *mut c_void {
            self.0
        }
    }

    impl Drop for InternetHandle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { WinHttpCloseHandle(self.0) };
            }
        }
    }

    struct RequestRegistration<'a>(&'a ModelCancellation);

    impl Drop for RequestRegistration<'_> {
        fn drop(&mut self) {
            self.0.close_native_request();
        }
    }

    fn query_status(
        handle: *mut c_void,
        request: &PreparedModelRequest,
        cancellation: &ModelCancellation,
    ) -> Result<u16, ModelGatewayError> {
        let mut status = 0_u32;
        let mut bytes = std::mem::size_of::<u32>() as u32;
        ensure_bool(
            unsafe {
                WinHttpQueryHeaders(
                    handle,
                    WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                    ptr::null(),
                    (&mut status as *mut u32).cast(),
                    &mut bytes,
                    ptr::null_mut(),
                )
            },
            "read provider status",
            request,
            cancellation,
        )?;
        u16::try_from(status).map_err(|_| {
            ModelGatewayError::transport("provider returned an invalid HTTP status", false)
        })
    }

    fn query_headers(
        handle: *mut c_void,
        request: &PreparedModelRequest,
        cancellation: &ModelCancellation,
    ) -> Result<BTreeMap<String, String>, ModelGatewayError> {
        let mut bytes = 0_u32;
        let first = unsafe {
            WinHttpQueryHeaders(
                handle,
                WINHTTP_QUERY_RAW_HEADERS_CRLF,
                ptr::null(),
                ptr::null_mut(),
                &mut bytes,
                ptr::null_mut(),
            )
        };
        if first == 0 && unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER {
            return Err(last_error(
                "measure provider headers",
                request,
                cancellation,
            ));
        }
        let mut wide = vec![0_u16; (bytes as usize).div_ceil(2)];
        ensure_bool(
            unsafe {
                WinHttpQueryHeaders(
                    handle,
                    WINHTTP_QUERY_RAW_HEADERS_CRLF,
                    ptr::null(),
                    wide.as_mut_ptr().cast(),
                    &mut bytes,
                    ptr::null_mut(),
                )
            },
            "read provider headers",
            request,
            cancellation,
        )?;
        let length = wide
            .iter()
            .position(|value| *value == 0)
            .unwrap_or(wide.len());
        let raw = String::from_utf16(&wide[..length]).map_err(|_| {
            ModelGatewayError::transport("provider headers are not valid UTF-16", false)
        })?;
        let mut output = BTreeMap::new();
        for line in raw.lines().skip(1) {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim();
            output
                .entry(name)
                .and_modify(|current: &mut String| {
                    current.push_str(", ");
                    current.push_str(value);
                })
                .or_insert_with(|| value.to_string());
        }
        Ok(output)
    }

    fn ensure_bool(
        result: i32,
        operation: &'static str,
        request: &PreparedModelRequest,
        cancellation: &ModelCancellation,
    ) -> Result<(), ModelGatewayError> {
        if result == 0 {
            Err(last_error(operation, request, cancellation))
        } else {
            Ok(())
        }
    }

    fn last_error(
        operation: &'static str,
        request: &PreparedModelRequest,
        cancellation: &ModelCancellation,
    ) -> ModelGatewayError {
        if let Some(error) = cancellation.cancellation_error(request.provider_id()) {
            return error;
        }
        let code = unsafe { GetLastError() };
        ModelGatewayError::transport(
            format!("provider network operation {operation} failed with OS code {code}"),
            true,
        )
    }

    fn wide_null(value: &str) -> Vec<u16> {
        value.encode_utf16().chain([0]).collect()
    }
}
