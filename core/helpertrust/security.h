#include <sys/acl.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <errno.h>
#include <sys/stat.h>
#include <Security/Security.h>
#include "acl_policy.h"
#include "../internal/sys/acl_presence.h"

// Deny ACEs (including the standard everyone deny delete on /Library) do
// not confer authority. Unknown ACE types or unreadable metadata fail closed.
static int throne_safe_acl(const char *path) {
    acl_t acl = NULL;
    int presence = throne_acl_read(-1, path, &acl);
    if (presence <= 0) return presence;
    if (acl_valid(acl) != 0) { acl_free(acl); return -1; }
    acl_entry_t entry;
    int selector = ACL_FIRST_ENTRY, result = 0;
    for (;;) {
        errno = 0;
        if (acl_get_entry(acl, selector, &entry) != 0) {
            if (errno != EINVAL) result = -1;
            break;
        }
        selector = ACL_NEXT_ENTRY;
        acl_tag_t tag;
        if (acl_get_tag_type(entry, &tag) != 0) { result = -1; break; }
        if (tag == ACL_EXTENDED_DENY) {
            if (throne_acl_decision(1, 0, 1, 0)) { result = -1; break; }
            continue;
        }
        if (tag != ACL_EXTENDED_ALLOW) { result = -1; break; }
        acl_permset_t perms;
        if (acl_get_permset(entry, &perms) != 0) { result = -1; break; }
        acl_perm_t dangerous[] = { ACL_WRITE_DATA, ACL_APPEND_DATA, ACL_DELETE,
            ACL_DELETE_CHILD, ACL_WRITE_ATTRIBUTES, ACL_WRITE_EXTATTRIBUTES,
            ACL_WRITE_SECURITY, ACL_CHANGE_OWNER };
        for (size_t i = 0; i < sizeof(dangerous)/sizeof(dangerous[0]); i++) {
            int granted = acl_get_perm_np(perms, dangerous[i]);
            if (throne_acl_decision(0, 1, granted >= 0, granted != 0)) { result = -1; break; }
        }
        if (result) break;
    }
    acl_free(acl);
    return result;
}

// Release signatures must not opt out of the injection/debugging protections
// this policy relies on. Absent/false entitlements are safe; malformed values
// and unreadable signing metadata fail closed, for both installed and live code.
static int throne_release_info(CFDictionaryRef info) {
    if (!info || CFGetTypeID(info) != CFDictionaryGetTypeID()) return -1;
    uint32_t flags = 0;
    CFNumberRef number = CFDictionaryGetValue(info, kSecCodeInfoFlags);
    CFStringRef team = CFDictionaryGetValue(info, kSecCodeInfoTeamIdentifier);
    if (!number || CFGetTypeID(number) != CFNumberGetTypeID() ||
        !CFNumberGetValue(number, kCFNumberSInt32Type, &flags) ||
        !(flags & 0x10000) || (flags & 0x2) || !team ||
        CFGetTypeID(team) != CFStringGetTypeID() || CFStringGetLength(team) == 0) return -1;
    CFDictionaryRef entitlements = CFDictionaryGetValue(info, kSecCodeInfoEntitlementsDict);
    if (!entitlements) return 0; // Signed programs may have no entitlements.
    if (CFGetTypeID(entitlements) != CFDictionaryGetTypeID()) return -1;
    CFStringRef dangerous[] = {
        CFSTR("get-task-allow"), CFSTR("com.apple.security.get-task-allow"),
        CFSTR("com.apple.security.cs.disable-library-validation"),
        CFSTR("com.apple.security.cs.allow-dyld-environment-variables"),
        CFSTR("com.apple.security.cs.allow-unsigned-executable-memory"),
        CFSTR("com.apple.security.cs.disable-executable-page-protection"),
        CFSTR("com.apple.security.cs.allow-jit"),
        CFSTR("com.apple.security.cs.debugger")
    };
    for (size_t i = 0; i < sizeof(dangerous)/sizeof(dangerous[0]); i++) {
        CFTypeRef value = CFDictionaryGetValue(entitlements, dangerous[i]);
        int boolean = value && CFGetTypeID(value) == CFBooleanGetTypeID();
        if (throne_entitlement_decision(value != NULL, boolean,
                                       boolean ? CFBooleanGetValue(value) : 0)) return -1;
    }
    return 0;
}

