//go:build darwin && cgo

#include "dns_native_darwin.h"
#include <CoreFoundation/CoreFoundation.h>
#include <SystemConfiguration/SystemConfiguration.h>
#include <stdlib.h>
#include <string.h>
#include <limits.h>
#include <sys/acl.h>
#include <errno.h>
#include "../../helpertrust/acl_policy.h"
#include "acl_presence.h"

struct nolurix_dns_tx {
 SCPreferencesRef prefs;
 SCNetworkServiceRef service;
 char *service_id, *location_id;
};

static char *string_copy(CFStringRef value) {
 if (!value) return NULL;
 CFIndex length=CFStringGetMaximumSizeForEncoding(CFStringGetLength(value),kCFStringEncodingUTF8)+1;
 char *result=malloc(length);
 if (result && !CFStringGetCString(value,result,length,kCFStringEncodingUTF8)) { free(result); return NULL; }
 return result;
}
void nolurix_dns_end(nolurix_dns_tx *tx) {
 if (!tx) return;
 if (tx->service) CFRelease(tx->service);
 if (tx->prefs) { SCPreferencesUnlock(tx->prefs); CFRelease(tx->prefs); }
 free(tx->service_id); free(tx->location_id); free(tx);
}
nolurix_dns_tx *nolurix_dns_begin(const char *interface, const char *service, const char *location, int *status) {
 *status=0;
 nolurix_dns_tx *tx=calloc(1,sizeof(*tx));
 if (!tx) { *status=-1; return NULL; }
 tx->prefs=SCPreferencesCreate(NULL,CFSTR("Nolurix DNS lease"),NULL);
 if (!tx->prefs) { *status=SCError(); nolurix_dns_end(tx); return NULL; }
 // Never wait behind another preferences writer while holding the DNS lease.
 if (!SCPreferencesLock(tx->prefs,0)) {
  *status=SCError();
  if (*status==kSCStatusPrefsBusy) *status=NOLURIX_DNS_PREFS_BUSY;
  CFRelease(tx->prefs); tx->prefs=NULL; nolurix_dns_end(tx); return NULL;
 }
 SCPreferencesSynchronize(tx->prefs);
 SCNetworkSetRef set=SCNetworkSetCopyCurrent(tx->prefs);
 if (!set) { *status=10001; nolurix_dns_end(tx); return NULL; }
 tx->location_id=string_copy(SCNetworkSetGetSetID(set));
 if (!tx->location_id || (*location && strcmp(location,tx->location_id)!=0)) {
  *status=10001; CFRelease(set); nolurix_dns_end(tx); return NULL;
 }
 CFArrayRef services=SCNetworkSetCopyServices(set); CFRelease(set);
 if (services) {
  for (CFIndex i=0;i<CFArrayGetCount(services);i++) {
   SCNetworkServiceRef candidate=(SCNetworkServiceRef)CFArrayGetValueAtIndex(services,i);
   char *id=string_copy(SCNetworkServiceGetServiceID(candidate));
   SCNetworkInterfaceRef device=SCNetworkServiceGetInterface(candidate);
   char *name=device ? string_copy(SCNetworkInterfaceGetBSDName(device)) : NULL;
   int matches=(*service) ? (id && strcmp(service,id)==0) :
     (name && *interface && strcmp(interface,name)==0 && SCNetworkServiceGetEnabled(candidate));
   // Recovery also checks interface identity, not just a potentially reused ID.
   if (matches && *interface && (!name || strcmp(interface,name)!=0)) matches=0;
   if (matches) {
    if (tx->service) { *status=10002; free(id); free(name); CFRelease(services); nolurix_dns_end(tx); return NULL; }
    tx->service=(SCNetworkServiceRef)CFRetain(candidate); tx->service_id=id; id=NULL;
   }
   free(id); free(name);
  }
  CFRelease(services);
 }
 if (!tx->service) { *status=10002; nolurix_dns_end(tx); return NULL; }
 return tx;
}
const char *nolurix_dns_service(nolurix_dns_tx *tx) { return tx->service_id; }
const char *nolurix_dns_location(nolurix_dns_tx *tx) { return tx->location_id; }

