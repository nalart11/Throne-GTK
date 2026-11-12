/* Package-owned pre-extraction guard. No Security framework or installed code. */
#include <sys/acl.h>
#include <sys/stat.h>
#include <fts.h>
#include <errno.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "acl_policy.h"

static int safe_acl(const char *path) {
    acl_t acl = acl_get_file(path, ACL_TYPE_EXTENDED);
    if (!acl) return -1;
    if (acl_valid(acl)) { acl_free(acl); return -1; }
    acl_entry_t entry;
    int selector = ACL_FIRST_ENTRY, result = 0;
    for (;;) {
        errno = 0;
        if (acl_get_entry(acl, selector, &entry)) {
            if (errno != EINVAL) result = -1;
            break;
        }
        selector = ACL_NEXT_ENTRY;
        acl_tag_t tag;
        if (acl_get_tag_type(entry, &tag)) { result = -1; break; }
        if (tag == ACL_EXTENDED_DENY) continue;
        if (tag != ACL_EXTENDED_ALLOW) { result = -1; break; }
        acl_permset_t perms;
        if (acl_get_permset(entry, &perms)) { result = -1; break; }
        /* Includes inherited and inherit-only entries: no flag bypass. */
        const acl_perm_t mutation[] = { ACL_WRITE_DATA, ACL_APPEND_DATA,
            ACL_DELETE, ACL_DELETE_CHILD, ACL_WRITE_ATTRIBUTES,
            ACL_WRITE_EXTATTRIBUTES, ACL_WRITE_SECURITY, ACL_CHANGE_OWNER };
        for (size_t i = 0; i < sizeof(mutation)/sizeof(mutation[0]); ++i) {
            int granted = acl_get_perm_np(perms, mutation[i]);
            if (throne_acl_decision(0, 1, granted >= 0, granted != 0)) {
                result = -1;
                break;
            }
        }
        if (result) break;
    }
    acl_free(acl);
    return result;
}

static int safe_node(const char *path, const char *tree) {
    struct stat st;
    if (lstat(path, &st) || st.st_uid != 0) return -1;
    if (S_ISLNK(st.st_mode)) {
        char resolved[PATH_MAX];
        if (!tree || !realpath(path, resolved) ||
            strncmp(resolved, tree, strlen(tree)) || resolved[strlen(tree)] != '/') return -1;
        /* Symlink modes do not grant authority; check its contained target. */
        if (stat(path, &st) || st.st_uid != 0 || (st.st_mode & 0022)) return -1;
    } else if ((!S_ISDIR(st.st_mode) && !S_ISREG(st.st_mode)) || (st.st_mode & 0022)) {
        return -1;
    }
    return safe_acl(path);
}

int main(int argc, char **argv) {
    if (argc < 3) return 2;
    if (!strcmp(argv[1], "--nodes")) {
        for (int i = 2; i < argc; ++i) {
            if (safe_node(argv[i], NULL)) {
                fprintf(stderr, "Unsafe installer node (owner/mode/ACL): %s\n", argv[i]);
                return 1;
            }
        }
        return 0;
    }
    if (!strcmp(argv[1], "--ancestors")) {
        for (int i = 2; i < argc; ++i) {
            struct stat st;
            if (lstat(argv[i], &st) || !S_ISDIR(st.st_mode) || safe_node(argv[i], NULL)) {
                fprintf(stderr, "Unsafe installer ancestor (owner/mode/ACL): %s\n", argv[i]);
                return 1;
            }
        }
        return 0;
    }
    if (argc != 3 || strcmp(argv[1], "--tree")) return 2;
    struct stat st;
    if (lstat(argv[2], &st) || !S_ISDIR(st.st_mode)) return 1;
    char *paths[] = { argv[2], NULL };
    FTS *fts = fts_open(paths, FTS_PHYSICAL | FTS_NOCHDIR, NULL);
    if (!fts) return 1;
    int result = 0;
    FTSENT *entry;
    errno = 0;
    while ((entry = fts_read(fts))) {
        if (entry->fts_info == FTS_DP) continue;
        if (entry->fts_info == FTS_ERR || entry->fts_info == FTS_DNR ||
            entry->fts_info == FTS_NS || safe_node(entry->fts_path, argv[2])) {
            fprintf(stderr, "Unsafe existing install node (owner/mode/ACL): %s\n", entry->fts_path);
            result = 1;
            break;
        }
        errno = 0;
    }
    if (!entry && errno) result = 1;
    if (fts_close(fts)) result = 1;
    return result;
}
