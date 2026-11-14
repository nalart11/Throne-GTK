#ifndef THRONE_MACOS_H
#define THRONE_MACOS_H
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
char *throne_macos_open(const char *options_json);
char *throne_macos_request(uint64_t handle, const char *request_json);
char *throne_macos_poll(uint64_t handle, uint32_t timeout_ms);
char *throne_macos_close(uint64_t handle);
void throne_macos_string_free(char *value);
#ifdef __cplusplus
}
#endif
#endif
