package postgres

import (
	"context"
	"database/sql"
	"fmt"
	"strings"

	"db/internal/database"
)

func fetchDefaultSchema(ctx context.Context, db *sql.DB) (string, error) {
	var schema string
	if err := db.QueryRowContext(ctx, `SELECT current_schema()`).Scan(&schema); err != nil {
		return "", fmt.Errorf("fetch postgres default schema: %w", err)
	}
	if schema == "" {
		schema = "public"
	}
	return schema, nil
}

func ensureSchemaVisible(ctx context.Context, db *sql.DB, schema string) error {
	var exists bool
	if err := db.QueryRowContext(ctx, `
SELECT EXISTS (
  SELECT 1
  FROM information_schema.schemata
  WHERE schema_name = $1
)`, schema).Scan(&exists); err != nil {
		return fmt.Errorf("check postgres schema %q: %w", schema, err)
	}
	if !exists {
		return fmt.Errorf("postgres schema %q not found or not visible", schema)
	}
	return nil
}

func (c *connection) ListSchemas(ctx context.Context, filter string, limit int) ([]database.SchemaInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	rows, cleanup, err := c.query(ctx, `
SELECT schema_name
FROM information_schema.schemata
WHERE schema_name ILIKE COALESCE(NULLIF($1, ''), schema_name)
ORDER BY schema_name
LIMIT $2`, likeFilter(filter), limit)
	if err != nil {
		return nil, fmt.Errorf("list postgres schemas: %w", err)
	}
	defer cleanup()

	var results []database.SchemaInfo
	for rows.Next() {
		var name string
		if err := rows.Scan(&name); err != nil {
			return nil, fmt.Errorf("scan postgres schema row: %w", err)
		}
		results = append(results, database.SchemaInfo{Name: name})
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate postgres schemas: %w", err)
	}
	return results, nil
}

func (c *connection) ListTables(ctx context.Context, schema, filter string, limit int) ([]database.TableInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	if strings.TrimSpace(schema) == "" {
		schema = c.schema()
	}

	rows, cleanup, err := c.query(ctx, `
SELECT table_schema, table_name, table_type
FROM information_schema.tables
WHERE table_schema = COALESCE(NULLIF($1, ''), table_schema)
  AND table_type IN ('BASE TABLE', 'VIEW')
  AND table_name ILIKE COALESCE(NULLIF($2, ''), table_name)
ORDER BY table_schema, table_type, table_name
LIMIT $3`, exactFilter(schema), likeFilter(filter), limit)
	if err != nil {
		return nil, fmt.Errorf("list postgres tables for schema %q: %w", schema, err)
	}
	defer cleanup()

	var results []database.TableInfo
	for rows.Next() {
		var item database.TableInfo
		if err := rows.Scan(&item.Schema, &item.Name, &item.Type); err != nil {
			return nil, fmt.Errorf("scan postgres table row: %w", err)
		}
		results = append(results, item)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate postgres tables: %w", err)
	}
	return results, nil
}

func (c *connection) SearchColumns(ctx context.Context, schema, filter string, limit int) ([]database.ColumnMatchInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	if strings.TrimSpace(schema) == "" {
		schema = c.schema()
	}

	rows, cleanup, err := c.query(ctx, `
SELECT table_schema, table_name, column_name, data_type, ordinal_position
FROM information_schema.columns
WHERE table_schema = COALESCE(NULLIF($1, ''), table_schema)
  AND column_name ILIKE COALESCE(NULLIF($2, ''), column_name)
ORDER BY table_schema, table_name, ordinal_position
LIMIT $3`, exactFilter(schema), likeFilter(filter), limit)
	if err != nil {
		return nil, fmt.Errorf("search postgres columns for schema %q: %w", schema, err)
	}
	defer cleanup()

	var results []database.ColumnMatchInfo
	for rows.Next() {
		var item database.ColumnMatchInfo
		if err := rows.Scan(&item.Schema, &item.Table, &item.Column, &item.DataType, &item.Position); err != nil {
			return nil, fmt.Errorf("scan postgres column match row: %w", err)
		}
		results = append(results, item)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate postgres column matches: %w", err)
	}
	return results, nil
}

func exactFilter(value string) string {
	value = strings.TrimSpace(value)
	if value == "" {
		return ""
	}
	return value
}

func likeFilter(value string) string {
	value = strings.TrimSpace(value)
	if value == "" {
		return ""
	}
	return "%" + value + "%"
}
