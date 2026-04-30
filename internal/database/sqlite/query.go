package sqlite

import (
	"context"
	"fmt"
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
		return nil, fmt.Errorf("execute sqlite query: %w", err)
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
	rows, cleanup, err := c.query(ctx, "EXPLAIN QUERY PLAN "+normalized)
	if err != nil {
		return nil, fmt.Errorf("explain sqlite query: %w", err)
	}
	defer cleanup()

	var lines []string
	for rows.Next() {
		var (
			id      int
			parent  int
			notused int
			detail  string
		)
		if err := rows.Scan(&id, &parent, &notused, &detail); err != nil {
			return nil, fmt.Errorf("scan sqlite explain row: %w", err)
		}
		_ = id
		_ = parent
		_ = notused
		lines = append(lines, detail)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate sqlite explain rows: %w", err)
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
