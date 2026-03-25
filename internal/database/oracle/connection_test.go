package oracle

import "testing"

func TestParseJDBCURL(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name        string
		jdbcURL     string
		wantHost    string
		wantPort    int
		wantService string
	}{
		{
			name:        "sid format",
			jdbcURL:     "jdbc:oracle:thin:@unraid.cloud1ful.com:1521:XE",
			wantHost:    "unraid.cloud1ful.com",
			wantPort:    1521,
			wantService: "XE",
		},
		{
			name:        "service format",
			jdbcURL:     "jdbc:oracle:thin:@//db.example.com:1521/orclpdb1",
			wantHost:    "db.example.com",
			wantPort:    1521,
			wantService: "orclpdb1",
		},
	}

	for _, tt := range tests {
		tt := tt
		t.Run(tt.name, func(t *testing.T) {
			t.Parallel()

			host, port, service, err := parseJDBCURL(tt.jdbcURL)
			if err != nil {
				t.Fatalf("unexpected error: %v", err)
			}
			if host != tt.wantHost || port != tt.wantPort || service != tt.wantService {
				t.Fatalf("got %s %d %s", host, port, service)
			}
		})
	}
}

func TestOptionalEqPredicate(t *testing.T) {
	t.Parallel()

	sql, arg := optionalEqPredicate("owner", 1, "ebank")
	if sql != "owner = NVL(:1, owner)" {
		t.Fatalf("unexpected predicate: %s", sql)
	}
	if arg != "EBANK" {
		t.Fatalf("unexpected arg: %#v", arg)
	}
}

func TestOptionalLikePredicate(t *testing.T) {
	t.Parallel()

	sql, arg := optionalLikePredicate("object_name", 2, "acct")
	if sql != "object_name LIKE NVL(:2, object_name)" {
		t.Fatalf("unexpected predicate: %s", sql)
	}
	if arg != "%ACCT%" {
		t.Fatalf("unexpected arg: %#v", arg)
	}
}
