package sqlite

import (
	"context"
	"fmt"
	"strings"

	"db/internal/database"
)

func (c *connection) ListSchemas(ctx context.Context, filter string, limit int) ([]database.SchemaInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	names, err := c.schemaNames(ctx)
	if err != nil {
		return nil, err
	}
	filter = strings.TrimSpace(strings.ToLower(filter))

	results := make([]database.SchemaInfo, 0, len(names))
	for _, name := range names {
		if filter != "" && !strings.Contains(strings.ToLower(name), filter) {
			continue
		}
		results = append(results, database.SchemaInfo{Name: name})
		if len(results) >= limit {
			break
		}
	}
	return results, nil
}

func (c *connection) ListTables(ctx context.Context, schema, filter string, limit int) ([]database.TableInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	if strings.TrimSpace(schema) == "" {
		schema = c.defaultSchema
	}
	schema, err := c.validateSchema(ctx, schema)
	if err != nil {
		return nil, err
	}

	query := fmt.Sprintf(`
SELECT
  %s AS table_schema,
  name AS table_name,
  CASE type WHEN 'table' THEN 'BASE TABLE' WHEN 'view' THEN 'VIEW' ELSE UPPER(type) END AS table_type
FROM %s.sqlite_schema
WHERE type IN ('table', 'view')
  AND name NOT LIKE 'sqlite_%%'
  AND name LIKE COALESCE(NULLIF(?1, ''), name) COLLATE NOCASE
ORDER BY table_name
LIMIT ?2`, quoteSQLiteString(schema), quoteSQLiteIdentifier(schema))

	rows, cleanup, err := c.query(ctx, query, likeFilter(filter), limit)
	if err != nil {
		return nil, fmt.Errorf("list sqlite tables for schema %q: %w", schema, err)
	}
	defer cleanup()

	var results []database.TableInfo
	for rows.Next() {
		var item database.TableInfo
		if err := rows.Scan(&item.Schema, &item.Name, &item.Type); err != nil {
			return nil, fmt.Errorf("scan sqlite table row: %w", err)
		}
		results = append(results, item)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate sqlite tables: %w", err)
	}
	return results, nil
}

func (c *connection) SearchColumns(ctx context.Context, schema, filter string, limit int) ([]database.ColumnMatchInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	if strings.TrimSpace(schema) == "" {
		schema = c.defaultSchema
	}
	schema, err := c.validateSchema(ctx, schema)
	if err != nil {
		return nil, err
	}

	tableNames, err := c.listTableNames(ctx, schema, "")
	if err != nil {
		return nil, err
	}

	columnFilter := strings.TrimSpace(strings.ToLower(filter))
	results := make([]database.ColumnMatchInfo, 0, limit)
	for _, tableName := range tableNames {
		columns, err := c.DescribeTable(ctx, schema, tableName)
		if err != nil {
			return nil, fmt.Errorf("describe sqlite table %s.%s for column search: %w", schema, tableName, err)
		}
		for _, column := range columns {
			if columnFilter != "" && !strings.Contains(strings.ToLower(column.Name), columnFilter) {
				continue
			}
			results = append(results, database.ColumnMatchInfo{
				Schema:   schema,
				Table:    tableName,
				Column:   column.Name,
				DataType: column.DataType,
				Position: column.Position,
			})
			if len(results) >= limit {
				return results, nil
			}
		}
	}
	return results, nil
}
