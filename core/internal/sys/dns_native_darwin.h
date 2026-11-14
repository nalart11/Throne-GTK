#ifndef NOLURIX_DNS_NATIVE_H
#define NOLURIX_DNS_NATIVE_H
#define NOLURIX_DNS_PREFS_BUSY 10004
typedef struct nolurix_dns_tx nolurix_dns_tx;
nolurix_dns_tx *nolurix_dns_begin(const char *, const char *, const char *, int *);
const char *nolurix_dns_service(nolurix_dns_tx *);
const char *nolurix_dns_location(nolurix_dns_tx *);
unsigned char *nolurix_dns_snapshot(nolurix_dns_tx *, const char *, int *, int *);
int nolurix_dns_equal(const unsigned char *, int, const unsigned char *, int);
int nolurix_dns_set(nolurix_dns_tx *, const unsigned char *, int, int *);
int nolurix_dns_commit(nolurix_dns_tx *, int *);
int nolurix_dns_apply(nolurix_dns_tx *, int *);
void nolurix_dns_end(nolurix_dns_tx *);
int nolurix_dns_safe_acl(int);
#endif
