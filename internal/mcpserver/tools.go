package mcpserver

import (
	"context"
	"fmt"
	"strings"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"db/internal/database"
)

func (s *service) listConnections(ctx context.Context, _ *mcp.CallToolRequest, input listConnectionsInput) (*mcp.CallToolResult, *listConnectionsOutput, error) {
	refresh := true
	if input.Refresh != nil {
		refresh = *input.Refresh
	}
	if _, err := s.manager.Health(ctx, "", refresh); err != nil {
		return errorResult(err), nil, nil
	}
	output := &listConnectionsOutput{
		DefaultConnection: s.manager.DefaultConnectionName(),
		Connections:       s.manager.ConnectionInfos(),
	}
	return textResult(fmt.Sprintf("Configured %d connection(s). Default is %s.", len(output.Connections), output.DefaultConnection)), output, nil
}

func (s *service) listSchemas(ctx context.Context, _ *mcp.CallToolRequest, input listInput) (*mcp.CallToolResult, *listSchemasOutput, error) {
	conn, err := s.manager.Connection(input.Connection)
	if err != nil {
		return errorResult(err), nil, nil
	}
	schemas, err := conn.ListSchemas(ctx, input.Filter, input.Limit)
	if err != nil {
		return errorResult(err), nil, nil
	}
	output := &listSchemasOutput{
		Connection: conn.Info().Name,
		Schemas:    schemas,
	}
	return textResult(fmt.Sprintf("Returned %d schema(s) from %s.", len(schemas), conn.Info().Name)), output, nil
}

func (s *service) listTables(ctx context.Context, _ *mcp.CallToolRequest, input listInput) (*mcp.CallToolResult, *listTablesOutput, error) {
	conn, err := s.manager.Connection(input.Connection)
	if err != nil {
		return errorResult(err), nil, nil
	}
	tables, err := conn.ListTables(ctx, input.Schema, input.Filter, input.Limit)
	if err != nil {
		return errorResult(err), nil, nil
	}
	output := &listTablesOutput{
		Connection: conn.Info().Name,
		Tables:     tables,
	}
	return textResult(fmt.Sprintf("Returned %d table/view entries from %s.", len(tables), conn.Info().Name)), output, nil
}

func (s *service) searchColumns(ctx context.Context, _ *mcp.CallToolRequest, input searchColumnsInput) (*mcp.CallToolResult, *searchColumnsOutput, error) {
	conn, err := s.manager.Connection(input.Connection)
	if err != nil {
		return errorResult(err), nil, nil
	}
	schema := input.Schema
	if schema == "" {
		schema = conn.Info().DefaultSchema
	}
	matches, err := conn.SearchColumns(ctx, schema, input.Filter, input.Limit)
	if err != nil {
		return errorResult(err), nil, nil
	}
	output := &searchColumnsOutput{
		Connection: conn.Info().Name,
		Matches:    matches,
	}
	return textResult(fmt.Sprintf("Returned %d column match(es) from %s.", len(matches), conn.Info().Name)), output, nil
}

func (s *service) listConstraints(ctx context.Context, _ *mcp.CallToolRequest, input listConstraintsInput) (*mcp.CallToolResult, *listConstraintsOutput, error) {
	conn, err := s.manager.Connection(input.Connection)
	if err != nil {
		return errorResult(err), nil, nil
	}
	schema := input.Schema
	if schema == "" {
		schema = conn.Info().DefaultSchema
	}
	constraints, err := conn.ListConstraints(ctx, schema, input.Table, input.Limit)
	if err != nil {
		return errorResult(err), nil, nil
	}
	constraints, truncated := database.CompactConstraints(constraints, s.manager.Limits().HardMaxRows, s.manager.Limits().MaxTotalChars, s.manager.Limits().MaxCellChars)
	output := &listConstraintsOutput{
		Connection:  conn.Info().Name,
		Constraints: constraints,
	}
	text := fmt.Sprintf("Returned %d constraint row(s) from %s.", len(constraints), conn.Info().Name)
	if truncated {
		output.Notice = database.MetadataNotice()
		text += " " + output.Notice
	}
	return textResult(text), output, nil
}

func (s *service) listIndexes(ctx context.Context, _ *mcp.CallToolRequest, input listIndexesInput) (*mcp.CallToolResult, *listIndexesOutput, error) {
	conn, err := s.manager.Connection(input.Connection)
	if err != nil {
		return errorResult(err), nil, nil
	}
	schema := input.Schema
	if schema == "" {
		schema = conn.Info().DefaultSchema
	}
	indexes, err := conn.ListIndexes(ctx, schema, input.Table, input.Limit)
	if err != nil {
		return errorResult(err), nil, nil
	}
	indexes, truncated := database.CompactIndexes(indexes, s.manager.Limits().HardMaxRows, s.manager.Limits().MaxTotalChars, s.manager.Limits().MaxCellChars)
	output := &listIndexesOutput{
		Connection: conn.Info().Name,
		Indexes:    indexes,
	}
	text := fmt.Sprintf("Returned %d index row(s) from %s.", len(indexes), conn.Info().Name)
	if truncated {
		output.Notice = database.MetadataNotice()
		text += " " + output.Notice
	}
	return textResult(text), output, nil
}

