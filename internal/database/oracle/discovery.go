package oracle

import (
	"context"
	"fmt"

	"db/internal/database"
)

func (c *connection) ListSchemas(ctx context.Context, filter string, limit int) ([]database.SchemaInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	usernameFilter, usernameArg := optionalLikePredicate("username", 1, filter)
	query := fmt.Sprintf(`
SELECT username
FROM (
  SELECT username
  FROM all_users
  WHERE %s
  ORDER BY username
)
WHERE ROWNUM <= %d`, usernameFilter, limit)

	rows, cleanup, err := c.query(ctx, query, usernameArg)
	if err != nil {
		return nil, fmt.Errorf("list oracle schemas: %w", err)
	}
	defer cleanup()

	var results []database.SchemaInfo
	for rows.Next() {
		var item database.SchemaInfo
		if err := rows.Scan(&item.Name); err != nil {
			return nil, fmt.Errorf("scan oracle schema row: %w", err)
		}
		results = append(results, item)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate oracle schemas: %w", err)
	}
	return results, nil
}

func (c *connection) ListTables(ctx context.Context, schema, filter string, limit int) ([]database.TableInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	schema = c.normalizedSchema(schema)
	ownerFilter, ownerArg := optionalEqPredicate("owner", 1, schema)
	nameFilter, nameArg := optionalLikePredicate("object_name", 2, filter)
	query := fmt.Sprintf(`
SELECT owner, object_name, object_type
FROM (
  SELECT owner, object_name, object_type
  FROM all_objects
  WHERE object_type IN ('TABLE', 'VIEW')
    AND generated = 'N'
    AND %s
    AND %s
  ORDER BY owner, object_type, object_name
)
WHERE ROWNUM <= %d`, ownerFilter, nameFilter, limit)

	rows, cleanup, err := c.query(ctx, query, ownerArg, nameArg)
	if err != nil {
		return nil, fmt.Errorf("list oracle tables for schema %q: %w", schema, err)
	}
	defer cleanup()

	var results []database.TableInfo
	for rows.Next() {
		var item database.TableInfo
		if err := rows.Scan(&item.Schema, &item.Name, &item.Type); err != nil {
			return nil, fmt.Errorf("scan oracle table row: %w", err)
		}
		results = append(results, item)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate oracle tables: %w", err)
	}
	return results, nil
}

func (c *connection) SearchColumns(ctx context.Context, schema, filter string, limit int) ([]database.ColumnMatchInfo, error) {
	limit = c.limits.NormalizeListLimit(limit)
	schema = c.normalizedSchema(schema)
	ownerFilter, ownerArg := optionalEqPredicate("owner", 1, schema)
	columnFilter, columnArg := optionalLikePredicate("column_name", 2, filter)
	query := fmt.Sprintf(`
SELECT owner, table_name, column_name, data_type, column_id
FROM (
  SELECT owner, table_name, column_name, data_type, column_id
  FROM all_tab_columns
  WHERE %s
    AND %s
  ORDER BY owner, table_name, column_id
)
WHERE ROWNUM <= %d`, ownerFilter, columnFilter, limit)

	rows, cleanup, err := c.query(ctx, query, ownerArg, columnArg)
	if err != nil {
		return nil, fmt.Errorf("search oracle columns for schema %q: %w", schema, err)
	}
	defer cleanup()

	var results []database.ColumnMatchInfo
	for rows.Next() {
		var item database.ColumnMatchInfo
		if err := rows.Scan(&item.Schema, &item.Table, &item.Column, &item.DataType, &item.Position); err != nil {
			return nil, fmt.Errorf("scan oracle column match row: %w", err)
		}
		results = append(results, item)
	}
	if err := rows.Err(); err != nil {
		return nil, fmt.Errorf("iterate oracle column matches: %w", err)
	}
	return results, nil
}