static CFPropertyListRef decode(const unsigned char *bytes,int length) {
 CFDataRef data=CFDataCreate(NULL,bytes,length);
 if (!data) return NULL;
 CFPropertyListRef result=CFPropertyListCreateWithData(NULL,data,kCFPropertyListImmutable,NULL,NULL);
 CFRelease(data); return result;
}
// Enumerate protocols rather than treating any failed CopyProtocol call as an
// absent protocol: an API read error must never become an invented snapshot.
static int copy_dns_protocol(nolurix_dns_tx *tx,SCNetworkProtocolRef *result,int *status) {
 *result=NULL;
 CFArrayRef protocols=SCNetworkServiceCopyProtocols(tx->service);
 if (!protocols) { *status=SCError(); return 0; }
 for (CFIndex i=0;i<CFArrayGetCount(protocols);i++) {
  SCNetworkProtocolRef protocol=(SCNetworkProtocolRef)CFArrayGetValueAtIndex(protocols,i);
  if (CFEqual(SCNetworkProtocolGetProtocolType(protocol),kSCNetworkProtocolTypeDNS)) {
   *result=(SCNetworkProtocolRef)CFRetain(protocol); break;
  }
 }
 CFRelease(protocols); return 1;
}
unsigned char *nolurix_dns_snapshot(nolurix_dns_tx *tx,const char *address,int *length,int *status) {
 *status=0; *length=0;
 SCNetworkProtocolRef protocol=NULL;
 if (!copy_dns_protocol(tx,&protocol,status)) return NULL;
 CFDictionaryRef configuration=protocol ? SCNetworkProtocolGetConfiguration(protocol) : NULL;
 CFMutableDictionaryRef snapshot=CFDictionaryCreateMutable(NULL,0,&kCFTypeDictionaryKeyCallBacks,&kCFTypeDictionaryValueCallBacks);
 CFDictionarySetValue(snapshot,CFSTR("Present"),(address || protocol) ? kCFBooleanTrue : kCFBooleanFalse);
 CFDictionarySetValue(snapshot,CFSTR("Enabled"),(address || (protocol && SCNetworkProtocolGetEnabled(protocol))) ? kCFBooleanTrue : kCFBooleanFalse);
 if (address) {
  CFMutableDictionaryRef intended=configuration ? CFDictionaryCreateMutableCopy(NULL,0,configuration) : CFDictionaryCreateMutable(NULL,0,&kCFTypeDictionaryKeyCallBacks,&kCFTypeDictionaryValueCallBacks);
  CFStringRef server=CFStringCreateWithCString(NULL,address,kCFStringEncodingUTF8);
  const void *values[]={server};
  CFArrayRef servers=CFArrayCreate(NULL,values,1,&kCFTypeArrayCallBacks);
  CFDictionarySetValue(intended,kSCPropNetDNSServerAddresses,servers);
  CFDictionarySetValue(snapshot,CFSTR("Configuration"),intended);
  CFRelease(servers); CFRelease(server); CFRelease(intended);
 } else if (configuration) CFDictionarySetValue(snapshot,CFSTR("Configuration"),configuration);
 CFDataRef encoded=CFPropertyListCreateData(NULL,snapshot,kCFPropertyListXMLFormat_v1_0,0,NULL);
 CFRelease(snapshot); if (protocol) CFRelease(protocol);
 if (!encoded || CFDataGetLength(encoded)>INT_MAX) { if (encoded) CFRelease(encoded); *status=10003; return NULL; }
 *length=(int)CFDataGetLength(encoded);
 unsigned char *result=malloc(*length);
 if (result) memcpy(result,CFDataGetBytePtr(encoded),*length); else *status=-1;
 CFRelease(encoded); return result;
}
int nolurix_dns_equal(const unsigned char *a,int na,const unsigned char *b,int nb) {
 CFPropertyListRef av=decode(a,na),bv=decode(b,nb);
 int equal=av && bv && CFEqual(av,bv);
 if (av) CFRelease(av); if (bv) CFRelease(bv); return equal;
}
int nolurix_dns_set(nolurix_dns_tx *tx,const unsigned char *data,int length,int *status) {
 *status=0;
 CFPropertyListRef value=decode(data,length);
 if (!value || CFGetTypeID(value)!=CFDictionaryGetTypeID()) { if (value) CFRelease(value); *status=10003; return 0; }
 CFDictionaryRef snapshot=(CFDictionaryRef)value;
 CFBooleanRef present=CFDictionaryGetValue(snapshot,CFSTR("Present"));
 CFBooleanRef enabled=CFDictionaryGetValue(snapshot,CFSTR("Enabled"));
 CFDictionaryRef config=CFDictionaryGetValue(snapshot,CFSTR("Configuration"));
 if (!present || !enabled || CFGetTypeID(present)!=CFBooleanGetTypeID() || CFGetTypeID(enabled)!=CFBooleanGetTypeID() || (config && CFGetTypeID(config)!=CFDictionaryGetTypeID())) {
  CFRelease(value); *status=10003; return 0;
 }
 SCNetworkProtocolRef protocol=NULL;
 if (!copy_dns_protocol(tx,&protocol,status)) { CFRelease(value); return 0; }
 int ok=1;
 if (!CFBooleanGetValue(present)) {
  if (protocol) ok=SCNetworkServiceRemoveProtocolType(tx->service,kSCNetworkProtocolTypeDNS);
 } else {
  if (!protocol) {
   ok=SCNetworkServiceAddProtocolType(tx->service,kSCNetworkProtocolTypeDNS);
   if (ok) protocol=SCNetworkServiceCopyProtocol(tx->service,kSCNetworkProtocolTypeDNS);
  }
  if (ok) ok=protocol && SCNetworkProtocolSetConfiguration(protocol,config);
  if (ok) ok=SCNetworkProtocolSetEnabled(protocol,CFBooleanGetValue(enabled));
 }
 if (!ok) *status=SCError();
 if (protocol) CFRelease(protocol); CFRelease(value); return ok;
}
int nolurix_dns_commit(nolurix_dns_tx *tx,int *status) {
 int ok=SCPreferencesCommitChanges(tx->prefs); *status=ok ? 0 : SCError(); return ok;
}
int nolurix_dns_apply(nolurix_dns_tx *tx,int *status) {
 int ok=SCPreferencesApplyChanges(tx->prefs); *status=ok ? 0 : SCError(); return ok;
}

