package sqlite

import (
	"context"
	"strings"
	"testing"
	"time"

	"db/internal/config"
	"db/internal/database"
)

func TestSQLiteConnectionMetadata(t *testing.T) {
	t.Parallel()

	conn, err := openConnection(context.Background(), config.ConnectionConfig{
		Name:   "sqlite-test",
		Driver: "sqlite",
		SQLite: &config.SQLiteConfig{
			DSN: "file:test-sqlite-meta?mode=memory&cache=shared",
		},
	}, database.Limits{
		DefaultMaxRows: 10,
		HardMaxRows:    20,
		MaxColumns:     20,
		MaxCellChars:   256,
		MaxTotalChars:  2048,
		QueryTimeout:   5 * time.Second,
	})
	if err != nil {
		t.Fatalf("open sqlite connection: %v", err)
	}
	defer conn.Close()

	if _, err := conn.ExecuteQuery(context.Background(), `SELECT 1`, 5); err != nil {
		t.Fatalf("execute query: %v", err)
	}

	sqliteConn := conn.(*connection)
	var queryOnly int
	if err := sqliteConn.db.QueryRow(`PRAGMA query_only`).Scan(&queryOnly); err != nil {
		t.Fatalf("read query_only pragma: %v", err)
	}
	if queryOnly != 1 {
		t.Fatalf("expected query_only pragma enabled, got %d", queryOnly)
	}
}

func TestSQLiteHelpers(t *testing.T) {
	t.Parallel()

	if got := quoteSQLiteIdentifier(`ma"in`); got != `"ma""in"` {
		t.Fatalf("unexpected quoted identifier: %s", got)
	}
	if got := quoteSQLiteString(`o'hare`); got != `'o''hare'` {
		t.Fatalf("unexpected quoted string: %s", got)
	}

	length, precision, scale := parseSQLiteType("VARCHAR(128)")
	if length == nil || *length != 128 || precision != nil || scale != nil {
		t.Fatalf("unexpected varchar parse result")
	}

	length, precision, scale = parseSQLiteType("DECIMAL(10,2)")
	if length != nil || precision == nil || scale == nil || *precision != 10 || *scale != 2 {
		t.Fatalf("unexpected decimal parse result")
	}
}

func TestSQLiteExecuteQueryHonorsFirstRowTotalCharLimit(t *testing.T) {
	t.Parallel()

	conn, err := openConnection(context.Background(), config.ConnectionConfig{
		Name:   "sqlite-limit-test",
		Driver: "sqlite",
		SQLite: &config.SQLiteConfig{
			DSN: "file:test-sqlite-limit?mode=memory&cache=shared",
		},
	}, database.Limits{
		DefaultMaxRows: 10,
		HardMaxRows:    20,
		MaxColumns:     20,
		MaxCellChars:   256,
		MaxTotalChars:  5,
		QueryTimeout:   5 * time.Second,
	})
	if err != nil {
		t.Fatalf("open sqlite connection: %v", err)
	}
	defer conn.Close()

	result, err := conn.ExecuteQuery(context.Background(), `SELECT 'abcdefghij' AS value`, 5)
	if err != nil {
		t.Fatalf("execute query: %v", err)
	}
	if result.RowCount != 0 {
		t.Fatalf("expected no rows after total char truncation, got %d", result.RowCount)
	}
	if !result.Truncated {
		t.Fatalf("expected truncated result")
	}
	if len(result.TruncationReasons) != 1 || result.TruncationReasons[0] != "total output limit reached" {
		t.Fatalf("unexpected truncation reasons: %#v", result.TruncationReasons)
	}
}

func TestSQLiteQueryContextStaysAliveUntilRowsClosed(t *testing.T) {
	t.Parallel()

	conn, err := openConnection(context.Background(), config.ConnectionConfig{
		Name:   "sqlite-query-context-test",
		Driver: "sqlite",
		SQLite: &config.SQLiteConfig{
			DSN: "file:test-sqlite-query-context?mode=memory&cache=shared",
		},
	}, database.Limits{
		DefaultMaxRows: 10,
		HardMaxRows:    20,
		MaxColumns:     20,
		MaxCellChars:   256,
		MaxTotalChars:  2048,
		QueryTimeout:   5 * time.Second,
	})
	if err != nil {
		t.Fatalf("open sqlite connection: %v", err)
	}
	defer conn.Close()

	sqliteConn := conn.(*connection)
	rows, cleanup, err := sqliteConn.query(context.Background(), `SELECT 'ok'`)
	if err != nil {
		t.Fatalf("query: %v", err)
	}
	defer cleanup()

	if !rows.Next() {
		t.Fatalf("expected one row, got err=%v", rows.Err())
	}
	var value string
	if err := rows.Scan(&value); err != nil {
		t.Fatalf("scan row: %v", err)
	}
	if value != "ok" {
		t.Fatalf("unexpected value %q", value)
	}

	if err := cleanup(); err != nil && !strings.Contains(err.Error(), "context canceled") {
		t.Fatalf("expected cleanup to succeed, got %v", err)
	}
}
