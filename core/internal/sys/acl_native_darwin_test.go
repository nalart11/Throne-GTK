//go:build darwin && cgo

package sys

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
)

// Exercise the real fd reader and BOTH path-reader call sites, without root or
// changes to system files. Keeping the wrappers in separate translation units
// also checks the installer guard independently from Security.framework code.
func TestNativeACLReaders(t *testing.T) {
	root, err := filepath.Abs("../../..")
	if err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	helper := filepath.Join(dir, "helper.c")
	installer := filepath.Join(dir, "installer.c")
	harness := filepath.Join(dir, "harness.c")
	binary := filepath.Join(dir, "acl-regression")
	sources := map[string]string{
		helper:    fmt.Sprintf("#include %q\nint test_helper_acl(const char *p) { return throne_safe_acl(p); }\n", filepath.Join(root, "core/helpertrust/security.h")),
		installer: fmt.Sprintf("#define main unused_installer_main\n#include %q\n#undef main\nint test_installer_acl(const char *p) { return safe_acl(p); }\n", filepath.Join(root, "scripts/check-macos-install-acl.c")),
		harness: `#include <sys/acl.h>
#include <sys/types.h>
#include <sys/stat.h>
#include <membership.h>
#include <uuid/uuid.h>
#include <unistd.h>
#include <fcntl.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include "dns_native_darwin.h"
int test_helper_acl(const char *);
int test_installer_acl(const char *);
static int fixture_error(void) {
 int saved=errno;
 fprintf(stderr,"ACL fixture failed: errno %d\n",saved);
 return saved==ENOTSUP || saved==EOPNOTSUPP || saved==EPERM || saved==EACCES ? 77 : 2;
}
int main(int argc,char **argv) {
 if (argc!=4) return 2;
 const char *path=argv[1], *kind=argv[2], *setter=argv[3];
 if (!strcmp(kind,"invalid")) {
  if (nolurix_dns_safe_acl(-1)==0 || test_helper_acl(path)==0 || test_installer_acl(path)==0) return 1;
  return 0;
 }
 int fd=open(path,O_RDWR|O_NOFOLLOW);
 if (fd<0) return 2;
 if (strcmp(kind,"absent")) {
  acl_t acl=acl_init(1);
  if (!acl) return 2;
  if (strcmp(kind,"zero")) {
   acl_entry_t entry;
   uuid_t owner;
   acl_permset_t perms;
   if (mbr_uid_to_uuid(getuid(),owner)!=0 || acl_create_entry(&acl,&entry)!=0 ||
       acl_set_tag_type(entry,!strcmp(kind,"deny") ? ACL_EXTENDED_DENY : ACL_EXTENDED_ALLOW)!=0 ||
       acl_set_qualifier(entry,&owner)!=0 || acl_get_permset(entry,&perms)!=0 || acl_clear_perms(perms)!=0 ||
       acl_add_perm(perms,!strcmp(kind,"write") ? ACL_WRITE_DATA : !strcmp(kind,"deny") ? ACL_DELETE : ACL_READ_DATA)!=0 ||
       acl_set_permset(entry,perms)!=0) return 2;
  }
  int result=!strcmp(setter,"fd") ? acl_set_fd_np(fd,acl,ACL_TYPE_EXTENDED) : acl_set_file(path,ACL_TYPE_EXTENDED,acl);
  acl_free(acl);
  if (result!=0) return fixture_error();
 }
 int expected=!strcmp(kind,"write") ? -1 : 0;
 int a=nolurix_dns_safe_acl(fd),b=test_helper_acl(path),c=test_installer_acl(path);
 close(fd);
 if ((a==0)!=(expected==0) || (b==0)!=(expected==0) || (c==0)!=(expected==0)) {
  fprintf(stderr,"kind=%s expected=%d fd=%d helper=%d installer=%d\n",kind,expected,a,b,c); return 1;
 }
 return 0;
}`,
	}
	for path, text := range sources {
		if err = os.WriteFile(path, []byte(text), 0600); err != nil {
			t.Fatal(err)
		}
	}
	args := []string{"-I", filepath.Join(root, "core/internal/sys"), "-I", filepath.Join(root, "core/helpertrust"), harness, helper, installer, filepath.Join(root, "core/internal/sys/dns_native_darwin.c"), "-framework", "CoreFoundation", "-framework", "SystemConfiguration", "-framework", "Security", "-o", binary}
	if output, err := exec.Command("/usr/bin/clang", args...).CombinedOutput(); err != nil {
		t.Fatalf("compile native ACL regression: %v\n%s", err, output)
	}
	for _, kind := range []string{"absent", "zero", "deny", "read", "write", "invalid"} {
		for _, setter := range []string{"fd", "path"} {
			t.Run(kind+"/"+setter, func(t *testing.T) {
				path := filepath.Join(t.TempDir(), "fixture")
				if kind != "invalid" {
					if err := os.WriteFile(path, []byte("fixture"), 0600); err != nil {
						t.Fatal(err)
					}
					defer func() {
						if output, err := exec.Command("/bin/chmod", "-N", path).CombinedOutput(); err != nil {
							t.Errorf("clear fixture ACL during cleanup: %v\n%s", err, output)
						}
					}()
					// Explicitly remove inherited ACLs for the absent case. Failure is not
					// silently skipped: this is the primary regression on ordinary Mac CI.
					if output, err := exec.Command("/bin/chmod", "-N", path).CombinedOutput(); err != nil {
						t.Fatalf("clear fixture ACL: %v\n%s", err, output)
					}
				}
				output, err := exec.Command(binary, path, kind, setter).CombinedOutput()
				if exit, ok := err.(*exec.ExitError); ok && exit.ExitCode() == 77 {
					t.Skipf("filesystem or sandbox forbids ACL fixtures: %s", output)
				}
				if err != nil {
					t.Fatalf("native ACL readers: %v\n%s", err, output)
				}
			})
		}
	}
}
