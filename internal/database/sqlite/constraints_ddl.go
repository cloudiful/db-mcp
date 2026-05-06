package sqlite

import (
	"context"
	"database/sql"
	"fmt"
	"strings"

	"db/internal/database"
)

func (c *connection) ListConstraints(ctx context.Context, schema, table string, limit int) ([]database.ConstraintInfo, error) {
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

	results := make([]database.ConstraintInfo, 0, limit)
	for _, tableName := range tableNames {
		var pkColumns []string
		query := fmt.Sprintf(`PRAGMA %s.table_info(%s)`, quoteSQLiteIdentifier(schema), quoteSQLiteString(tableName))
		rows, cleanup, err := c.query(ctx, query)
		if err != nil {
			return nil, err
		}
		for rows.Next() {
			var cid, notNull, pk int
			var name, dataType string
			var defaultVal any
			if err := rows.Scan(&cid, &name, &dataType, &notNull, &defaultVal, &pk); err != nil {
				cleanup()
				return nil, err
			}
			_ = cid
			_ = dataType
			_ = notNull
			_ = defaultVal
			if pk > 0 {
				pkColumns = append(pkColumns, name)
			}
		}
		if err := cleanup(); err != nil {
			return nil, err
		}
		if len(pkColumns) > 0 {
			results = append(results, database.ConstraintInfo{
				Name:    fmt.Sprintf("pk_%s", tableName),
				Schema:  schema,
				Table:   tableName,
				Type:    "PRIMARY KEY",
				Columns: pkColumns,
			})
			if len(results) >= limit {
				return results, nil
			}
		}

		indexes, err := c.ListIndexes(ctx, schema, tableName, c.limits.HardMaxRows)
		if err != nil {
			return nil, err
		}
		for _, idx := range indexes {
			if idx.Primary || !idx.Unique {
				continue
			}
			results = append(results, database.ConstraintInfo{
				Name:    idx.Name,
				Schema:  schema,
				Table:   tableName,
				Type:    "UNIQUE",
				Columns: idx.Columns,
			})
			if len(results) >= limit {
				return results, nil
			}
		}

		sqlText, objectType, err := c.readObjectSQL(ctx, schema, tableName)
		if err != nil {
			return nil, err
		}
		if objectType == "table" {
			for i, expr := range extractSQLiteChecks(sqlText) {
				results = append(results, database.ConstraintInfo{
					Name:            fmt.Sprintf("chk_%s_%d", tableName, i+1),
					Schema:          schema,
					Table:           tableName,
					Type:            "CHECK",
					CheckExpression: expr,
				})
				if len(results) >= limit {
					return results, nil
				}
			}
		}

		fkRows, err := c.sqliteForeignKeys(ctx, schema, tableName, c.limits.HardMaxRows)
		if err != nil {
			return nil, err
		}
		fkByName := map[string]*database.ConstraintInfo{}
		for _, fk := range fkRows {
			item := fkByName[fk.ConstraintName]
			if item == nil {
				results = append(results, database.ConstraintInfo{
					Name:             fk.ConstraintName,
					Schema:           fk.Schema,
					Table:            fk.Table,
					Type:             "FOREIGN KEY",
					ReferencedSchema: fk.ReferencedSchema,
					ReferencedTable:  fk.ReferencedTable,
				})
				item = &results[len(results)-1]
				fkByName[fk.ConstraintName] = item
			}
			item.Columns = append(item.Columns, fk.Column)
			if fk.ReferencedColumn != "" {
				item.ReferencedColumns = append(item.ReferencedColumns, fk.ReferencedColumn)
			}
			if len(results) >= limit {
				return results, nil
			}
		}
	}
	return results, nil
}

func (c *connection) sqliteForeignKeys(ctx context.Context, schema, table string, limit int) ([]database.ForeignKeyInfo, error) {
	query := fmt.Sprintf(`PRAGMA %s.foreign_key_list(%s)`,
		quoteSQLiteIdentifier(schema),
		quoteSQLiteString(table),
	)
	rows, cleanup, err := c.query(ctx, query)
	if err != nil {
		return nil, fmt.Errorf("list sqlite foreign keys for %s.%s: %w", schema, table, err)
	}
	defer cleanup()

	results := make([]database.ForeignKeyInfo, 0, limit)
	for rows.Next() {
		var (
			id              int
			seq             int
			referencedTable string
			column          string
			referencedCol   sql.NullString
			updateRule      string
			deleteRule      string
			matchRule       string
		)
		if err := rows.Scan(&id, &seq, &referencedTable, &column, &referencedCol, &updateRule, &deleteRule, &matchRule); err != nil {
			return nil, fmt.Errorf("scan sqlite foreign key row: %w", err)
		}
		_ = matchRule
		item := database.ForeignKeyInfo{
			ConstraintName:   fmt.Sprintf("fk_%s_%d", table, id),
			Schema:           schema,
			Table:            table,
			Column:           column,
			Position:         seq + 1,
			ReferencedSchema: schema,
			ReferencedTable:  referencedTable,
			UpdateRule:       strings.ToUpper(updateRule),
			DeleteRule:       strings.ToUpper(deleteRule),
		}
		if referencedCol.Valid {
			item.ReferencedColumn = referencedCol.String
		}
		results = append(results, item)
		if len(results) >= limit {
			return results, nil
		}
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate sqlite foreign keys for %s.%s: %w", schema, table, err)
	}
	return results, nil
}

func (c *connection) GetTableDDL(ctx context.Context, schema, table string) (*database.TableDDLInfo, error) {
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
	sqlText, objectType, err := c.readObjectSQL(ctx, schema, table)
	if err != nil {
		return nil, err
	}
	return &database.TableDDLInfo{
		Schema:     schema,
		Table:      table,
		ObjectType: objectType,
		DDL:        sqlText,
	}, nil
}

func (c *connection) readObjectSQL(ctx context.Context, schema, table string) (string, string, error) {
	query := fmt.Sprintf(`
SELECT type, sql
FROM %s.sqlite_schema
WHERE name = ?1
  AND type IN ('table', 'view')
LIMIT 1`, quoteSQLiteIdentifier(schema))
	rows, cleanup, err := c.query(ctx, query, table)
	if err != nil {
		return "", "", fmt.Errorf("read sqlite ddl for %s.%s: %w", schema, table, err)
	}
	defer cleanup()

	if !rows.Next() {
		if err := rows.Err(); err != nil {
			return "", "", err
		}
		return "", "", fmt.Errorf("table %s.%s not found or not visible", schema, table)
	}
	var objectType, sqlText string
	if err := rows.Scan(&objectType, &sqlText); err != nil {
		return "", "", err
	}
	return sqlText, objectType, nil
}

func extractSQLiteChecks(sqlText string) []string {
	upper := strings.ToUpper(sqlText)
	var checks []string
	searchFrom := 0
	for {
		idx := strings.Index(upper[searchFrom:], "CHECK(")
		if idx == -1 {
			return checks
		}
		start := searchFrom + idx + len("CHECK(")
		depth := 1
		end := start
		for end < len(sqlText) && depth > 0 {
			switch sqlText[end] {
			case '(':
				depth++
			case ')':
				depth--
			}
			end++
		}
		if depth == 0 {
			checks = append(checks, strings.TrimSpace(sqlText[start:end-1]))
		}
		searchFrom = end
	}
}
