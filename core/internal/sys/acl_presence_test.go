package sys

import (
	"os"
	"os/exec"
	"path/filepath"
	"testing"
)

func TestACLAbsenceRequiresSuccessfulMetadataAndQuery(t *testing.T) {
	compiler, err := exec.LookPath("cc")
	if err != nil {
		t.Skip("C compiler unavailable")
	}
	include, err := filepath.Abs(".")
	if err != nil {
		t.Fatal(err)
	}
	dir := t.TempDir()
	source := filepath.Join(dir, "presence.c")
	binary := filepath.Join(dir, "presence")
	code := `#include "acl_presence.h"
int main(void) {
 for (int metadata=0;metadata<=1;metadata++)
  for (int query=0;query<=1;query++)
   for (int present=-1;present<=2;present++) {
    int expected=(metadata && query && (present==0 || present==1)) ? present : -1;
    if (throne_acl_presence(metadata,query,present)!=expected) return 1;
   }
 return 0;
}`
	if err = os.WriteFile(source, []byte(code), 0600); err != nil {
		t.Fatal(err)
	}
	if output, err := exec.Command(compiler, "-I", include, source, "-o", binary).CombinedOutput(); err != nil {
		t.Fatalf("compile: %v\n%s", err, output)
	}
	if output, err := exec.Command(binary).CombinedOutput(); err != nil {
		t.Fatalf("decision matrix: %v\n%s", err, output)
	}
}
