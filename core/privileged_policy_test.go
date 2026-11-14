package main

import (
	"context"
	"testing"
)

func TestPrivilegedPolicyAdmission(t *testing.T) {
	privilegedWorker.Store(false)
	t.Cleanup(func() { privilegedWorker.Store(false) })
	if err := AuthorizePrivilegedWorkerMethod("arbitrary ordinary method"); err != nil {
		t.Fatal(err)
	}
	enablePrivilegedWorkerPolicy()
	for _, method := range []string{"Start", "Stop", "IsPrivileged", "GetDefaultInterface", "QueryStats", "QueryConnections", "QueryAutoSelectors", "AutoSelectorAction"} {
		if err := AuthorizePrivilegedWorkerMethod(method); err != nil {
			t.Fatalf("%s: %v", method, err)
		}
	}
	for _, method := range []string{"CheckConfig", "Test", "SpeedTest", "IPTest", "SetSystemDNS", "GenWgKeyPair", "unknown"} {
		if err := AuthorizePrivilegedWorkerMethod(method); err == nil {
			t.Fatalf("authorized %s", method)
		}
	}
	// Invalid protobuf must be rejected by policy, not decoded or constructed.
	for method := range handlers {
		if AuthorizePrivilegedWorkerMethod(method) == nil {
			continue
		}
		_, err := dispatch(method, []byte{0xff})
		if err == nil || err.Error() != "privileged worker: method not authorized" {
			t.Fatalf("%s did not reject before decoding: %v", method, err)
		}
	}
}

func TestPrivilegedPolicyDirectConstructionGuards(t *testing.T) {
	enablePrivilegedWorkerPolicy()
	t.Cleanup(func() { privilegedWorker.Store(false) })
	s := &server{}
	ctx := context.Background()
	// Nil requests would panic if any configuration fields were accessed. These
	// checks establish denial before file/process/core construction, even for
	// direct callers that bypass the wire dispatcher.
	if out, err := s.Start(ctx, nil); err != nil || out.GetError() == "" {
		t.Fatalf("Start: %v %v", out, err)
	}
	if out, err := s.CheckConfig(ctx, nil); err != nil || out.GetError() == "" {
		t.Fatalf("CheckConfig: %v %v", out, err)
	}
	if _, err := s.Test(ctx, nil); err == nil {
		t.Fatal("Test allowed")
	}
	if _, err := s.SpeedTest(ctx, nil); err == nil {
		t.Fatal("SpeedTest allowed")
	}
	if _, err := s.IPTest(ctx, nil); err == nil {
		t.Fatal("IPTest allowed")
	}
}
