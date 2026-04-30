package postgres

import (
	"context"
	"database/sql"
	"fmt"
	"strings"
	"time"

	"db/internal/database"
)

func (c *connection) ExecuteQuery(ctx context.Context, query string, maxRows int) (*database.QueryResult, error) {
	normalized, err := database.NormalizeReadOnlyQuery(query)
	if err != nil {
		return nil, err
	}
	appliedMaxRows := c.limits.NormalizeMaxRows(maxRows)
	startedAt := time.Now()
	rows, cleanup, err := c.query(ctx, normalized)
	if err != nil {
		return nil, fmt.Errorf("execute postgres query: %w", err)
	}
	result, err := database.CollectRows(rows, appliedMaxRows, c.limits, startedAt)
	if cleanupErr := cleanup(); err == nil && cleanupErr != nil {
		err = cleanupErr
	}
	return result, err
}

func (c *connection) ExplainQuery(ctx context.Context, query string) (*database.ExplainResult, error) {
	normalized, err := database.NormalizeReadOnlyQuery(query)
	if err != nil {
		return nil, err
	}
	rows, cleanup, err := c.query(ctx, "EXPLAIN (FORMAT TEXT) "+normalized)
	if err != nil {
		return nil, fmt.Errorf("explain postgres query: %w", err)
	}
	defer cleanup()

	var lines []string
	for rows.Next() {
		var line string
		if err := rows.Scan(&line); err != nil {
			return nil, fmt.Errorf("scan postgres explain row: %w", err)
		}
		lines = append(lines, line)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate postgres explain rows: %w", err)
	}
	lines, truncated := database.TruncateLines(lines, c.limits.ExplainMaxLines, c.limits.ExplainMaxTotalChars)
	result := &database.ExplainResult{
		Format:    "text",
		PlanLines: lines,
	}
	if truncated {
		result.Notice = "Explain output was truncated to protect MCP context size."
	}
	return result, nil
}

func (c *connection) query(ctx context.Context, query string, args ...any) (*sql.Rows, func() error, error) {
	queryCtx, cancel := c.limits.WithTimeout(ctx)
	tx, err := c.db.BeginTx(queryCtx, &sql.TxOptions{ReadOnly: true})
	if err != nil {
		cancel()
		return nil, nil, err
	}
	if err := setLocalSearchPath(queryCtx, tx, c.schema()); err != nil {
		_ = tx.Rollback()
		cancel()
		return nil, nil, err
	}
	rows, err := tx.QueryContext(queryCtx, query, args...)
	if err != nil {
		_ = tx.Rollback()
		cancel()
		return nil, nil, err
	}
	return rows, func() error {
		var firstErr error
		if err := rows.Close(); err != nil && firstErr == nil {
			firstErr = err
		}
		if err := tx.Rollback(); err != nil && err != sql.ErrTxDone && firstErr == nil {
			firstErr = err
		}
		cancel()
		return firstErr
	}, nil
}

func setLocalSearchPath(ctx context.Context, tx *sql.Tx, schema string) error {
	schema = strings.TrimSpace(schema)
	if schema == "" {
		return nil
	}

	searchPath := quoteIdentifier(schema)
	if !strings.EqualFold(schema, "public") {
		searchPath += ", public"
	}
	if _, err := tx.ExecContext(ctx, "SET LOCAL search_path TO "+searchPath); err != nil {
		return fmt.Errorf("set postgres search_path: %w", err)
	}
	return nil
}

func quoteIdentifier(value string) string {
	return `"` + strings.ReplaceAll(value, `"`, `""`) + `"`
}