// Inspect the already-open inode, never a path that can be substituted. Match
// helpertrust's policy: deny/read-only ACEs are safe; write grants, unknown
// ACEs, unreadable ACLs and unreadable permission bits all fail closed.
int nolurix_dns_safe_acl(int fd) {
 acl_t acl=NULL;
 int presence=throne_acl_read(fd,NULL,&acl);
 if (presence<=0) return presence;
 if (acl_valid(acl)!=0) { acl_free(acl); return -1; }
 acl_entry_t entry;
 int selector=ACL_FIRST_ENTRY,result=0;
 for (;;) {
  errno=0;
  if (acl_get_entry(acl,selector,&entry)!=0) {
   if (errno!=EINVAL) result=-1;
   break;
  }
  selector=ACL_NEXT_ENTRY;
  acl_tag_t tag;
  if (acl_get_tag_type(entry,&tag)!=0) { result=-1; break; }
  if (tag==ACL_EXTENDED_DENY) {
   if (throne_acl_decision(1,0,1,0)) { result=-1; break; }
   continue;
  }
  if (tag!=ACL_EXTENDED_ALLOW) { result=-1; break; }
  acl_permset_t perms;
  if (acl_get_permset(entry,&perms)!=0) { result=-1; break; }
  acl_perm_t dangerous[]={ACL_WRITE_DATA,ACL_APPEND_DATA,ACL_DELETE,
   ACL_DELETE_CHILD,ACL_WRITE_ATTRIBUTES,ACL_WRITE_EXTATTRIBUTES,
   ACL_WRITE_SECURITY,ACL_CHANGE_OWNER};
  for (size_t i=0;i<sizeof(dangerous)/sizeof(dangerous[0]);i++) {
   int granted=acl_get_perm_np(perms,dangerous[i]);
   if (throne_acl_decision(0,1,granted>=0,granted!=0)) { result=-1; break; }
  }
  if (result) break;
 }
 acl_free(acl); return result;
}
