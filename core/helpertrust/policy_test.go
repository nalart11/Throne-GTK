package helpertrust

import (
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

type node struct{ mode fs.FileMode }

func (n node) Name() string       { return "test" }
func (n node) Size() int64        { return 0 }
func (n node) Mode() fs.FileMode  { return n.mode }
func (n node) ModTime() time.Time { return time.Time{} }
func (n node) IsDir() bool        { return n.mode.IsDir() }
func (n node) Sys() any           { return nil }

func TestMandatoryNodePolicy(t *testing.T) {
	for _, tc := range []struct {
		name string
		mode fs.FileMode
		uid  uint32
		fail bool
	}{
		{"root", 0755, 0, false}, {"user", 0755, 501, true}, {"admin writable", 0775, 0, true},
		{"world writable", 0777, 0, true}, {"symlink", fs.ModeSymlink | 0755, 0, true},
	} {
		t.Run(tc.name, func(t *testing.T) {
			if got := CheckNode("/Applications", node{tc.mode}, tc.uid); (got != nil) != tc.fail {
				t.Fatalf("error = %v", got)
			}
		})
	}
}

func TestContainedFrameworkLinks(t *testing.T) {
	bundle := t.TempDir()
	frameworks := filepath.Join(bundle, "Contents", "Frameworks")
	if err := os.MkdirAll(frameworks, 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(frameworks, "real"), []byte("code"), 0644); err != nil {
		t.Fatal(err)
	}
	for _, tc := range []struct {
		name, target string
		valid        bool
	}{
		{"alias", "real", true}, {"escape", "../../../outside", false},
		{"dangling", "missing", false}, {"cycle", "cycle", false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			link := filepath.Join(frameworks, tc.name)
			if err := os.Symlink(tc.target, link); err != nil {
				t.Fatal(err)
			}
			err := CheckBundleLink(bundle, link)
			// Ordinary CI users cannot manufacture a root-owned link. The
			// positive case must reach the ownership guard, not fail resolution.
			if tc.valid && os.Getuid() != 0 {
				if err == nil || !strings.Contains(err.Error(), "owner") {
					t.Fatalf("%v", err)
				}
			} else if (err == nil) != tc.valid {
				t.Fatalf("error = %v", err)
			}
		})
	}
}

