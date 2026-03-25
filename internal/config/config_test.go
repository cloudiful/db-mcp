package config

import "testing"

func TestValidateAssignsDefaultConnectionName(t *testing.T) {
	t.Parallel()

	cfg := &Config{
		Server: ServerConfig{},
		Limits: LimitsConfig{},
		Connections: []ConnectionConfig{{
			Driver: "oracle",
			Oracle: &OracleConfig{
				JDBCURL:  "jdbc:oracle:thin:@localhost:1521:XE",
				Username: "readuser",
				Password: "readuser",
			},
		}},
	}

	if err := cfg.Validate(); err != nil {
		t.Fatalf("validate config: %v", err)
	}
	if cfg.Connections[0].Name != "oracle" {
		t.Fatalf("expected default connection name oracle, got %q", cfg.Connections[0].Name)
	}
}
