package oracle

import (
	"context"
	"database/sql"
	"fmt"
	"strings"

	"db/internal/database"
)

const (
	indexesQuery = `
SELECT ai.table_owner,
       ai.table_name,
       ai.index_name,
       ai.index_type,
       ai.uniqueness,
       CASE WHEN pk.index_name IS NULL THEN 0 ELSE 1 END AS is_primary,
       aic.column_position,
       aic.column_name
FROM all_indexes ai
JOIN all_ind_columns aic
  ON aic.index_owner = ai.owner
 AND aic.index_name = ai.index_name
LEFT JOIN (
  SELECT owner, table_name, index_name
  FROM all_constraints
  WHERE constraint_type = 'P'
) pk
  ON pk.owner = ai.table_owner
 AND pk.table_name = ai.table_name
 AND pk.index_name = ai.index_name
WHERE %s
  AND %s
ORDER BY ai.table_owner, ai.table_name, ai.index_name, aic.column_position`

	describeTableQuery = `
SELECT column_id, column_name, data_type, data_length, data_precision, data_scale, nullable
FROM all_tab_columns
WHERE owner = :1 AND table_name = :2
ORDER BY column_id`
)

func (c *connection) ListIndexes(ctx context.Context, schema, table string, limit int) ([]database.IndexInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	schema = c.normalizedSchema(schema)
	table = normalizedOptionalName(table)
	ownerFilter, ownerArg := optionalEqPredicate("ai.table_owner", 1, schema)
	tableFilter, tableArg := optionalEqPredicate("ai.table_name", 2, table)

	rows, cleanup, err := c.query(ctx, fmt.Sprintf(indexesQuery, ownerFilter, tableFilter), ownerArg, tableArg)
	if err != nil {
		return nil, fmt.Errorf("list oracle indexes for schema %q table %q: %w", schema, table, err)
	}
	defer cleanup()

	var (
		results []database.IndexInfo
		current *database.IndexInfo
		lastKey string
	)
	for rows.Next() {
		var (
			item         database.IndexInfo
			indexTypeVal sql.NullString
			uniqueness   sql.NullString
			isPrimaryNum int
			position     int
			columnName   sql.NullString
		)
		if err := rows.Scan(
			&item.Schema,
			&item.Table,
			&item.Name,
			&indexTypeVal,
			&uniqueness,
			&isPrimaryNum,
			&position,
			&columnName,
		); err != nil {
			return nil, fmt.Errorf("scan oracle index row: %w", err)
		}
		_ = position
		item.Type = nullStringValue(indexTypeVal)
		item.Unique = strings.EqualFold(nullStringValue(uniqueness), "UNIQUE")
		item.Primary = isPrimaryNum == 1
		key := item.Schema + "." + item.Table + "." + item.Name
		if key != lastKey {
			if len(results) >= limit {
				return results, nil
			}
			results = append(results, item)
			current = &results[len(results)-1]
			lastKey = key
		}
		appendIndexColumn(current, columnName)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate oracle indexes: %w", err)
	}
	return results, nil
}

func (c *connection) DescribeTable(ctx context.Context, schema, table string) ([]database.ColumnInfo, error) {
	var err error
	schema, table, err = c.normalizedSchemaTable(schema, table)
	if err != nil {
		return nil, err
	}

	rows, cleanup, err := c.query(ctx, describeTableQuery, schema, table)
	if err != nil {
		return nil, fmt.Errorf("describe oracle table %s.%s: %w", schema, table, err)
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
			return nil, fmt.Errorf("scan oracle column row: %w", err)
		}
		item.Nullable = strings.EqualFold(nullable, "Y")
		item.Length = toPtr(length)
		item.Precision = toPtr(precision)
		item.Scale = toPtr(scale)
		results = append(results, item)
	}
	if len(results) == 0 {
		return nil, fmt.Errorf("table %s.%s not found or not visible", schema, table)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate oracle columns: %w", err)
	}
	return results, nil
}
