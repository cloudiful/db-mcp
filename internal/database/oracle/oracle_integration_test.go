//go:build integration

package oracle

import (
	"context"
	"os"
	"testing"
	"time"

	"db/internal/config"
	"db/internal/database"
)

func newOracleIntegrationConnection(t *testing.T) (database.Connection, string) {
	t.Helper()

	jdbcURL := os.Getenv("DBMCP_TEST_ORACLE_JDBC_URL")
	username := os.Getenv("DBMCP_TEST_ORACLE_USERNAME")
	password := os.Getenv("DBMCP_TEST_ORACLE_PASSWORD")
	if jdbcURL == "" || username == "" || password == "" {
		t.Skip("integration credentials are not set")
	}

	conn, err := openConnection(context.Background(), config.ConnectionConfig{
		Name:   "oracle-test",
		Driver: "oracle",
		Oracle: &config.OracleConfig{
			JDBCURL:  jdbcURL,
			Username: username,
			Password: password,
		},
	}, database.Limits{
		DefaultMaxRows: 10,
		HardMaxRows:    20,
		MaxColumns:     20,
		MaxCellChars:   256,
		MaxTotalChars:  2048,
		QueryTimeout:   10 * time.Second,
	})
	if err != nil {
		t.Fatalf("open oracle connection: %v", err)
	}
	return conn, username
}

func TestOracleIntegrationListSchemas(t *testing.T) {
	conn, username := newOracleIntegrationConnection(t)
	defer conn.Close()

	schemas, err := conn.ListSchemas(context.Background(), username, 5)
	if err != nil {
		t.Fatalf("list schemas: %v", err)
	}
	if len(schemas) == 0 {
		t.Fatalf("expected at least one schema")
	}
}

func TestOracleIntegrationListTables(t *testing.T) {
	conn, username := newOracleIntegrationConnection(t)
	defer conn.Close()

	schema := os.Getenv("DBMCP_TEST_ORACLE_SCHEMA")
	if schema == "" {
		schema = username
	}

	if _, err := conn.ListTables(context.Background(), schema, "", 5); err != nil {
		t.Fatalf("list tables: %v", err)
	}
}

func TestOracleIntegrationGetTableComments(t *testing.T) {
	conn, username := newOracleIntegrationConnection(t)
	defer conn.Close()

	schema := os.Getenv("DBMCP_TEST_ORACLE_SCHEMA")
	if schema == "" {
		schema = username
	}
	table := os.Getenv("DBMCP_TEST_ORACLE_TABLE")
	if table == "" {
		t.Skip("DBMCP_TEST_ORACLE_TABLE is not set")
	}

	if _, err := conn.GetTableComments(context.Background(), schema, table); err != nil {
		t.Fatalf("get table comments: %v", err)
	}
}
