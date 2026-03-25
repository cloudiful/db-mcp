package postgres

import "testing"

func TestFilterHelpers(t *testing.T) {
	t.Parallel()

	if got := exactFilter(" public "); got != "public" {
		t.Fatalf("unexpected exactFilter result: %q", got)
	}
	if got := likeFilter(" users "); got != "%users%" {
		t.Fatalf("unexpected likeFilter result: %q", got)
	}
}
