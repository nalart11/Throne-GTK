#ifndef THRONE_DNS_NATIVE_H
#define THRONE_DNS_NATIVE_H
#define THRONE_DNS_PREFS_BUSY 10004
typedef struct throne_dns_tx throne_dns_tx;
throne_dns_tx *throne_dns_begin(const char *, const char *, const char *, int *);
const char *throne_dns_service(throne_dns_tx *);
const char *throne_dns_location(throne_dns_tx *);
unsigned char *throne_dns_snapshot(throne_dns_tx *, const char *, int *, int *);
int throne_dns_equal(const unsigned char *, int, const unsigned char *, int);
int throne_dns_set(throne_dns_tx *, const unsigned char *, int, int *);
int throne_dns_commit(throne_dns_tx *, int *);
int throne_dns_apply(throne_dns_tx *, int *);
void throne_dns_end(throne_dns_tx *);
int throne_dns_safe_acl(int);
#endif
