package sqlite

import (
	"context"
	"fmt"
	"strings"

	"db/internal/database"
)

func (c *connection) ListIndexes(ctx context.Context, schema, table string, limit int) ([]database.IndexInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	if strings.TrimSpace(schema) == "" {
		schema = c.defaultSchema
	}
	schema, err := c.validateSchema(ctx, schema)
	if err != nil {
		return nil, err
	}

	tableNames, err := c.listTableNames(ctx, schema, table)
	if err != nil {
		return nil, err
	}

	results := make([]database.IndexInfo, 0, limit)
	for _, tableName := range tableNames {
		query := fmt.Sprintf(`PRAGMA %s.index_list(%s)`,
			quoteSQLiteIdentifier(schema),
			quoteSQLiteString(tableName),
		)
		rows, cleanup, err := c.query(ctx, query)
		if err != nil {
			return nil, fmt.Errorf("list sqlite indexes for %s.%s: %w", schema, tableName, err)
		}

		type pendingIndex struct {
			name    string
			unique  bool
			origin  string
			partial bool
		}
		var pending []pendingIndex
		for rows.Next() {
			var (
				seq     int
				name    string
				unique  int
				origin  string
				partial int
			)
			if err := rows.Scan(&seq, &name, &unique, &origin, &partial); err != nil {
				cleanup()
				return nil, fmt.Errorf("scan sqlite index list row: %w", err)
			}
			_ = seq
			pending = append(pending, pendingIndex{
				name:    name,
				unique:  unique == 1,
				origin:  strings.ToLower(origin),
				partial: partial == 1,
			})
		}
		if err := rows.Err(); err != nil {
			cleanup()
			return nil, fmt.Errorf("iterate sqlite index list rows: %w", err)
		}
		if err := cleanup(); err != nil {
			return nil, fmt.Errorf("close sqlite index list rows: %w", err)
		}

		for _, idx := range pending {
			infoRows, infoCleanup, err := c.query(ctx, fmt.Sprintf(`PRAGMA %s.index_info(%s)`,
				quoteSQLiteIdentifier(schema),
				quoteSQLiteString(idx.name),
			))
			if err != nil {
				return nil, fmt.Errorf("describe sqlite index %s.%s.%s: %w", schema, tableName, idx.name, err)
			}

			item := database.IndexInfo{
				Schema:  schema,
				Table:   tableName,
				Name:    idx.name,
				Unique:  idx.unique,
				Primary: idx.origin == "pk",
				Partial: idx.partial,
				Type:    "btree",
			}
			for infoRows.Next() {
				var (
					seqno int
					cid   int
					name  string
				)
				if err := infoRows.Scan(&seqno, &cid, &name); err != nil {
					infoCleanup()
					return nil, fmt.Errorf("scan sqlite index info row: %w", err)
				}
				_ = seqno
				_ = cid
				if name != "" {
					item.Columns = append(item.Columns, name)
				}
			}
			if err := infoRows.Err(); err != nil {
				infoCleanup()
				return nil, fmt.Errorf("iterate sqlite index info rows: %w", err)
			}
			if err := infoCleanup(); err != nil {
				return nil, fmt.Errorf("close sqlite index info rows: %w", err)
			}

			results = append(results, item)
			if len(results) >= limit {
				return results, nil
			}
		}
	}
	return results, nil
}

func (c *connection) GetTableComments(ctx context.Context, schema, table string) (*database.TableCommentInfo, error) {
	if strings.TrimSpace(table) == "" {
		return nil, fmt.Errorf("table is required")
	}
	if strings.TrimSpace(schema) == "" {
		schema = c.defaultSchema
	}
	schema, err := c.validateSchema(ctx, schema)
	if err != nil {
		return nil, err
	}
	if _, err := c.DescribeTable(ctx, schema, table); err != nil {
		return nil, err
	}
	return &database.TableCommentInfo{
		Schema: schema,
		Table:  table,
		Notice: "SQLite does not store table or column comments in system catalogs.",
	}, nil
}

func (c *connection) DescribeTable(ctx context.Context, schema, table string) ([]database.ColumnInfo, error) {
	if strings.TrimSpace(table) == "" {
		return nil, fmt.Errorf("table is required")
	}
	if strings.TrimSpace(schema) == "" {
		schema = c.defaultSchema
	}
	schema, err := c.validateSchema(ctx, schema)
	if err != nil {
		return nil, err
	}

	query := fmt.Sprintf(`
PRAGMA %s.table_info(%s)`,
		quoteSQLiteIdentifier(schema),
		quoteSQLiteString(table),
	)

	rows, cleanup, err := c.query(ctx, query)
	if err != nil {
		return nil, fmt.Errorf("describe sqlite table %s.%s: %w", schema, table, err)
	}
	defer cleanup()

	var results []database.ColumnInfo
	for rows.Next() {
		var (
			cid        int
			item       database.ColumnInfo
			notNull    int
			defaultVal any
			pk         int
		)
		if err := rows.Scan(&cid, &item.Name, &item.DataType, &notNull, &defaultVal, &pk); err != nil {
			return nil, fmt.Errorf("scan sqlite column row: %w", err)
		}
		_ = defaultVal
		_ = pk
		item.Position = cid + 1
		item.Nullable = notNull == 0
		if length, precision, scale := parseSQLiteType(item.DataType); length != nil || precision != nil || scale != nil {
			item.Length = length
			item.Precision = precision
			item.Scale = scale
		}
		results = append(results, item)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate sqlite columns: %w", err)
	}
	if len(results) == 0 {
		return nil, fmt.Errorf("table %s.%s not found or not visible", schema, table)
	}
	return results, nil
}
