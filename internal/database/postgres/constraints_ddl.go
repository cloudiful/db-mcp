package postgres

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
		schema = c.schema()
	}

	rows, cleanup, err := c.query(ctx, `
SELECT
  tc.constraint_name,
  tc.table_schema,
  tc.table_name,
  tc.constraint_type,
  COALESCE(string_agg(kcu.column_name, ',' ORDER BY kcu.ordinal_position) FILTER (WHERE kcu.column_name IS NOT NULL), '') AS columns_csv,
  COALESCE(ccu.table_schema, '') AS referenced_schema,
  COALESCE(ccu.table_name, '') AS referenced_table,
  COALESCE(string_agg(ccu.column_name, ',' ORDER BY kcu.ordinal_position) FILTER (WHERE ccu.column_name IS NOT NULL), '') AS referenced_columns_csv,
  COALESCE(chk.check_clause, '') AS check_clause
FROM information_schema.table_constraints tc
LEFT JOIN information_schema.key_column_usage kcu
  ON kcu.constraint_schema = tc.constraint_schema
 AND kcu.constraint_name = tc.constraint_name
LEFT JOIN information_schema.referential_constraints rc
  ON rc.constraint_schema = tc.constraint_schema
 AND rc.constraint_name = tc.constraint_name
LEFT JOIN information_schema.constraint_column_usage ccu
  ON ccu.constraint_schema = rc.unique_constraint_schema
 AND ccu.constraint_name = rc.unique_constraint_name
LEFT JOIN information_schema.check_constraints chk
  ON chk.constraint_schema = tc.constraint_schema
 AND chk.constraint_name = tc.constraint_name
WHERE tc.table_schema = COALESCE(NULLIF($1, ''), tc.table_schema)
  AND tc.table_name = COALESCE(NULLIF($2, ''), tc.table_name)
  AND tc.constraint_type IN ('PRIMARY KEY', 'UNIQUE', 'CHECK', 'FOREIGN KEY')
GROUP BY tc.constraint_name, tc.table_schema, tc.table_name, tc.constraint_type, referenced_schema, referenced_table, check_clause
ORDER BY tc.table_schema, tc.table_name, tc.constraint_name
LIMIT $3`, exactFilter(schema), exactFilter(table), limit)
	if err != nil {
		return nil, fmt.Errorf("list postgres constraints for schema %q table %q: %w", schema, table, err)
	}
	defer cleanup()

	var results []database.ConstraintInfo
	for rows.Next() {
		var (
			item               database.ConstraintInfo
			columnsCSV         string
			referencedColsCSV  string
			checkClause        string
		)
		if err := rows.Scan(
			&item.Name,
			&item.Schema,
			&item.Table,
			&item.Type,
			&columnsCSV,
			&item.ReferencedSchema,
			&item.ReferencedTable,
			&referencedColsCSV,
			&checkClause,
		); err != nil {
			return nil, fmt.Errorf("scan postgres constraint row: %w", err)
		}
		item.Columns = splitCSV(columnsCSV)
		item.ReferencedColumns = splitCSV(referencedColsCSV)
		if item.Type == "CHECK" {
			item.CheckExpression = checkClause
		}
		results = append(results, item)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate postgres constraints: %w", err)
	}
	return results, nil
}

func (c *connection) GetTableDDL(ctx context.Context, schema, table string) (*database.TableDDLInfo, error) {
	if strings.TrimSpace(table) == "" {
		return nil, fmt.Errorf("table is required")
	}
	if strings.TrimSpace(schema) == "" {
		schema = c.schema()
	}

	var objectType string
	err := c.db.QueryRowContext(ctx, `
SELECT CASE table_type WHEN 'VIEW' THEN 'view' ELSE 'table' END
FROM information_schema.tables
WHERE table_schema = $1 AND table_name = $2
LIMIT 1`, schema, table).Scan(&objectType)
	if err != nil {
		if err == sql.ErrNoRows {
			return nil, fmt.Errorf("table %s.%s not found or not visible", schema, table)
		}
		return nil, fmt.Errorf("detect postgres table type for %s.%s: %w", schema, table, err)
	}

	if objectType == "view" {
		var definition string
		if err := c.db.QueryRowContext(ctx, `
SELECT pg_get_viewdef(format('%I.%I', $1, $2)::regclass, true)`, schema, table).Scan(&definition); err != nil {
			return nil, fmt.Errorf("get postgres view ddl for %s.%s: %w", schema, table, err)
		}
		return &database.TableDDLInfo{
			Schema:     schema,
			Table:      table,
			ObjectType: objectType,
			DDL:        fmt.Sprintf("CREATE VIEW %s.%s AS\n%s", quoteIdentifier(schema), quoteIdentifier(table), definition),
		}, nil
	}

	rows, cleanup, err := c.query(ctx, `
SELECT
  a.attnum,
  a.attname,
  pg_catalog.format_type(a.atttypid, a.atttypmod),
  a.attnotnull,
  COALESCE(pg_get_expr(ad.adbin, ad.adrelid), ''),
  a.attidentity,
  a.attgenerated
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
JOIN pg_attribute a ON a.attrelid = c.oid
LEFT JOIN pg_attrdef ad ON ad.adrelid = a.attrelid AND ad.adnum = a.attnum
WHERE n.nspname = $1
  AND c.relname = $2
  AND c.relkind IN ('r', 'p', 'f')
  AND a.attnum > 0
  AND NOT a.attisdropped
ORDER BY a.attnum`, schema, table)
	if err != nil {
		return nil, fmt.Errorf("get postgres table ddl for %s.%s: %w", schema, table, err)
	}
	defer cleanup()

	var lines []string
	for rows.Next() {
		var (
			position   int
			name       string
			dataType   string
			notNull    bool
			defaultVal string
			identity   string
			generated  string
		)
		if err := rows.Scan(&position, &name, &dataType, &notNull, &defaultVal, &identity, &generated); err != nil {
			return nil, fmt.Errorf("scan postgres ddl column row: %w", err)
		}
		_ = position
		line := fmt.Sprintf("  %s %s", quoteIdentifier(name), dataType)
		if identity != "" {
			line += " GENERATED BY DEFAULT AS IDENTITY"
		} else if generated != "" {
			line += " GENERATED ALWAYS AS (...) STORED"
		} else if defaultVal != "" {
			line += " DEFAULT " + defaultVal
		}
		if notNull {
			line += " NOT NULL"
		}
		lines = append(lines, line)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate postgres ddl columns: %w", err)
	}
	if len(lines) == 0 {
		return nil, fmt.Errorf("table %s.%s not found or not visible", schema, table)
	}

	return &database.TableDDLInfo{
		Schema:     schema,
		Table:      table,
		ObjectType: objectType,
		DDL: fmt.Sprintf("CREATE TABLE %s.%s (\n%s\n)",
			quoteIdentifier(schema),
			quoteIdentifier(table),
			strings.Join(lines, ",\n"),
		),
	}, nil
}
