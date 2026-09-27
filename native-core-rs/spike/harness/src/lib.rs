//! eqts exports: the Rust side of the parity harness, callable from
//! TypeScript (Bun) through the generated adapter.
//!
//! `parity_probe` mirrors `spike/differential/driver.cpp` line-for-line so a
//! TS driver can diff the C++ driver's output against the Rust
//! implementation on the same vectors. `null_battery` reproduces the
//! driver's fixed null-argument probes.

eqts::setup!();

use std::ffi::{CStr, CString, c_char};

/// Bumped when the exported surface changes.
#[eqts::export]
pub fn harness_version() -> u32 {
    1
}

fn hex_decode(hex: &str, out: &mut Vec<u8>) {
    out.clear();
    let bytes = hex.as_bytes();
    let nib = |c: u8| -> i32 {
        match c {
            b'0'..=b'9' => (c - b'0') as i32,
            b'a'..=b'f' => (c - b'a') as i32 + 10,
            _ => -1,
        }
    };
    let mut i = 0;
    while i + 1 < bytes.len() {
        let (hi, lo) = (nib(bytes[i]), nib(bytes[i + 1]));
        if hi < 0 || lo < 0 {
            return;
        }
        out.push(((hi << 4) | lo) as u8);
        i += 2;
    }
}

/// C `atoi` approximation (optional sign, leading digits, 0 on no digits).
fn atoi(s: &str) -> i32 {
    let t = s.trim_start();
    let (sign, digits) = match t.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, t),
    };
    let end = digits
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(digits.len());
    if end == 0 {
        return 0;
    }
    sign * digits[..end].parse::<i32>().unwrap_or(0)
}

enum Arg {
    Str(CString),
    Null,
}

impl Arg {
    fn resolve(s: Option<&str>) -> Result<Self, String> {
        match s {
            None => Ok(Self::Null),
            Some(s) => CString::new(s).map(Self::Str).map_err(|_| "interior NUL in argument".to_string()),
        }
    }

    fn ptr(&self) -> *const c_char {
        match self {
            Self::Str(s) => s.as_ptr(),
            Self::Null => std::ptr::null(),
        }
    }
}

/// Runs one probe against the Rust implementation and emits the exact line
/// format `driver.cpp` prints, so outputs diff byte-for-byte.
///
/// `a`/`b` mirror the driver's whitespace-separated argument columns; pass
/// `None` to probe the null-pointer path (the C++ driver covers nulls via
/// its fixed null battery, mirrored by `null_battery`).
#[eqts::export]
pub fn parity_probe(cmd: String, a: Option<String>, b: Option<String>) -> Result<String, String> {
    let a_echo = a.clone().unwrap_or_else(|| "(null)".to_string());
    let b_echo = b.clone().unwrap_or_else(|| "(null)".to_string());
    match cmd.as_str() {
        "strip" => {
            let mut out = vec![b'#'; 256];
            let arg = Arg::resolve(a.as_deref())?;
            let ret = unsafe { omi_native_core_rs::omi_backend_route_strip(arg.ptr(), out.as_mut_ptr().cast(), out.len()) };
            if ret >= 0 {
                let s = CStr::from_bytes_until_nul(&out).expect("buffer prefilled with NULs");
                Ok(format!("strip {a_echo} => ret={ret} out={}", s.to_string_lossy()))
            } else {
                Ok(format!("strip {a_echo} => ret={ret}"))
            }
        }
        "capture" => {
            let arg = Arg::resolve(a.as_deref())?;
            let ret = unsafe { omi_native_core_rs::omi_backend_is_capture_path(arg.ptr()) };
            Ok(format!("capture {a_echo} => ret={ret}"))
        }
        "timeout" => {
            let pa = Arg::resolve(a.as_deref())?;
            let pb = Arg::resolve(b.as_deref())?;
            let ret = unsafe { omi_native_core_rs::omi_backend_request_timeout_seconds(pa.ptr(), pb.ptr()) };
            Ok(format!("timeout {a_echo} {b_echo} => ret={ret}"))
        }
        "example" => {
            let pa = Arg::resolve(a.as_deref())?;
            let pb = Arg::resolve(b.as_deref())?;
            let ret = unsafe { omi_native_core_rs::omi_backend_example_platform_supported(pa.ptr(), pb.ptr()) };
            Ok(format!("example {a_echo} {b_echo} => ret={ret}"))
        }
        "loopback" | "cloud" | "allowed" => {
            let pa = Arg::resolve(a.as_deref())?;
            let ret = match cmd.as_str() {
                "loopback" => unsafe { omi_native_core_rs::omi_backend_is_loopback_hostname(pa.ptr()) },
                "cloud" => unsafe { omi_native_core_rs::omi_backend_is_cloud_hostname(pa.ptr()) },
                _ => unsafe { omi_native_core_rs::omi_backend_is_allowed_v5_hostname(pa.ptr()) },
            };
            Ok(format!("{cmd} {a_echo} => ret={ret}"))
        }
        "plane" => {
            let stored = Arg::resolve(a.as_deref().filter(|v| *v != "-"))?;
            let stamped = atoi(b.as_deref().unwrap_or(""));
            let ret = unsafe { omi_native_core_rs::omi_backend_software_plane_is_new(stored.ptr(), stamped) };
            Ok(format!("plane {a_echo} {stamped} => ret={ret}"))
        }
        "crc" => {
            let mut bytes = Vec::new();
            if let Some(hex) = a.as_deref() {
                if hex != "-" {
                    hex_decode(hex, &mut bytes);
                }
            }
            let ret = unsafe {
                omi_native_core_rs::omi_calculate_packet_checksum(
                    if bytes.is_empty() { std::ptr::null() } else { bytes.as_ptr() },
                    bytes.len(),
                )
            };
            Ok(format!("crc {a_echo} => ret={ret}"))
        }
        "frame" => {
            let mut raw = Vec::new();
            if let Some(hex) = a.as_deref() {
                if hex != "-" {
                    hex_decode(hex, &mut raw);
                }
            }
            let max_out = atoi(b.as_deref().unwrap_or("")) as usize;
            let mut out = vec![0u8; max_out + 16];
            let mut out_len = 0usize;
            let status = unsafe {
                omi_native_core_rs::omi_normalize_packet(
                    if raw.is_empty() { std::ptr::null() } else { raw.as_ptr() },
                    raw.len(),
                    if max_out == 0 { std::ptr::null_mut() } else { out.as_mut_ptr() },
                    max_out,
                    &mut out_len,
                )
            };
            let mut line = format!("frame {} max={max_out} => status={status} len={out_len}", a_echo);
            if status == 0 && out_len > 0 {
                line.push_str(" out=");
                for byte in &out[..out_len] {
                    line.push_str(&format!("{byte:02x}"));
                }
            }
            Ok(line)
        }
        other => Err(format!("unknown command: {other}")),
    }
}

