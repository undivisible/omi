#ifndef OMI_NATIVE_CORE_RS_H
#define OMI_NATIVE_CORE_RS_H

// Spike: drop-in C header for the Rust staticlib build of native-core's
// backend-policy and packet-checksum functions. Declarations are identical to
// native-core/include/omi_backend_policy.h and omi_native_boundary.h, so
// consumers keep including the original headers; this file only documents the
// Rust library's exported surface (verified with `nm` in the spike).

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* --- omi_backend_policy.h parity --- */

int32_t omi_backend_route_strip(const char* path, char* out, size_t out_cap);
int32_t omi_backend_is_capture_path(const char* path);
int32_t omi_backend_request_timeout_seconds(const char* method, const char* path);
int32_t omi_backend_example_platform_supported(const char* method, const char* path);
int32_t omi_backend_is_loopback_hostname(const char* hostname);
int32_t omi_backend_is_cloud_hostname(const char* hostname);
int32_t omi_backend_is_allowed_v5_hostname(const char* hostname);
int32_t omi_backend_software_plane_is_new(const char* stored, int32_t stamped_valid);

/* --- omi_native_boundary.h parity (checksum + framing) --- */

#define OMI_STATUS_OK 0
#define OMI_STATUS_ERR_INVALID_PARAM -1
#define OMI_STATUS_ERR_SYNC_BYTES -2
#define OMI_STATUS_ERR_CHECKSUM -3
#define OMI_STATUS_ERR_BUFFER_OVERFLOW -4

uint32_t omi_calculate_packet_checksum(const uint8_t* data, size_t length);
int32_t omi_normalize_packet(const uint8_t* raw_data, size_t raw_len,
                             uint8_t* out_data, size_t max_out_len,
                             size_t* out_len);

#ifdef __cplusplus
}
#endif

#endif /* OMI_NATIVE_CORE_RS_H */