func (s *service) getTableComments(ctx context.Context, _ *mcp.CallToolRequest, input getTableCommentsInput) (*mcp.CallToolResult, *getTableCommentsOutput, error) {
	conn, err := s.manager.Connection(input.Connection)
	if err != nil {
		return errorResult(err), nil, nil
	}
	schema := input.Schema
	if schema == "" {
		schema = conn.Info().DefaultSchema
	}
	comments, err := conn.GetTableComments(ctx, schema, input.Table)
	if err != nil {
		return errorResult(err), nil, nil
	}
	comments, truncated := database.CompactTableComments(comments, s.manager.Limits().HardMaxRows, s.manager.Limits().MaxTotalChars, s.manager.Limits().MaxCellChars)
	output := &getTableCommentsOutput{
		Connection: conn.Info().Name,
		Comments:   comments,
	}
	text := fmt.Sprintf("Loaded comments for %s.%s from %s.", comments.Schema, comments.Table, conn.Info().Name)
	if truncated {
		text += " " + database.MetadataNotice()
	}
	return textResult(text), output, nil
}

func (s *service) describeTable(ctx context.Context, _ *mcp.CallToolRequest, input describeTableInput) (*mcp.CallToolResult, *describeTableOutput, error) {
	conn, err := s.manager.Connection(input.Connection)
	if err != nil {
		return errorResult(err), nil, nil
	}
	schema := input.Schema
	if schema == "" {
		schema = conn.Info().DefaultSchema
	}
	columns, err := conn.DescribeTable(ctx, schema, input.Table)
	if err != nil {
		return errorResult(err), nil, nil
	}
	columns, truncated := database.CompactColumns(columns, s.manager.Limits().HardMaxRows, s.manager.Limits().MaxTotalChars)
	output := &describeTableOutput{
		Connection: conn.Info().Name,
		Schema:     schema,
		Table:      input.Table,
		Columns:    columns,
	}
	text := fmt.Sprintf("Described %s.%s with %d column(s).", output.Schema, output.Table, len(columns))
	if truncated {
		output.Notice = database.MetadataNotice()
		text += " " + output.Notice
	}
	return textResult(text), output, nil
}

func (s *service) getTableDDL(ctx context.Context, _ *mcp.CallToolRequest, input getTableDDLInput) (*mcp.CallToolResult, *getTableDDLOutput, error) {
	conn, err := s.manager.Connection(input.Connection)
	if err != nil {
		return errorResult(err), nil, nil
	}
	schema := input.Schema
	if schema == "" {
		schema = conn.Info().DefaultSchema
	}
	ddl, err := conn.GetTableDDL(ctx, schema, input.Table)
	if err != nil {
		return errorResult(err), nil, nil
	}
	ddl, _ = database.CompactTableDDL(ddl, s.manager.Limits().MaxTotalChars)
	output := &getTableDDLOutput{
		Connection: conn.Info().Name,
		DDL:        ddl,
	}
	return textResult(fmt.Sprintf("Loaded %s DDL for %s.%s from %s.", ddl.ObjectType, ddl.Schema, ddl.Table, conn.Info().Name)), output, nil
}

func (s *service) executeQuery(ctx context.Context, _ *mcp.CallToolRequest, input executeQueryInput) (*mcp.CallToolResult, *executeQueryOutput, error) {
	conn, err := s.manager.Connection(input.Connection)
	if err != nil {
		return errorResult(err), nil, nil
	}
	result, err := conn.ExecuteQuery(ctx, input.SQL, input.MaxRows)
	if err != nil {
		return errorResult(err), nil, nil
	}
	output := &executeQueryOutput{
		Connection: conn.Info().Name,
		Result:     result,
	}
	text := fmt.Sprintf("Returned %d row(s) with %d column(s) from %s in %s.", result.RowCount, len(result.Columns), conn.Info().Name, result.ExecutionTime)
	if result.Truncated {
		text += " Output was truncated: " + strings.Join(result.TruncationReasons, ", ") + "."
	}
	return textResult(text), output, nil
}

func (s *service) explainQuery(ctx context.Context, _ *mcp.CallToolRequest, input explainQueryInput) (*mcp.CallToolResult, *explainQueryOutput, error) {
	conn, err := s.manager.Connection(input.Connection)
	if err != nil {
		return errorResult(err), nil, nil
	}
	explain, err := conn.ExplainQuery(ctx, input.SQL)
	if err != nil {
		return errorResult(err), nil, nil
	}
	output := &explainQueryOutput{
		Connection: conn.Info().Name,
		Explain:    explain,
	}
	text := fmt.Sprintf("Explained query on %s with %d plan line(s).", conn.Info().Name, len(explain.PlanLines))
	if explain.Notice != "" {
		text += " " + explain.Notice
	}
	return textResult(text), output, nil
}