static int throne_local_development_install(void) {
    const char *marker = "/Library/Application Support/Throne GTK/.throne-gtk-manual";
    struct stat st;
    // Callers have already validated the protected installation ancestors.
    return lstat(marker, &st) == 0 && S_ISREG(st.st_mode) && st.st_uid == 0 &&
        !(st.st_mode & 0022) && throne_safe_acl(marker) == 0;
}

// An ad-hoc designated requirement is not sufficient on its own. Pin the
// protected installed image's CodeDirectory hash; release images must also be
// hardened and satisfy their designated requirement (team/identifier anchor).
// Ad-hoc mode is explicitly local development only: hash identity does not
// promise hardened-runtime injection resistance.
static int throne_signature(const char *path, int pid) {
    CFURLRef url = CFURLCreateFromFileSystemRepresentation(NULL, (const UInt8*)path, strlen(path), false);
    if (!url) return -1;
    SecStaticCodeRef installed = NULL;
    SecRequirementRef requirement = NULL;
    SecCodeRef live = NULL;
    CFDictionaryRef info = NULL;
    CFDictionaryRef liveInfo = NULL;
    CFNumberRef number = NULL;
    CFDictionaryRef attrs = NULL;
    OSStatus s = SecStaticCodeCreateWithPath(url, kSecCSDefaultFlags, &installed);
    if (!s) s = SecStaticCodeCheckValidity(installed, kSecCSStrictValidate | kSecCSCheckAllArchitectures, NULL);
    if (!s) s = SecCodeCopySigningInformation(installed, kSecCSSigningInformation, &info);
    uint32_t flags = 0;
    CFNumberRef flagNumber = info ? CFDictionaryGetValue(info, kSecCodeInfoFlags) : NULL;
    if (!s && (!flagNumber || CFGetTypeID(flagNumber) != CFNumberGetTypeID() || !CFNumberGetValue(flagNumber, kCFNumberSInt32Type, &flags))) s = -1;
    if (!s && (flags & 0x2)) { // CS_ADHOC
        if (!throne_local_development_install()) s = -1;
        CFDataRef hash = CFDictionaryGetValue(info, kSecCodeInfoUnique);
        if (!hash || CFGetTypeID(hash) != CFDataGetTypeID() || CFDataGetLength(hash) != 20) s = -1;
        if (!s) {
            char expression[64] = "cdhash H\"";
            const UInt8 *bytes = CFDataGetBytePtr(hash);
            for (int i = 0; i < 20; i++) sprintf(expression + 9 + i*2, "%02x", bytes[i]);
            strcat(expression, "\"");
            CFStringRef text = CFStringCreateWithCString(NULL, expression, kCFStringEncodingASCII);
            s = SecRequirementCreateWithString(text, kSecCSDefaultFlags, &requirement);
            CFRelease(text);
        }
    } else if (!s) {
        if (throne_release_info(info)) s = -1;
        if (!s) s = SecCodeCopyDesignatedRequirement(installed, kSecCSDefaultFlags, &requirement);
    }
    number = CFNumberCreate(NULL, kCFNumberIntType, &pid);
    const void *keys[] = {kSecGuestAttributePid}; const void *values[] = {number};
    attrs = CFDictionaryCreate(NULL, keys, values, 1, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    if (!s) s = SecCodeCopyGuestWithAttributes(NULL, attrs, kSecCSDefaultFlags, &live);
    if (!s) s = SecCodeCheckValidity(live, kSecCSStrictValidate, requirement);
    if (!s && !(flags & 0x2)) {
        s = SecCodeCopySigningInformation(live, kSecCSSigningInformation, &liveInfo);
        if (!s && throne_release_info(liveInfo)) s = -1;
        if (!s && !CFEqual(CFDictionaryGetValue(info, kSecCodeInfoTeamIdentifier),
                          CFDictionaryGetValue(liveInfo, kSecCodeInfoTeamIdentifier))) s = -1;
    }
    if (liveInfo) CFRelease(liveInfo);
    if (live) CFRelease(live);
    if (attrs) CFRelease(attrs);
    if (number) CFRelease(number);
    if (info) CFRelease(info);
    if (requirement) CFRelease(requirement);
    if (installed) CFRelease(installed);
    CFRelease(url);
    return (int)s;
}
