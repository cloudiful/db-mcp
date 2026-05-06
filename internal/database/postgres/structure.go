package postgres

import (
	"context"
	"database/sql"
	"fmt"
	"strings"

	"db/internal/database"
)

func (c *connection) ListIndexes(ctx context.Context, schema, table string, limit int) ([]database.IndexInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	if strings.TrimSpace(schema) == "" {
		schema = c.schema()
	}

	rows, cleanup, err := c.query(ctx, `
SELECT
  ns.nspname,
  tbl.relname,
  idx.relname,
  am.amname,
  i.indisunique,
  i.indisprimary,
  (i.indpred IS NOT NULL) AS is_partial,
  COALESCE(string_agg(att.attname, ',' ORDER BY ord.ordinality), '') AS columns_csv,
  pg_get_indexdef(i.indexrelid)
FROM pg_class tbl
JOIN pg_namespace ns ON ns.oid = tbl.relnamespace
JOIN pg_index i ON i.indrelid = tbl.oid
JOIN pg_class idx ON idx.oid = i.indexrelid
JOIN pg_am am ON am.oid = idx.relam
LEFT JOIN LATERAL unnest(i.indkey) WITH ORDINALITY AS ord(attnum, ordinality) ON TRUE
LEFT JOIN pg_attribute att ON att.attrelid = tbl.oid AND att.attnum = ord.attnum
WHERE ns.nspname = COALESCE(NULLIF($1, ''), ns.nspname)
  AND tbl.relname = COALESCE(NULLIF($2, ''), tbl.relname)
GROUP BY ns.nspname, tbl.relname, idx.relname, am.amname, i.indisunique, i.indisprimary, is_partial, i.indexrelid
ORDER BY ns.nspname, tbl.relname, idx.relname
LIMIT $3`, exactFilter(schema), exactFilter(table), limit)
	if err != nil {
		return nil, fmt.Errorf("list postgres indexes for schema %q table %q: %w", schema, table, err)
	}
	defer cleanup()

	var results []database.IndexInfo
	for rows.Next() {
		var (
			item       database.IndexInfo
			columnsCSV string
		)
		if err := rows.Scan(
			&item.Schema,
			&item.Table,
			&item.Name,
			&item.Type,
			&item.Unique,
			&item.Primary,
			&item.Partial,
			&columnsCSV,
			&item.Definition,
		); err != nil {
			return nil, fmt.Errorf("scan postgres index row: %w", err)
		}
		item.Columns = splitCSV(columnsCSV)
		results = append(results, item)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate postgres indexes: %w", err)
	}
	return results, nil
}

func (c *connection) GetTableComments(ctx context.Context, schema, table string) (*database.TableCommentInfo, error) {
	if strings.TrimSpace(table) == "" {
		return nil, fmt.Errorf("table is required")
	}
	if strings.TrimSpace(schema) == "" {
		schema = c.schema()
	}

	rows, cleanup, err := c.query(ctx, `
SELECT COALESCE(obj_description(c.oid, 'pg_class'), '')
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE n.nspname = $1
  AND c.relname = $2
  AND c.relkind IN ('r', 'v', 'm', 'p', 'f')`, schema, table)
	if err != nil {
		return nil, fmt.Errorf("get postgres table comment for %s.%s: %w", schema, table, err)
	}
	var result database.TableCommentInfo
	result.Schema = schema
	result.Table = table
	if !rows.Next() {
		cleanup()
		if err := rows.Err(); err != nil {
			return nil, fmt.Errorf("iterate postgres table comment: %w", err)
		}
		return nil, fmt.Errorf("table %s.%s not found or not visible", schema, table)
	}
	if err := rows.Scan(&result.TableComment); err != nil {
		cleanup()
		return nil, fmt.Errorf("scan postgres table comment: %w", err)
	}
	if err := cleanup(); err != nil {
		return nil, err
	}

	rows, cleanup, err = c.query(ctx, `
SELECT
  a.attname,
  COALESCE(col_description(c.oid, a.attnum), '')
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
JOIN pg_attribute a ON a.attrelid = c.oid
WHERE n.nspname = $1
  AND c.relname = $2
  AND a.attnum > 0
  AND NOT a.attisdropped
ORDER BY a.attnum`, schema, table)
	if err != nil {
		return nil, fmt.Errorf("get postgres column comments for %s.%s: %w", schema, table, err)
	}
	defer cleanup()

	for rows.Next() {
		var item database.ColumnCommentInfo
		if err := rows.Scan(&item.Name, &item.Comment); err != nil {
			return nil, fmt.Errorf("scan postgres column comment row: %w", err)
		}
		result.Columns = append(result.Columns, item)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate postgres column comments: %w", err)
	}
	return &result, nil
}

func (c *connection) DescribeTable(ctx context.Context, schema, table string) ([]database.ColumnInfo, error) {
	if strings.TrimSpace(table) == "" {
		return nil, fmt.Errorf("table is required")
	}
	if strings.TrimSpace(schema) == "" {
		schema = c.schema()
	}

	rows, cleanup, err := c.query(ctx, `
SELECT
  ordinal_position,
  column_name,
  data_type,
  character_maximum_length,
  numeric_precision,
  numeric_scale,
  is_nullable
FROM information_schema.columns
WHERE table_schema = $1
  AND table_name = $2
ORDER BY ordinal_position`, schema, table)
	if err != nil {
		return nil, fmt.Errorf("describe postgres table %s.%s: %w", schema, table, err)
	}
	defer cleanup()

	var results []database.ColumnInfo
	for rows.Next() {
		var (
			item      database.ColumnInfo
			length    sql.NullInt64
			precision sql.NullInt64
			scale     sql.NullInt64
			nullable  string
		)
		if err := rows.Scan(&item.Position, &item.Name, &item.DataType, &length, &precision, &scale, &nullable); err != nil {
			return nil, fmt.Errorf("scan postgres column row: %w", err)
		}
		item.Nullable = strings.EqualFold(nullable, "YES")
		item.Length = toPtr(length)
		item.Precision = toPtr(precision)
		item.Scale = toPtr(scale)
		results = append(results, item)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate postgres columns: %w", err)
	}
	if len(results) == 0 {
		return nil, fmt.Errorf("table %s.%s not found or not visible", schema, table)
	}
	return results, nil
}

func splitCSV(value string) []string {
	if value == "" {
		return nil
	}
	parts := strings.Split(value, ",")
	result := make([]string, 0, len(parts))
	for _, part := range parts {
		part = strings.TrimSpace(part)
		if part != "" {
			result = append(result, part)
		}
	}
	return result
}

func toPtr(value sql.NullInt64) *int64 {
	if !value.Valid {
		return nil
	}
	v := value.Int64
	return &v
}
