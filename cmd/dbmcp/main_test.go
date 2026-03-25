package main

import "testing"

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
