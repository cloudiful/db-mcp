package database

import (
	"testing"
)

func TestTruncateString(t *testing.T) {
	t.Parallel()

	value, chars, truncated := truncateString("abcdefgh", 4)
	if !truncated {
		t.Fatalf("expected truncated result")
	}
	if value != "abcd...(truncated)" {
		t.Fatalf("unexpected value: %s", value)
	}
	if chars <= 4 {
		t.Fatalf("expected char count to include suffix, got %d", chars)
	}
}
