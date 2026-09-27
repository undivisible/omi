//! Ported from `native-core/tests/test_omi_backend_policy.cpp` plus
//! `omi_native_boundary` checksum vectors, so the Rust implementation is
//! held to the same acceptance vectors as the C++ host tests.

use std::ffi::{CStr, CString};

use omi_native_core_rs::*;

fn s(v: &str) -> CString {
    CString::new(v).expect("no interior NUL")
}

unsafe fn strip(path: &str) -> (i32, String) {
    let mut out = vec![0u8; 256];
    let ret = omi_backend_route_strip(s(path).as_ptr(), out.as_mut_ptr().cast(), out.len());
    let out_str = CStr::from_bytes_until_nul(&out).expect("nul present").to_string_lossy().into_owned();
    (ret, out_str)
}

#[test]
fn route_strip() {
    unsafe {
        assert_eq!(strip("/v1/tasks?limit=2"), (9, "/v1/tasks".into()));
        assert_eq!(strip("/v1/conversations#frag"), (17, "/v1/conversations".into()));
        assert_eq!(
            omi_backend_route_strip(std::ptr::null(), [0u8; 8].as_mut_ptr().cast(), 8),
            -1
        );
        assert_eq!(omi_backend_route_strip(s("/v1/x").as_ptr(), std::ptr::null_mut(), 8), -1);
        assert_eq!(omi_backend_route_strip(s("/v1/x").as_ptr(), [0u8; 8].as_mut_ptr().cast(), 0), -1);
        // Overflow: cap smaller than route + NUL.
        assert_eq!(omi_backend_route_strip(s("/v1/tasks").as_ptr(), [0u8; 4].as_mut_ptr().cast(), 4), -1);
    }
}

#[test]
fn capture_paths() {
    let cases: &[(&str, i32)] = &[
        ("/v1/live/sessions", 1),
        ("/v1/live/sessions?retry=1", 1),
        ("/v1/live/sessions-extra", 0),
        ("/v1/chat-messages", 1),
        ("/v1/chat-messages?limit=50", 1),
        ("/v1/chat-generations/id/events", 1),
        ("/v1/chat-attachments", 1),
        ("/v1/chat-attachments/id/complete", 1),
        ("/v1/device-sessions", 1),
        ("/v1/device-sessions/id/audio", 1),
        ("/v1/device-sessions-extra", 0),
        ("/v1/settings", 1),
        ("/v1/conversations", 1),
        ("/v1/memories", 1),
        ("/v1/tasks", 1),
        ("/v1/tasks/ops", 1),
        ("/v1/tasks/ops?op=complete", 1),
        ("/v1/tasks/one", 0),
        ("/v1/users/me", 0),
        ("/v1/conversations#keep", 1),
    ];
    unsafe {
        for (path, want) in cases {
            assert_eq!(omi_backend_is_capture_path(s(path).as_ptr()), *want, "path {path}");
        }
        assert_eq!(omi_backend_is_capture_path(std::ptr::null()), -1);
    }
}

#[test]
fn timeouts() {
    unsafe {
        assert_eq!(omi_backend_request_timeout_seconds(s("POST").as_ptr(), s("/v1/device-sessions/id/transcribe").as_ptr()), 150);
        assert_eq!(omi_backend_request_timeout_seconds(s("POST").as_ptr(), s("/v1/device-sessions/id/transcribe?retry=1").as_ptr()), 150);
        assert_eq!(omi_backend_request_timeout_seconds(s("GET").as_ptr(), s("/v1/device-sessions/id/transcribe").as_ptr()), 60);
        assert_eq!(omi_backend_request_timeout_seconds(s("POST").as_ptr(), s("/v1/device-sessions/id/complete").as_ptr()), 60);
        assert_eq!(omi_backend_request_timeout_seconds(s("POST").as_ptr(), s("/v1/device-sessions//transcribe").as_ptr()), 60);
        assert_eq!(omi_backend_request_timeout_seconds(s("POST").as_ptr(), s("/v1/live/sessions").as_ptr()), 60);
        assert_eq!(omi_backend_request_timeout_seconds(s("POST").as_ptr(), s("/v1/device-sessions/11111111-2222-3333-4444-555555555555/transcribe").as_ptr()), 150);
        // Nulls default to 60 like the C++.
        assert_eq!(omi_backend_request_timeout_seconds(std::ptr::null(), s("/v1/x").as_ptr()), 60);
        assert_eq!(omi_backend_request_timeout_seconds(s("POST").as_ptr(), std::ptr::null()), 60);
    }
}

#[test]
fn example_platform() {
    let cases: &[(&str, &str, i32)] = &[
        ("GET", "/v1/tasks?limit=2", 1),
        ("GET", "/v1/conversations", 1),
        ("GET", "/v1/memories", 1),
        ("GET", "/v1/settings", 1),
        ("GET", "/v1/chat-messages?limit=50", 1),
        ("GET", "/v1/device-sessions/ownership", 1),
        ("GET", "/v1/device-sessions/11111111-2222-3333-4444-555555555555", 1),
        ("GET", "/v1/device-sessions/11111111-2222-3333-4444-555555555555/transcript", 1),
        ("POST", "/v1/device-sessions", 1),
        ("POST", "/v1/device-sessions/11111111-2222-3333-4444-555555555555/audio", 1),
        ("POST", "/v1/tasks/ops", 1),
        ("DELETE", "/v1/tasks/ops", 0),
        ("POST", "/v1/tasks", 0),
        ("POST", "/v1/chat-messages", 0),
        ("GET", "/v1/chat-generations/one/events", 0),
        ("POST", "/v1/chat-attachments", 0),
    ];
    unsafe {
        for (method, path, want) in cases {
            assert_eq!(
                omi_backend_example_platform_supported(s(method).as_ptr(), s(path).as_ptr()),
                *want,
                "{method} {path}"
            );
        }
        assert_eq!(
            omi_backend_example_platform_supported(std::ptr::null(), s("/v1/x").as_ptr()),
            -1
        );
    }
}

