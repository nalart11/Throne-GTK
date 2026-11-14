package main

import (
	"context"
	"errors"
	"os"
	"testing"
	"time"

	"github.com/nalart11/Nolurix/core/gen"
)

func minimalStartRequest() *gen.LoadConfigReq {
	return &gen.LoadConfigReq{CoreConfig: To(`{"outbounds":[{"type":"direct","tag":"direct"}]}`)}
}

func TestRepeatedStartPreservesOwnedInstance(t *testing.T) {
	s := &server{}
	defer s.Stop(context.Background(), &gen.EmptyReq{})
	first, err := s.Start(context.Background(), minimalStartRequest())
	if err != nil || first.GetError() != "" {
		t.Fatalf("initial Start failed: %v %s", err, first.GetError())
	}
	original, _ := currentInstance()
	if original == nil {
		t.Fatal("Start did not publish instance")
	}
	second, err := s.Start(context.Background(), minimalStartRequest())
	if err != nil || second.GetError() != "instance already started" {
		t.Fatalf("unexpected repeated Start: %v %s", err, second.GetError())
	}
	if currentBox() != original {
		t.Fatal("rejected Start lost original owner")
	}
	if _, err := s.Stop(context.Background(), &gen.EmptyReq{}); err != nil {
		t.Fatal(err)
	}
	if currentBox() != nil {
		t.Fatal("Stop retained original")
	}
	if err := original.Close(); !errors.Is(err, os.ErrClosed) {
		t.Fatalf("Stop did not close original: %v", err)
	}
}

func TestCancelledQueuedStartCannotPublishAfterStop(t *testing.T) {
	s := &server{}
	ctx, cancel := context.WithCancel(context.Background())
	lifecycleMu.Lock()
	done := make(chan *gen.ErrorResp, 1)
	go func() { out, _ := s.Start(ctx, minimalStartRequest()); done <- out }()
	// Hold the lifecycle lock throughout cancellation, just as a concurrent
	// Stop does. Whether Start reached the mutex yet cannot affect the result.
	cancel()
	lifecycleMu.Unlock()
	s.Stop(context.Background(), &gen.EmptyReq{})
	select {
	case out := <-done:
		if out.GetError() != context.Canceled.Error() {
			t.Fatalf("expected cancellation: %s", out.GetError())
		}
	case <-time.After(5 * time.Second):
		t.Fatal("queued Start did not finish")
	}
	if currentBox() != nil || autoRedirectMark.Load() != 0 {
		t.Fatal("cancelled Start published resources after Stop")
	}
}
