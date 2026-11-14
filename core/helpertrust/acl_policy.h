// Portable decision shared by the native ACL reader and its host-side tests.
// Unknown entries/permissions never grant access to the protected closure.
static int throne_acl_decision(int deny, int allow, int known, int mutation) {
    if (!known) return -1;
    if (deny) return 0;
    if (!allow || mutation) return -1;
    return 0;
}

static int throne_entitlement_decision(int present, int boolean, int enabled) {
    return present && (!boolean || enabled) ? -1 : 0;
}
