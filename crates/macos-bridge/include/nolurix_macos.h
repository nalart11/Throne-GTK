#ifndef NOLURIX_MACOS_H
#define NOLURIX_MACOS_H
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
char *nolurix_macos_open(const char *options_json);
char *nolurix_macos_request(uint64_t handle, const char *request_json);
char *nolurix_macos_poll(uint64_t handle, uint32_t timeout_ms);
char *nolurix_macos_close(uint64_t handle);
void nolurix_macos_string_free(char *value);
#ifdef __cplusplus
}
#endif
#endif