func TestContainedFrameworkLinksThroughSymlinkedAncestor(t *testing.T) {
	root := t.TempDir()
	realParent := filepath.Join(root, "private", "var")
	realBundle := filepath.Join(realParent, "Nolurix.app")
	frameworks := filepath.Join(realBundle, "Contents", "Frameworks")
	if err := os.MkdirAll(frameworks, 0755); err != nil {
		t.Fatal(err)
	}
	// Model macOS /var -> /private/var without relying on the host layout.
	aliasParent := filepath.Join(root, "var")
	if err := os.Symlink(realParent, aliasParent); err != nil {
		t.Fatal(err)
	}
	bundle := filepath.Join(aliasParent, "Nolurix.app")
	for _, path := range []string{filepath.Join(frameworks, "real"), filepath.Join(root, "outside")} {
		if err := os.WriteFile(path, []byte("code"), 0644); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.Symlink(filepath.Join(root, "outside"), filepath.Join(frameworks, "redirect")); err != nil {
		t.Fatal(err)
	}
	for _, tc := range []struct {
		name, target string
		valid        bool
	}{
		{"alias", "real", true},
		{"outside", filepath.Join(root, "outside"), false},
		{"resolved-outside", "redirect", false},
		{"dangling", "missing", false},
		{"cycle", "cycle", false},
	} {
		t.Run(tc.name, func(t *testing.T) {
			link := filepath.Join(bundle, "Contents", "Frameworks", tc.name)
			if err := os.Symlink(tc.target, link); err != nil {
				t.Fatal(err)
			}
			err := CheckBundleLink(bundle, link)
			if tc.valid && os.Getuid() != 0 {
				if err == nil || !strings.Contains(err.Error(), "owner") {
					t.Fatalf("contained link should reach ownership guard: %v", err)
				}
			} else if (err == nil) != tc.valid {
				t.Fatalf("error = %v", err)
			} else if !tc.valid && strings.Contains(err.Error(), "owner") {
				t.Fatalf("invalid link should fail before ownership check: %v", err)
			}
		})
	}
	// Canonical containment is not permission to trust a symlinked install
	// ancestor: the independent ancestor guard must still reject this alias.
	if err := CheckAncestors(aliasParent, func(string) error { return nil }); err == nil || !strings.Contains(err.Error(), "symlink") {
		t.Fatalf("symlinked installation ancestor accepted: %v", err)
	}
}

func TestNativePolicySourceContract(t *testing.T) {
	data, err := os.ReadFile("security.h")
	if err != nil {
		t.Fatal(err)
	}
	source := string(data)
	for _, guard := range []string{"tag == ACL_EXTENDED_DENY", "tag != ACL_EXTENDED_ALLOW", "ACL_DELETE_CHILD", "ACL_WRITE_SECURITY", "ACL_CHANGE_OWNER", "kSecCodeInfoUnique", "cdhash H", "kSecCSStrictValidate", "kSecCodeInfoTeamIdentifier"} {
		if !strings.Contains(source, guard) {
			t.Errorf("missing native policy guard %s", guard)
		}
	}
	for _, guard := range []string{
		"throne_release_info(info)", "throne_release_info(liveInfo)",
		"if (!throne_local_development_install()) s = -1;", ".nolurix-manual",
		"SecCodeCopySigningInformation(live,", "com.apple.security.get-task-allow",
		"com.apple.security.cs.disable-library-validation",
		"com.apple.security.cs.allow-dyld-environment-variables",
		"com.apple.security.cs.allow-unsigned-executable-memory",
		"com.apple.security.cs.disable-executable-page-protection",
		"com.apple.security.cs.allow-jit", "com.apple.security.cs.debugger",
	} {
		if !strings.Contains(source, guard) {
			t.Errorf("missing release guard %s", guard)
		}
	}
}

func TestSingleAuthenticatedControlConnectionSourceContract(t *testing.T) {
	data, err := os.ReadFile("../../crates/ipc/src/service_macos.rs")
	if err != nil {
		t.Fatal(err)
	}
	source := string(data)
	start := strings.Index(source, "async fn start_inner(")
	if start < 0 {
		t.Fatal("missing bootstrap function")
	}
	end := strings.Index(source[start:], "let path = endpoint")
	if start < 0 || end < 0 {
		t.Fatal("missing bootstrap function")
	}
	bootstrap := source[start : start+end]
	if strings.Contains(bootstrap, "drop(stream)") {
		t.Fatal("probe connection reintroduced")
	}
	for _, guard := range []string{
		"validate_helper_endpoint()?;", "authenticate_helper(&stream)?;",
		"std::io::ErrorKind::NotFound", "std::io::ErrorKind::ConnectionRefused",
		"if manual_installation()?", "Err(error) => return Err(error)",
	} {
		if !strings.Contains(bootstrap, guard) {
			t.Errorf("missing bootstrap guard %s", guard)
		}
	}
	if strings.Count(bootstrap, "authenticate_helper(&stream)?;") != 1 {
		t.Fatal("expected one authentication of the original stream")
	}
}

func TestPortableACLDecisions(t *testing.T) {
	cc, err := exec.LookPath("cc")
	if err != nil {
		t.Skip("C compiler unavailable")
	}
	dir := t.TempDir()
	include, err := filepath.Abs(".")
	if err != nil {
		t.Fatal(err)
	}
	source := `#include "acl_policy.h"
int main(void) {
 if (throne_acl_decision(1,0,1,1) != 0) return 1; /* deny delete */
 if (throne_acl_decision(0,1,1,0) != 0) return 2; /* read allow */
 if (throne_acl_decision(0,1,1,1) == 0) return 3; /* mutation allow */
 if (throne_acl_decision(0,0,1,0) == 0) return 4; /* unknown ACE */
 if (throne_acl_decision(0,1,0,0) == 0) return 5; /* unreadable permission */
 if (throne_entitlement_decision(0,0,0) != 0) return 6; /* absent */
 if (throne_entitlement_decision(1,1,0) != 0) return 7; /* explicit false */
 if (throne_entitlement_decision(1,1,1) == 0) return 8; /* enabled */
 if (throne_entitlement_decision(1,0,0) == 0) return 9; /* malformed */
 return 0;
}`
	input, binary := filepath.Join(dir, "policy.c"), filepath.Join(dir, "policy")
	if err := os.WriteFile(input, []byte(source), 0600); err != nil {
		t.Fatal(err)
	}
	if out, err := exec.Command(cc, "-I", include, input, "-o", binary).CombinedOutput(); err != nil {
		t.Fatalf("%s: %v", out, err)
	}
	if out, err := exec.Command(binary).CombinedOutput(); err != nil {
		t.Fatalf("%s: %v", out, err)
	}
}

func TestLayoutAlwaysEnforced(t *testing.T) {
	// Debug is deliberately not a policy input, nor is it a build-tag bypass.
	t.Setenv("NOLURIX_CORE_DEBUG", "1")
	if err := CheckLayout(Core, GUI); err != nil {
		t.Fatal(err)
	}
	for _, pair := range [][2]string{{"/tmp/nolurix-core", GUI}, {Core, "/tmp/nolurix"}, {Core, Bundle + "/Contents/MacOS/../MacOS/nolurix"}} {
		if CheckLayout(pair[0], pair[1]) == nil {
			t.Fatal("accepted wrong layout in debug mode")
		}
	}
}
