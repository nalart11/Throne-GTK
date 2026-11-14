#ifndef THRONE_ACL_PRESENCE_H
#define THRONE_ACL_PRESENCE_H
#include <stddef.h>

// Only a successful metadata read AND successful presence query can prove
// absence. ENOENT/ENOATTR/ENOTSUP from a failed read are never that proof.
static int throne_acl_presence(int metadata_ok, int query_ok, int present) {
    if (!metadata_ok || !query_ok || (present != 0 && present != 1)) return -1;
    return present;
}

#ifdef __APPLE__
#include <sys/acl.h>
#include <sys/stat.h>
#include <fcntl.h>

// Apple Libc posix1e/acl_file.c returns NULL for both read failures and absent
// ACLs. gen/filesec.c returns ENOENT for an absent FILESEC_ACL property; accepting
// that errno directly would also accept a missing path. Read and query the
// metadata separately, exactly as sys/statx_np.c and filesec_query_property
// specify. A zero-entry (but present) ACL still goes through acl_valid/iteration.
static int throne_acl_read(int fd, const char *path, acl_t *result) {
    *result = NULL;
    filesec_t security = filesec_init();
    if (!security) return -1;
    struct stat st;
    int metadata_ok = (path ? statx_np(path, &st, security) : fstatx_np(fd, &st, security)) == 0;
    int present = -1;
    int query_ok = metadata_ok && filesec_query_property(security, FILESEC_ACL, &present) == 0;
    // Apple's implementation returns an internal validity bit, not necessarily
    // the integer 1, for a present property.
    int state = throne_acl_presence(metadata_ok, query_ok, present != 0);
    if (state == 1 && (filesec_get_property(security, FILESEC_ACL, result) != 0 || !*result)) state = -1;
    filesec_free(security);
    return state;
}
#endif
#endif
