//go:build !darwin

package main

func runPlatformMode() bool { return false }

func trustedGUIOverrideAllowed() bool { return false }
