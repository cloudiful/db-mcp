package database

import "testing"

func TestNormalizeReadOnlyQuery(t *testing.T) {
	t.Parallel()

	tests := []struct {
		name    string
		query   string
		wantErr error
	}{
		{name: "select", query: "SELECT * FROM dual", wantErr: nil},
		{name: "with", query: "WITH x AS (SELECT 1 FROM dual) SELECT * FROM x", wantErr: nil},
		{name: "insert", query: "INSERT INTO t VALUES (1)", wantErr: ErrWriteQuery},
		{name: "for update", query: "SELECT * FROM dual FOR UPDATE", wantErr: ErrRestrictedClause},
		{name: "multiple", query: "SELECT * FROM dual; SELECT * FROM dual", wantErr: ErrMultipleQueries},
	}

	for _, tt := range tests {
		tt := tt
		t.Run(tt.name, func(t *testing.T) {
			t.Parallel()
			_, err := NormalizeReadOnlyQuery(tt.query)
			if tt.wantErr == nil && err != nil {
				t.Fatalf("unexpected error: %v", err)
			}
			if tt.wantErr != nil && err != tt.wantErr {
				t.Fatalf("expected %v, got %v", tt.wantErr, err)
			}
		})
	}
}
