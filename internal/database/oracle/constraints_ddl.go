package oracle

import (
	"context"
	"database/sql"
	"fmt"
	"strings"

	"db/internal/database"
)

func (c *connection) ListConstraints(ctx context.Context, schema, table string, limit int) ([]database.ConstraintInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	schema = c.normalizedSchema(schema)
	table = normalizedOptionalName(table)
	ownerFilter, ownerArg := optionalEqPredicate("ac.owner", 1, schema)
	tableFilter, tableArg := optionalEqPredicate("ac.table_name", 2, table)

	rows, cleanup, err := c.query(ctx, fmt.Sprintf(`
SELECT
  ac.constraint_name,
  ac.owner,
  ac.table_name,
  ac.constraint_type,
  acc.column_name,
  acc.position,
  ref.owner,
  ref.table_name,
  refc.column_name,
  ac.search_condition
FROM all_constraints ac
LEFT JOIN all_cons_columns acc
  ON acc.owner = ac.owner
 AND acc.constraint_name = ac.constraint_name
LEFT JOIN all_constraints ref
  ON ref.owner = ac.r_owner
 AND ref.constraint_name = ac.r_constraint_name
LEFT JOIN all_cons_columns refc
  ON refc.owner = ref.owner
 AND refc.constraint_name = ref.constraint_name
 AND refc.position = acc.position
WHERE %s
  AND %s
  AND ac.constraint_type IN ('P', 'U', 'C', 'R')
ORDER BY ac.owner, ac.table_name, ac.constraint_name, acc.position`, ownerFilter, tableFilter), ownerArg, tableArg)
	if err != nil {
		return nil, fmt.Errorf("list oracle constraints for schema %q table %q: %w", schema, table, err)
	}
	defer cleanup()

	var (
		results []database.ConstraintInfo
		current *database.ConstraintInfo
		lastKey string
	)
	for rows.Next() {
		var (
			name, owner, tableName, constraintType string
			columnName, refOwner, refTable, refColumn sql.NullString
			position sql.NullInt64
			checkExpr sql.NullString
		)
		if err := rows.Scan(&name, &owner, &tableName, &constraintType, &columnName, &position, &refOwner, &refTable, &refColumn, &checkExpr); err != nil {
			return nil, fmt.Errorf("scan oracle constraint row: %w", err)
		}
		key := owner + "." + tableName + "." + name
		if key != lastKey {
			if len(results) >= limit {
				return results, nil
			}
			results = append(results, database.ConstraintInfo{
				Name:   name,
				Schema: owner,
				Table:  tableName,
				Type:   oracleConstraintType(constraintType),
			})
			current = &results[len(results)-1]
			if current.Type == "CHECK" {
				current.CheckExpression = nullStringValue(checkExpr)
			}
			lastKey = key
		}
		if current == nil {
			continue
		}
		if value := nullStringValue(columnName); value != "" {
			current.Columns = append(current.Columns, value)
		}
		if current.Type == "FOREIGN KEY" {
			current.ReferencedSchema = nullStringValue(refOwner)
			current.ReferencedTable = nullStringValue(refTable)
			if value := nullStringValue(refColumn); value != "" {
				current.ReferencedColumns = append(current.ReferencedColumns, value)
			}
		}
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate oracle constraints: %w", err)
	}
	return results, nil
}

func (c *connection) GetTableDDL(ctx context.Context, schema, table string) (*database.TableDDLInfo, error) {
	var err error
	schema, table, err = c.normalizedSchemaTable(schema, table)
	if err != nil {
		return nil, err
	}

	viewText, err := c.queryNullableString(ctx, `
SELECT text
FROM all_views
WHERE owner = :1 AND view_name = :2`, schema, table)
	if err == nil {
		return &database.TableDDLInfo{
			Schema:     schema,
			Table:      table,
			ObjectType: "view",
			DDL:        fmt.Sprintf("CREATE OR REPLACE VIEW %s.%s AS\n%s", schema, table, strings.TrimSpace(viewText)),
		}, nil
	}
	if err != nil && !strings.Contains(err.Error(), errOracleRowNotFound.Error()) {
		if err != errOracleRowNotFound {
			return nil, fmt.Errorf("get oracle view ddl for %s.%s: %w", schema, table, err)
		}
	}

	columns, err := c.DescribeTable(ctx, schema, table)
	if err != nil {
		return nil, err
	}
	lines := make([]string, 0, len(columns))
	for _, column := range columns {
		line := fmt.Sprintf("  %s %s", column.Name, oracleColumnType(column))
		if !column.Nullable {
			line += " NOT NULL"
		}
		lines = append(lines, line)
	}
	return &database.TableDDLInfo{
		Schema:     schema,
		Table:      table,
		ObjectType: "table",
		DDL:        fmt.Sprintf("CREATE TABLE %s.%s (\n%s\n)", schema, table, strings.Join(lines, ",\n")),
	}, nil
}

func oracleConstraintType(value string) string {
	switch strings.ToUpper(strings.TrimSpace(value)) {
	case "P":
		return "PRIMARY KEY"
	case "U":
		return "UNIQUE"
	case "C":
		return "CHECK"
	case "R":
		return "FOREIGN KEY"
	default:
		return strings.ToUpper(strings.TrimSpace(value))
	}
}

func oracleColumnType(column database.ColumnInfo) string {
	dataType := strings.TrimSpace(column.DataType)
	if column.Length != nil && *column.Length > 0 && !strings.Contains(dataType, "(") {
		switch dataType {
		case "VARCHAR2", "CHAR", "NCHAR", "NVARCHAR2", "RAW":
			return fmt.Sprintf("%s(%d)", dataType, *column.Length)
		}
	}
	if column.Precision != nil {
		if column.Scale != nil {
			return fmt.Sprintf("%s(%d,%d)", dataType, *column.Precision, *column.Scale)
		}
		return fmt.Sprintf("%s(%d)", dataType, *column.Precision)
	}
	return dataType
}
