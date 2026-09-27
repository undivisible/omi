// ObjC++ smoke test for the Rust staticlib: links libomi_native_core_rs.a,
// exercises the exported C ABI from Objective-C++ (including an NSString
// round-trip), and exits non-zero on any failure.
//
// Build (macOS):
//   clang++ -x objective-c++ -std=c++20 -O2 -I ../cinclude \
//     omi_policy_smoke.mm ../build-apple/libomi_native_core_rs_macos.a \
//     -framework Foundation -o omi_policy_smoke

#import <Foundation/Foundation.h>

#include <cstring>
#include <cstdio>

#include "omi_native_core_rs.h"

namespace {

int failures = 0;

void expect(bool cond, const char* msg) {
  if (!cond) {
    std::fprintf(stderr, "FAIL: %s\n", msg);
    ++failures;
  }
}

}  // namespace

int main() {
  @autoreleasepool {
    NSString* path = @"/v1/device-sessions/abc-123/transcribe?chunk=1";
    char route[256];
    std::memset(route, 0, sizeof(route));
    const int32_t n =
        omi_backend_route_strip(path.UTF8String, route, sizeof(route));
    const char* kStripped = "/v1/device-sessions/abc-123/transcribe";
    expect(n == static_cast<int32_t>(std::strlen(kStripped)),
           "route strip length");
    expect(std::strcmp(route, kStripped) == 0, "route strip value");

    expect(omi_backend_request_timeout_seconds("POST", path.UTF8String) == 150,
           "transcribe timeout from NSString-backed bytes");
    expect(omi_backend_request_timeout_seconds("GET", "/v1/settings") == 60,
           "default timeout");
    expect(omi_backend_is_capture_path("/v1/live/sessions") == 1,
           "capture path accepted");
    expect(omi_backend_is_capture_path("/v1/users/me") == 0,
           "non-capture path rejected");
    expect(omi_backend_is_allowed_v5_hostname(
               "omi-platform-example.workers.dev") == 1,
           "workers.dev allowed");
    expect(omi_backend_is_allowed_v5_hostname("evil.example.com") == 0,
           "unrelated host rejected");

    const uint8_t payload[] = {0xde, 0xad, 0xbe, 0xef};
    const uint32_t crc =
        omi_calculate_packet_checksum(payload, sizeof(payload));
    expect(crc != 0, "crc32 nonzero");

    uint8_t out[64];
    size_t out_len = 0;
    uint8_t frame[16];
    frame[0] = 0xAA;
    frame[1] = 0x55;
    std::memcpy(frame + 2, payload, sizeof(payload));
    const uint32_t be = __builtin_bswap32(crc);
    std::memcpy(frame + 6, &be, sizeof(be));
    expect(omi_normalize_packet(frame, sizeof(frame), out, sizeof(out),
                                &out_len) == OMI_STATUS_OK &&
               out_len == sizeof(payload) &&
               std::memcmp(out, payload, sizeof(payload)) == 0,
           "framed packet round-trip");

    NSString* summary = [NSString
        stringWithFormat:@"omi-rs smoke: strip=%d capture=%d crc=%08x", n,
                         omi_backend_is_capture_path("/v1/tasks"), crc];
    NSLog(@"%@", summary);
  }
  if (failures != 0) {
    std::fprintf(stderr, "%d failure(s)\n", failures);
    return 1;
  }
  std::printf("omi_policy_smoke: all checks passed\n");
  return 0;
}
