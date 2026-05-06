package database

import "context"

func (c *managedConnection) Info() ConnectionInfo {
	return c.slot.info()
}

func (c *managedConnection) Ping(ctx context.Context) error {
	if err := c.slot.refresh(ctx); err != nil {
		return unavailableError(c.slot.conn.Info().Name, err)
	}
	return nil
}

func (c *managedConnection) ListSchemas(ctx context.Context, filter string, limit int) ([]SchemaInfo, error) {
	if err := c.slot.ensureReady(ctx); err != nil {
		return nil, err
	}
	result, err := c.slot.conn.ListSchemas(ctx, filter, limit)
	if err != nil {
		return nil, c.slot.handleOperationError(err)
	}
	c.slot.markUp()
	return result, nil
}

func (c *managedConnection) ListTables(ctx context.Context, schema, filter string, limit int) ([]TableInfo, error) {
	if err := c.slot.ensureReady(ctx); err != nil {
		return nil, err
	}
	result, err := c.slot.conn.ListTables(ctx, schema, filter, limit)
	if err != nil {
		return nil, c.slot.handleOperationError(err)
	}
	c.slot.markUp()
	return result, nil
}

func (c *managedConnection) SearchColumns(ctx context.Context, schema, filter string, limit int) ([]ColumnMatchInfo, error) {
	if err := c.slot.ensureReady(ctx); err != nil {
		return nil, err
	}
	result, err := c.slot.conn.SearchColumns(ctx, schema, filter, limit)
	if err != nil {
		return nil, c.slot.handleOperationError(err)
	}
	c.slot.markUp()
	return result, nil
}

func (c *managedConnection) ListConstraints(ctx context.Context, schema, table string, limit int) ([]ConstraintInfo, error) {
	if err := c.slot.ensureReady(ctx); err != nil {
		return nil, err
	}
	result, err := c.slot.conn.ListConstraints(ctx, schema, table, limit)
	if err != nil {
		return nil, c.slot.handleOperationError(err)
	}
	c.slot.markUp()
	return result, nil
}

func (c *managedConnection) ListIndexes(ctx context.Context, schema, table string, limit int) ([]IndexInfo, error) {
	if err := c.slot.ensureReady(ctx); err != nil {
		return nil, err
	}
	result, err := c.slot.conn.ListIndexes(ctx, schema, table, limit)
	if err != nil {
		return nil, c.slot.handleOperationError(err)
	}
	c.slot.markUp()
	return result, nil
}

func (c *managedConnection) GetTableComments(ctx context.Context, schema, table string) (*TableCommentInfo, error) {
	if err := c.slot.ensureReady(ctx); err != nil {
		return nil, err
	}
	result, err := c.slot.conn.GetTableComments(ctx, schema, table)
	if err != nil {
		return nil, c.slot.handleOperationError(err)
	}
	c.slot.markUp()
	return result, nil
}

func (c *managedConnection) DescribeTable(ctx context.Context, schema, table string) ([]ColumnInfo, error) {
	if err := c.slot.ensureReady(ctx); err != nil {
		return nil, err
	}
	result, err := c.slot.conn.DescribeTable(ctx, schema, table)
	if err != nil {
		return nil, c.slot.handleOperationError(err)
	}
	c.slot.markUp()
	return result, nil
}

func (c *managedConnection) GetTableDDL(ctx context.Context, schema, table string) (*TableDDLInfo, error) {
	if err := c.slot.ensureReady(ctx); err != nil {
		return nil, err
	}
	result, err := c.slot.conn.GetTableDDL(ctx, schema, table)
	if err != nil {
		return nil, c.slot.handleOperationError(err)
	}
	c.slot.markUp()
	return result, nil
}

func (c *managedConnection) ExecuteQuery(ctx context.Context, query string, maxRows int) (*QueryResult, error) {
	if err := c.slot.ensureReady(ctx); err != nil {
		return nil, err
	}
	result, err := c.slot.conn.ExecuteQuery(ctx, query, maxRows)
	if err != nil {
		return nil, c.slot.handleOperationError(err)
	}
	c.slot.markUp()
	return result, nil
}

func (c *managedConnection) ExplainQuery(ctx context.Context, query string) (*ExplainResult, error) {
	if err := c.slot.ensureReady(ctx); err != nil {
		return nil, err
	}
	result, err := c.slot.conn.ExplainQuery(ctx, query)
	if err != nil {
		return nil, c.slot.handleOperationError(err)
	}
	c.slot.markUp()
	return result, nil
}

func (c *managedConnection) Close() error {
	return nil
}
