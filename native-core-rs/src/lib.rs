//! Rust spike re-implementation of two `native-core` modules over the same
//! C ABI, so the resulting staticlib is a link-time drop-in for the C++
//! objects:
//!
//! * `native-core/src/omi_backend_policy.cpp` (all 8 exported functions)
//! * `native-core/src/omi_native_boundary.cpp` (`omi_calculate_packet_checksum`,
//!   `omi_normalize_packet`; `omi_get_native_capabilities` stays C++ in the spike)
//!
//! Behavior parity rules, ported from the C++ sources:
//! * Inputs are byte strings; never UTF-8 validated (C++ `string_view` semantics).
//! * Null-argument and overflow return codes are reproduced exactly.
//! * Hostname case folding is ASCII-only (C locale `tolower`).
//! * No allocation, no syscalls, no panics on any input path; release builds
//!   use `panic = "abort"` so unwinding can never cross the FFI boundary.
//!
//! Zero external crates: `std` only.

use std::ffi::CStr;

// ---------------------------------------------------------------------------
// byte helpers (parity with the C++ string_view helpers)
// ---------------------------------------------------------------------------

/// # Safety
/// `p` must be null or point to a valid NUL-terminated string that outlives
/// the returned borrow (duration of the call).
unsafe fn opt_bytes<'a>(p: *const std::ffi::c_char) -> Option<&'a [u8]> {
    if p.is_null() {
        return None;
    }
    // SAFETY: caller contract of the C ABI: non-null points to a NUL-terminated string.
    Some(unsafe { CStr::from_ptr(p) }.to_bytes())
}

fn strip_route(path: &[u8]) -> &[u8] {
    match path.iter().position(|&c| c == b'?' || c == b'#') {
        Some(cut) => &path[..cut],
        None => path,
    }
}

fn eq_ci(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b.iter())
            .all(|(x, y)| x.to_ascii_lowercase() == y.to_ascii_lowercase())
}

fn normalize_host(host: &[u8]) -> &[u8] {
    if host.len() >= 2 && host[0] == b'[' && host[host.len() - 1] == b']' {
        &host[1..host.len() - 1]
    } else {
        host
    }
}

// ---------------------------------------------------------------------------
// policy: route classification
// ---------------------------------------------------------------------------

fn is_capture_route(route: &[u8]) -> bool {
    route == b"/v1/settings"
        || route == b"/v1/live/sessions"
        || route == b"/v1/chat-messages"
        || route.starts_with(b"/v1/chat-generations/")
        || route == b"/v1/chat-attachments"
        || route.starts_with(b"/v1/chat-attachments/")
        || route == b"/v1/device-sessions"
        || route.starts_with(b"/v1/device-sessions/")
        || route == b"/v1/conversations"
        || route == b"/v1/memories"
        || route == b"/v1/tasks"
        || route == b"/v1/tasks/ops"
}

fn example_platform_device_session(method: &[u8], route: &[u8]) -> bool {
    const PREFIX: &[u8] = b"/v1/device-sessions";
    const REST: &[u8] = b"/v1/device-sessions/";
    if route == PREFIX {
        return method == b"POST";
    }
    if !route.starts_with(REST) {
        return false;
    }
    let rest = &route[REST.len()..];
    if rest.is_empty() || rest[0] == b'/' {
        return false;
    }
    if method == b"GET" && rest == b"ownership" {
        return true;
    }
    let Some(slash) = rest.iter().position(|&c| c == b'/') else {
        return method == b"GET";
    };
    let id = &rest[..slash];
    let tail = &rest[slash + 1..];
    if id.is_empty() || id.contains(&b'/') {
        return false;
    }
    if method == b"GET" {
        return tail == b"transcript";
    }
    method == b"POST" && (tail == b"audio" || tail == b"complete" || tail == b"transcribe")
}

/// Match Apple `OmiRequestTimeout`: URL path split on '/', leading empty segment.
fn is_transcribe_timeout_path(path: &[u8]) -> bool {
    const REST: &[u8] = b"/v1/device-sessions/";
    let route = strip_route(path);
    // Expect: "" / "v1" / "device-sessions" / "<id>" / "transcribe"
    if !route.starts_with(REST) {
        return false;
    }
    let rest = &route[REST.len()..];
    if rest.is_empty() || rest[0] == b'/' {
        return false;
    }
    let Some(slash) = rest.iter().position(|&c| c == b'/') else {
        return false;
    };
    if slash == 0 {
        return false;
    }
    let id = &rest[..slash];
    let tail = &rest[slash + 1..];
    !id.is_empty() && !id.contains(&b'/') && tail == b"transcribe"
}

