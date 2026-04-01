package main

import (
	"testing"

	"db/internal/config"
)

func TestSplitListenAddr(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name     string
		addr     string
		wantHost string
		wantPort string
	}{
		{name: "empty", addr: "", wantHost: "0.0.0.0", wantPort: "8080"},
		{name: "all interfaces", addr: ":8080", wantHost: "0.0.0.0", wantPort: "8080"},
		{name: "explicit host", addr: "127.0.0.1:9090", wantHost: "127.0.0.1", wantPort: "9090"},
	}

	for _, tt := range tests {
		tt := tt
		t.Run(tt.name, func(t *testing.T) {
			t.Parallel()

			host, port := splitListenAddr(tt.addr)
			if host != tt.wantHost || port != tt.wantPort {
				t.Fatalf("got host=%q port=%q", host, port)
			}
		})
	}
}

func TestRedactPostgresDSNURL(t *testing.T) {
	t.Parallel()

	got := redactPostgresDSN("postgres://reader:secret@localhost:5432/app?sslmode=disable")
	want := "postgres://reader@localhost:5432/app?sslmode=disable"
	if got != want {
		t.Fatalf("unexpected redacted dsn: %q", got)
	}
}

func TestRedactPostgresDSNKeywordFormat(t *testing.T) {
	t.Parallel()

	got := redactPostgresDSN("host=localhost port=5432 user=reader password=secret dbname=app sslmode=disable")
	want := "host=localhost port=5432 user=reader password=REDACTED dbname=app sslmode=disable"
	if got != want {
		t.Fatalf("unexpected redacted dsn: %q", got)
	}
}

func TestConnectionTargetSummaryUsesRedactedPostgresDSN(t *testing.T) {
	t.Parallel()

	got := connectionTargetSummary(config.ConnectionConfig{
		Driver: "postgres",
		Postgres: &config.PostgresConfig{
			DSN: "postgres://reader:secret@localhost:5432/app?sslmode=disable",
		},
	})
	want := "postgres://reader@localhost:5432/app?sslmode=disable"
	if got != want {
		t.Fatalf("unexpected postgres target summary: %q", got)
	}
}