#[test]
fn hosts() {
    unsafe {
        assert_eq!(omi_backend_is_loopback_hostname(s("localhost").as_ptr()), 1);
        assert_eq!(omi_backend_is_loopback_hostname(s("127.0.0.1").as_ptr()), 1);
        assert_eq!(omi_backend_is_loopback_hostname(s("[::1]").as_ptr()), 1);
        assert_eq!(omi_backend_is_cloud_hostname(s("api.omi.me").as_ptr()), 1);
        assert_eq!(omi_backend_is_cloud_hostname(s("API.OMI.ME").as_ptr()), 1);
        assert_eq!(omi_backend_is_allowed_v5_hostname(s("synthetic.workers.dev").as_ptr()), 1);
        assert_eq!(omi_backend_is_allowed_v5_hostname(s("workers.dev").as_ptr()), 0);
        assert_eq!(omi_backend_is_allowed_v5_hostname(s("omi-platform-dev-abcdef012345-uc.a.run.app").as_ptr()), 0);
        assert_eq!(omi_backend_is_allowed_v5_hostname(s("omi-platform-dev-attacker.a.run.app").as_ptr()), 0);
        assert_eq!(omi_backend_is_allowed_v5_hostname(s("untrusted.invalid").as_ptr()), 0);
        assert_eq!(omi_backend_is_loopback_hostname(std::ptr::null()), -1);
        assert_eq!(omi_backend_is_cloud_hostname(std::ptr::null()), -1);
        assert_eq!(omi_backend_is_allowed_v5_hostname(std::ptr::null()), -1);
    }
}

#[test]
fn software_plane() {
    unsafe {
        assert_eq!(omi_backend_software_plane_is_new(std::ptr::null(), 1), 1);
        assert_eq!(omi_backend_software_plane_is_new(std::ptr::null(), 0), 0);
        assert_eq!(omi_backend_software_plane_is_new(s("").as_ptr(), 1), 1);
        assert_eq!(omi_backend_software_plane_is_new(s("new").as_ptr(), 0), 1);
        assert_eq!(omi_backend_software_plane_is_new(s("old").as_ptr(), 1), 0);
        assert_eq!(omi_backend_software_plane_is_new(s("unexpected").as_ptr(), 1), 0);
    }
}

#[test]
fn packet_checksum() {
    // Known CRC-32 (IEEE, reflected) vectors.
    let payload = b"123456789";
    unsafe {
        assert_eq!(omi_calculate_packet_checksum(payload.as_ptr(), payload.len()), 0xCBF4_3926);
        assert_eq!(omi_calculate_packet_checksum(std::ptr::null(), 16), 0);
        assert_eq!(omi_calculate_packet_checksum(payload.as_ptr(), 0), 0);
    }
}

#[test]
fn normalize_packet() {
    let payload = b"hello";
    let crc = 0xCBF4_3926u32; // unused placeholder; compute real one below
    let _ = crc;
    let mut framed = vec![0xAA, 0x55];
    framed.extend_from_slice(payload);
    let c = unsafe { omi_calculate_packet_checksum(payload.as_ptr(), payload.len()) };
    framed.extend_from_slice(&c.to_be_bytes());
    let mut out = [0u8; 64];
    let mut out_len = 0usize;
    unsafe {
        let status = omi_normalize_packet(framed.as_ptr(), framed.len(), out.as_mut_ptr(), out.len(), &mut out_len);
        assert_eq!(status, 0);
        assert_eq!(out_len, payload.len());
        assert_eq!(&out[..out_len], payload);

        // Bad sync bytes.
        let mut bad = framed.clone();
        bad[0] = 0x00;
        assert_eq!(
            omi_normalize_packet(bad.as_ptr(), bad.len(), out.as_mut_ptr(), out.len(), &mut out_len),
            -2
        );
        // Bad checksum.
        let mut bad = framed.clone();
        let last = bad.len() - 1;
        bad[last] ^= 0xFF;
        assert_eq!(
            omi_normalize_packet(bad.as_ptr(), bad.len(), out.as_mut_ptr(), out.len(), &mut out_len),
            -3
        );
        // Truncated frame.
        assert_eq!(
            omi_normalize_packet(framed.as_ptr(), 5, out.as_mut_ptr(), out.len(), &mut out_len),
            -1
        );
        // Overflow of the output buffer.
        assert_eq!(
            omi_normalize_packet(framed.as_ptr(), framed.len(), out.as_mut_ptr(), 2, &mut out_len),
            -4
        );
        // Null args.
        assert_eq!(
            omi_normalize_packet(std::ptr::null(), framed.len(), out.as_mut_ptr(), out.len(), &mut out_len),
            -1
        );
    }
}
