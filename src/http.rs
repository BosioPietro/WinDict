//! Minimal HTTPS GET on top of WinHTTP.
//!
//! WinHTTP ships with Windows, uses the system TLS stack and proxy settings,
//! and costs nothing in binary size compared to pulling in a Rust HTTP/TLS stack.

use std::ffi::c_void;
use std::sync::OnceLock;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Networking::WinHttp::*;

/// A WinHTTP handle that is closed on drop.
struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

// WinHTTP session handles are thread-safe.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

fn session() -> Result<&'static Handle, String> {
    static SESSION: OnceLock<Handle> = OnceLock::new();
    let session = SESSION.get_or_init(|| unsafe {
        let h = WinHttpOpen(
            w!("WinDict/0.1"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        );
        if !h.is_null() {
            // resolve, connect, send, receive (ms)
            let _ = WinHttpSetTimeouts(h, 4000, 4000, 4000, 8000);
        }
        Handle(h)
    });
    if session.0.is_null() {
        Err("Could not initialise the network stack.".into())
    } else {
        Ok(session)
    }
}

/// Performs `GET https://{host}{path}` and returns the status code and body.
pub fn get(host: &str, path: &str) -> Result<(u32, Vec<u8>), String> {
    let offline = || "Couldn't reach the dictionary. Check your connection.".to_string();
    unsafe {
        let session = session()?;
        let connect = Handle(WinHttpConnect(
            session.0,
            &HSTRING::from(host),
            INTERNET_DEFAULT_HTTPS_PORT,
            0,
        ));
        if connect.0.is_null() {
            return Err(offline());
        }
        let request = Handle(WinHttpOpenRequest(
            connect.0,
            w!("GET"),
            &HSTRING::from(path),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ));
        if request.0.is_null() {
            return Err(offline());
        }
        WinHttpSendRequest(request.0, None, None, 0, 0, 0).map_err(|_| offline())?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut()).map_err(|_| offline())?;

        let mut status: u32 = 0;
        let mut len = std::mem::size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut c_void),
            &mut len,
            std::ptr::null_mut(),
        )
        .map_err(|_| offline())?;

        let mut body = Vec::new();
        let mut chunk = vec![0u8; 16 * 1024];
        loop {
            let mut read = 0u32;
            WinHttpReadData(
                request.0,
                chunk.as_mut_ptr() as *mut c_void,
                chunk.len() as u32,
                &mut read,
            )
            .map_err(|_| offline())?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..read as usize]);
            if body.len() > 4 * 1024 * 1024 {
                return Err("The dictionary response was unexpectedly large.".into());
            }
        }
        Ok((status, body))
    }
}