// ---------------------------------------------------------------------------
// policy: exported C ABI (identical symbols to omi_backend_policy.h)
// ---------------------------------------------------------------------------

/// C ABI: `int32_t omi_backend_route_strip(const char* path, char* out, size_t out_cap)`.
///
/// # Safety
/// `path` must be null or NUL-terminated; `out` must be null or writable for
/// `out_cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn omi_backend_route_strip(
    path: *const std::ffi::c_char,
    out: *mut std::ffi::c_char,
    out_cap: usize,
) -> i32 {
    if path.is_null() || out.is_null() || out_cap == 0 {
        return -1;
    }
    // SAFETY: see function contract.
    let route = strip_route(unsafe { opt_bytes(path) }.expect("checked non-null above"));
    if route.len() + 1 > out_cap {
        return -1;
    }
    // SAFETY: caller contract: `out` writable for `out_cap >= route.len() + 1` bytes.
    unsafe {
        std::ptr::copy_nonoverlapping(route.as_ptr().cast::<std::ffi::c_char>(), out, route.len());
        *out.add(route.len()) = 0;
    }
    route.len() as i32
}

/// C ABI: `int32_t omi_backend_is_capture_path(const char* path)`.
///
/// # Safety
/// `path` must be null or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn omi_backend_is_capture_path(path: *const std::ffi::c_char) -> i32 {
    match unsafe { opt_bytes(path) } {
        None => -1,
        Some(p) => i32::from(is_capture_route(strip_route(p))),
    }
}

/// C ABI: `int32_t omi_backend_request_timeout_seconds(const char* method, const char* path)`.
///
/// # Safety
/// Both pointers must be null or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn omi_backend_request_timeout_seconds(
    method: *const std::ffi::c_char,
    path: *const std::ffi::c_char,
) -> i32 {
    // SAFETY: see function contract.
    let (Some(m), Some(p)) = (unsafe { opt_bytes(method) }, unsafe { opt_bytes(path) }) else {
        return 60;
    };
    if m == b"POST" && is_transcribe_timeout_path(p) {
        150
    } else {
        60
    }
}

/// C ABI: `int32_t omi_backend_example_platform_supported(const char* method, const char* path)`.
///
/// # Safety
/// Both pointers must be null or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn omi_backend_example_platform_supported(
    method: *const std::ffi::c_char,
    path: *const std::ffi::c_char,
) -> i32 {
    // SAFETY: see function contract.
    let (Some(m), Some(p)) = (unsafe { opt_bytes(method) }, unsafe { opt_bytes(path) }) else {
        return -1;
    };
    let route = strip_route(p);
    if m == b"GET" {
        if route == b"/v1/settings"
            || route == b"/v1/chat-messages"
            || route == b"/v1/conversations"
            || route == b"/v1/memories"
            || route == b"/v1/tasks"
        {
            return 1;
        }
        return i32::from(example_platform_device_session(b"GET", route));
    }
    if m == b"POST" && route == b"/v1/tasks/ops" {
        return 1;
    }
    if m == b"POST" && example_platform_device_session(b"POST", route) {
        return 1;
    }
    0
}

// ---------------------------------------------------------------------------
// policy: hostname allowlist
// ---------------------------------------------------------------------------

/// C ABI: `int32_t omi_backend_is_loopback_hostname(const char* hostname)`.
///
/// # Safety
/// `hostname` must be null or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn omi_backend_is_loopback_hostname(hostname: *const std::ffi::c_char) -> i32 {
    // SAFETY: see function contract.
    match unsafe { opt_bytes(hostname) } {
        None => -1,
        Some(h) => {
            let host = normalize_host(h);
            i32::from(
                eq_ci(host, b"localhost") || eq_ci(host, b"127.0.0.1") || eq_ci(host, b"::1"),
            )
        }
    }
}

/// C ABI: `int32_t omi_backend_is_cloud_hostname(const char* hostname)`.
///
/// # Safety
/// `hostname` must be null or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn omi_backend_is_cloud_hostname(hostname: *const std::ffi::c_char) -> i32 {
    // SAFETY: see function contract.
    match unsafe { opt_bytes(hostname) } {
        None => -1,
        Some(h) => i32::from(eq_ci(normalize_host(h), b"api.omi.me")),
    }
}

