// Differential FFI driver: executes identical probes against whichever
// implementation is linked in (native-core C++ objects or the Rust staticlib)
// and prints one deterministic line per probe, so `diff` proves behavioral
// parity of the shared C ABI.
//
// Build A (C++): link driver.cpp with native-core's omi_backend_policy.cpp
//                and omi_native_boundary.cpp.
// Build B (Rust): link driver.cpp with libomi_native_core_rs.a.
// Both builds compile against the drop-in header omi_native_core_rs.h.

#include <cstdint>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

#include "omi_native_core_rs.h"

namespace {

void hex_decode(const std::string& hex, std::vector<uint8_t>& out) {
  out.clear();
  out.reserve(hex.size() / 2);
  auto nib = [](char c) -> int {
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    return -1;
  };
  for (size_t i = 0; i + 1 < hex.size(); i += 2) {
    const int hi = nib(hex[i]);
    const int lo = nib(hex[i + 1]);
    if (hi < 0 || lo < 0) return;
    out.push_back(static_cast<uint8_t>((hi << 4) | lo));
  }
}

void print_hex(const uint8_t* p, size_t n) {
  for (size_t i = 0; i < n; ++i) {
    std::printf("%02x", p[i]);
  }
}

void null_battery() {
  std::printf("null strip path ret=%d\n",
              omi_backend_route_strip(nullptr, nullptr, 8));
  std::printf("null capture ret=%d\n", omi_backend_is_capture_path(nullptr));
  std::printf("null timeout ret=%d\n",
              omi_backend_request_timeout_seconds(nullptr, nullptr));
  std::printf("null example ret=%d\n",
              omi_backend_example_platform_supported(nullptr, nullptr));
  std::printf("null loopback ret=%d\n",
              omi_backend_is_loopback_hostname(nullptr));
  std::printf("null cloud ret=%d\n", omi_backend_is_cloud_hostname(nullptr));
  std::printf("null allowed ret=%d\n",
              omi_backend_is_allowed_v5_hostname(nullptr));
  std::printf("null plane ret=%d\n",
              omi_backend_software_plane_is_new(nullptr, 1));
  std::printf("null crc ret=%u\n",
              omi_calculate_packet_checksum(nullptr, 8));
  uint8_t out8[8] = {};
  size_t out_len = 12345;
  std::printf("null frame ret=%d\n",
              omi_normalize_packet(nullptr, 8, out8, sizeof(out8), &out_len));
  std::printf("null frame out_len=%zu\n", out_len);
}

}  // namespace

int main(int argc, char** argv) {
  if (argc != 2) {
    std::fprintf(stderr, "usage: driver <vectors-file>\n");
    return 2;
  }
  FILE* f = std::fopen(argv[1], "r");
  if (f == nullptr) {
    std::perror("open");
    return 2;
  }

  null_battery();

  char line[4096];
  std::vector<uint8_t> bytes;
  while (std::fgets(line, sizeof(line), f) != nullptr) {
    size_t n = std::strlen(line);
    while (n > 0 && (line[n - 1] == '\n' || line[n - 1] == '\r')) {
      line[--n] = '\0';
    }
    if (n == 0 || line[0] == '#') {
      continue;
    }
    char cmd[16] = {};
    char a1[2048] = {};
    char a2[2048] = {};
    const int got = std::sscanf(line, "%15s %2047s %2047s", cmd, a1, a2);
    if (got <= 0) {
      continue;
    }
    if (std::strcmp(cmd, "strip") == 0) {
      char out[256];
      std::memset(out, '#', sizeof(out));
      const int32_t ret = omi_backend_route_strip(a1, out, sizeof(out));
      if (ret >= 0) {
        std::printf("strip %s => ret=%d out=%s\n", a1, ret, out);
      } else {
        std::printf("strip %s => ret=%d\n", a1, ret);
      }
    } else if (std::strcmp(cmd, "capture") == 0) {
      std::printf("capture %s => ret=%d\n", a1,
                  omi_backend_is_capture_path(a1));
    } else if (std::strcmp(cmd, "timeout") == 0) {
      std::printf("timeout %s %s => ret=%d\n", a1, a2,
                  omi_backend_request_timeout_seconds(a1, a2));
    } else if (std::strcmp(cmd, "example") == 0) {
      std::printf("example %s %s => ret=%d\n", a1, a2,
                  omi_backend_example_platform_supported(a1, a2));
    } else if (std::strcmp(cmd, "loopback") == 0) {
      std::printf("loopback %s => ret=%d\n", a1,
                  omi_backend_is_loopback_hostname(a1));
    } else if (std::strcmp(cmd, "cloud") == 0) {
      std::printf("cloud %s => ret=%d\n", a1,
                  omi_backend_is_cloud_hostname(a1));
    } else if (std::strcmp(cmd, "allowed") == 0) {
      std::printf("allowed %s => ret=%d\n", a1,
                  omi_backend_is_allowed_v5_hostname(a1));
    } else if (std::strcmp(cmd, "plane") == 0) {
      const char* stored = std::strcmp(a1, "-") == 0 ? nullptr : a1;
      const int stamped = std::atoi(a2);
      std::printf("plane %s %d => ret=%d\n", a1, stamped,
                  omi_backend_software_plane_is_new(stored, stamped));
    } else if (std::strcmp(cmd, "crc") == 0) {
      hex_decode(a1, bytes);
      std::printf("crc %s => ret=%u\n", a1,
                  omi_calculate_packet_checksum(
                      bytes.empty() ? nullptr : bytes.data(), bytes.size()));
    } else if (std::strcmp(cmd, "frame") == 0) {
      hex_decode(a1, bytes);
      const size_t max_out = static_cast<size_t>(std::atoi(a2));
      std::vector<uint8_t> out(max_out + 16, 0);
      size_t out_len = 0;
      const int32_t status = omi_normalize_packet(
          bytes.empty() ? nullptr : bytes.data(), bytes.size(),
          max_out == 0 ? nullptr : out.data(), max_out, &out_len);
      std::printf("frame %s max=%zu => status=%d len=%zu", a1, max_out,
                  status, out_len);
      if (status == OMI_STATUS_OK && out_len > 0) {
        std::printf(" out=");
        print_hex(out.data(), out_len);
      }
      std::printf("\n");
    } else {
      std::printf("unknown %s\n", cmd);
    }
  }
  std::fclose(f);
  return 0;
}