/// The C++ driver's fixed null-argument probes, computed live through the
/// same exported functions (not hardcoded), in driver order.
#[eqts::export]
pub fn null_battery() -> Vec<String> {
    let mut out8 = [0u8; 8];
    let mut out_len = 12_345usize;
    let mut frame_out = [0u8; 8];
    vec![
        format!(
            "null strip path ret={}",
            unsafe { omi_native_core_rs::omi_backend_route_strip(std::ptr::null(), std::ptr::null_mut(), 8) }
        ),
        format!("null capture ret={}", unsafe { omi_native_core_rs::omi_backend_is_capture_path(std::ptr::null()) }),
        format!(
            "null timeout ret={}",
            unsafe { omi_native_core_rs::omi_backend_request_timeout_seconds(std::ptr::null(), std::ptr::null()) }
        ),
        format!(
            "null example ret={}",
            unsafe { omi_native_core_rs::omi_backend_example_platform_supported(std::ptr::null(), std::ptr::null()) }
        ),
        format!("null loopback ret={}", unsafe { omi_native_core_rs::omi_backend_is_loopback_hostname(std::ptr::null()) }),
        format!("null cloud ret={}", unsafe { omi_native_core_rs::omi_backend_is_cloud_hostname(std::ptr::null()) }),
        format!("null allowed ret={}", unsafe { omi_native_core_rs::omi_backend_is_allowed_v5_hostname(std::ptr::null()) }),
        format!(
            "null plane ret={}",
            unsafe { omi_native_core_rs::omi_backend_software_plane_is_new(std::ptr::null(), 1) }
        ),
        format!(
            "null crc ret={}",
            unsafe { omi_native_core_rs::omi_calculate_packet_checksum(std::ptr::null(), 8) }
        ),
        format!(
            "null frame ret={}",
            unsafe {
                omi_native_core_rs::omi_normalize_packet(
                    std::ptr::null(),
                    8,
                    frame_out.as_mut_ptr(),
                    frame_out.len(),
                    &mut out_len,
                )
            }
        ),
        format!("null frame out_len={out_len}"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_vector() {
        assert_eq!(
            parity_probe(
                "timeout".into(),
                Some("POST".into()),
                Some("/v1/device-sessions/id/transcribe".into())
            )
            .unwrap(),
            "timeout POST /v1/device-sessions/id/transcribe => ret=150"
        );
    }

    #[test]
    fn strip_vector_and_null() {
        assert_eq!(
            parity_probe("strip".into(), Some("/v1/tasks?limit=2".into()), None).unwrap(),
            "strip /v1/tasks?limit=2 => ret=9 out=/v1/tasks"
        );
        assert_eq!(
            parity_probe("strip".into(), None, None).unwrap(),
            "strip (null) => ret=-1"
        );
    }

    #[test]
    fn plane_dash_means_null() {
        assert_eq!(
            parity_probe("plane".into(), Some("-".into()), Some("1".into())).unwrap(),
            "plane - 1 => ret=1"
        );
    }

    #[test]
    fn crc_known_vector() {
        assert_eq!(
            parity_probe("crc".into(), Some("313233343536373839".into()), None).unwrap(),
            "crc 313233343536373839 => ret=3421780262"
        );
    }

    #[test]
    fn battery_shapes_like_driver() {
        let lines = null_battery();
        assert_eq!(lines.len(), 11);
        assert_eq!(lines[0], "null strip path ret=-1");
        assert_eq!(lines[9], "null frame ret=-1");
        assert_eq!(lines[10], "null frame out_len=12345");
    }
}