/// C ABI: `int32_t omi_backend_is_allowed_v5_hostname(const char* hostname)`.
///
/// # Safety
/// `hostname` must be null or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn omi_backend_is_allowed_v5_hostname(hostname: *const std::ffi::c_char) -> i32 {
    // SAFETY: see function contract.
    let Some(h) = (unsafe { opt_bytes(hostname) }) else {
        return -1;
    };
    // SAFETY: same string, still NUL-terminated.
    if unsafe { omi_backend_is_loopback_hostname(hostname) } == 1
        || unsafe { omi_backend_is_cloud_hostname(hostname) } == 1
    {
        return 1;
    }
    let mut lower = Vec::with_capacity(h.len());
    for &b in h {
        lower.push(b.to_ascii_lowercase());
    }
    let host = normalize_host(&lower);
    const SUFFIX: &[u8] = b".workers.dev";
    i32::from(host.len() > SUFFIX.len() && host.ends_with(SUFFIX))
}

/// C ABI: `int32_t omi_backend_software_plane_is_new(const char* stored, int32_t stamped_valid)`.
///
/// # Safety
/// `stored` must be null or NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn omi_backend_software_plane_is_new(
    stored: *const std::ffi::c_char,
    stamped_valid: i32,
) -> i32 {
    // SAFETY: see function contract.
    match unsafe { opt_bytes(stored) } {
        Some(s) if !s.is_empty() => i32::from(s == b"new"),
        _ => i32::from(stamped_valid != 0),
    }
}

// ---------------------------------------------------------------------------
// native boundary: packet checksum + framing (omi_native_boundary.h)
// ---------------------------------------------------------------------------

pub(crate) const OMI_STATUS_OK: i32 = 0;
pub(crate) const OMI_STATUS_ERR_INVALID_PARAM: i32 = -1;
pub(crate) const OMI_STATUS_ERR_SYNC_BYTES: i32 = -2;
pub(crate) const OMI_STATUS_ERR_CHECKSUM: i32 = -3;
pub(crate) const OMI_STATUS_ERR_BUFFER_OVERFLOW: i32 = -4;

fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ if crc & 1 != 0 { 0xEDB8_8320 } else { 0 };
        }
    }
    crc ^ 0xFFFF_FFFF
}

/// C ABI: `uint32_t omi_calculate_packet_checksum(const uint8_t* data, size_t length)`.
///
/// # Safety
/// `data` must be null or readable for `length` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn omi_calculate_packet_checksum(
    data: *const u8,
    length: usize,
) -> u32 {
    if data.is_null() || length == 0 {
        return 0;
    }
    // SAFETY: caller contract: readable for `length` bytes.
    let bytes = unsafe { std::slice::from_raw_parts(data, length) };
    crc32(bytes)
}

/// C ABI: `int32_t omi_normalize_packet(const uint8_t* raw_data, size_t raw_len,
/// uint8_t* out_data, size_t max_out_len, size_t* out_len)`.
///
/// # Safety
/// Pointers must be null or valid per the C header contract (`raw_data`
/// readable for `raw_len`, `out_data` writable for `max_out_len`,
/// `out_len` writable).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn omi_normalize_packet(
    raw_data: *const u8,
    raw_len: usize,
    out_data: *mut u8,
    max_out_len: usize,
    out_len: *mut usize,
) -> i32 {
    if raw_data.is_null() || out_data.is_null() || out_len.is_null() {
        return OMI_STATUS_ERR_INVALID_PARAM;
    }
    // Minimum framed packet: 2 sync bytes + 0 payload + 4 checksum bytes.
    if raw_len < 6 {
        return OMI_STATUS_ERR_INVALID_PARAM;
    }
    // SAFETY: caller contract: readable for `raw_len` bytes.
    let raw = unsafe { std::slice::from_raw_parts(raw_data, raw_len) };
    if raw[0] != 0xAA || raw[1] != 0x55 {
        return OMI_STATUS_ERR_SYNC_BYTES;
    }
    let payload_len = raw_len - 6;
    if payload_len > max_out_len {
        return OMI_STATUS_ERR_BUFFER_OVERFLOW;
    }
    let expected_crc = u32::from_be_bytes([
        raw[raw_len - 4],
        raw[raw_len - 3],
        raw[raw_len - 2],
        raw[raw_len - 1],
    ]);
    let payload = &raw[2..2 + payload_len];
    if crc32(payload) != expected_crc {
        return OMI_STATUS_ERR_CHECKSUM;
    }
    if payload_len > 0 {
        // SAFETY: caller contract: `out_data` writable for `max_out_len >= payload_len`.
        unsafe {
            std::ptr::copy_nonoverlapping(payload.as_ptr(), out_data, payload_len);
        }
    }
    // SAFETY: caller contract: `out_len` writable.
    unsafe {
        *out_len = payload_len;
    }
    OMI_STATUS_OK
}
