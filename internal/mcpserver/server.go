package mcpserver

import (
	"context"
	"fmt"
	"strings"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"db/internal/config"
	"db/internal/database"
)

type service struct {
	cfg     *config.Config
	manager *database.Manager
}

type listInput struct {
	Connection string `json:"connection,omitempty" jsonschema:"optional connection name; default uses the first configured connection"`
	Schema     string `json:"schema,omitempty" jsonschema:"schema name; when omitted, tools use the connection's default schema when applicable"`
	Filter     string `json:"filter,omitempty" jsonschema:"optional case-insensitive contains filter"`
	Limit      int    `json:"limit,omitempty" jsonschema:"optional result limit; clamped by the server"`
}

type describeTableInput struct {
	Connection string `json:"connection,omitempty" jsonschema:"optional connection name; default uses the first configured connection"`
	Schema     string `json:"schema,omitempty" jsonschema:"schema name; defaults to the connection's default schema"`
	Table      string `json:"table" jsonschema:"table or view name"`
}

type executeQueryInput struct {
	Connection string `json:"connection,omitempty" jsonschema:"optional connection name; default uses the first configured connection"`
	SQL        string `json:"sql" jsonschema:"read-only SELECT or WITH query"`
	MaxRows    int    `json:"maxRows,omitempty" jsonschema:"optional row limit; clamped by the server"`
}

type listConnectionsOutput struct {
	DefaultConnection string                    `json:"defaultConnection"`
	Connections       []database.ConnectionInfo `json:"connections"`
}

type listSchemasOutput struct {
	Connection string                `json:"connection"`
	Schemas    []database.SchemaInfo `json:"schemas"`
}

type listTablesOutput struct {
	Connection string               `json:"connection"`
	Tables     []database.TableInfo `json:"tables"`
}

type describeTableOutput struct {
	Connection string                `json:"connection"`
	Schema     string                `json:"schema"`
	Table      string                `json:"table"`
	Columns    []database.ColumnInfo `json:"columns"`
}

type executeQueryOutput struct {
	Connection string                `json:"connection"`
	Result     *database.QueryResult `json:"result"`
}

func New(cfg *config.Config, manager *database.Manager) *mcp.Server {
	server := mcp.NewServer(&mcp.Implementation{
		Name:    cfg.Server.Name,
		Version: cfg.Server.Version,
	}, nil)

	svc := &service{cfg: cfg, manager: manager}

	mcp.AddTool(server, &mcp.Tool{
		Name:        "list_connections",
		Description: "List configured database connections.",
	}, svc.listConnections)

	mcp.AddTool(server, &mcp.Tool{
		Name:        "list_schemas",
		Description: "List visible schemas for a connection.",
	}, svc.listSchemas)

	mcp.AddTool(server, &mcp.Tool{
		Name:        "list_tables",
		Description: "List tables and views in a schema.",
	}, svc.listTables)

	mcp.AddTool(server, &mcp.Tool{
		Name:        "describe_table",
		Description: "Describe table or view columns.",
	}, svc.describeTable)

	mcp.AddTool(server, &mcp.Tool{
		Name:        "execute_query",
		Description: "Run a read-only SELECT or WITH query. Results are truncated to protect MCP context.",
	}, svc.executeQuery)

	return server
}

func (s *service) listConnections(_ context.Context, _ *mcp.CallToolRequest, _ struct{}) (*mcp.CallToolResult, listConnectionsOutput, error) {
	output := listConnectionsOutput{
		DefaultConnection: s.manager.DefaultConnectionName(),
		Connections:       s.manager.ConnectionInfos(),
	}
	return textResult(fmt.Sprintf("Configured %d connection(s). Default is %s.", len(output.Connections), output.DefaultConnection)), output, nil
}

func (s *service) listSchemas(ctx context.Context, _ *mcp.CallToolRequest, input listInput) (*mcp.CallToolResult, listSchemasOutput, error) {
	conn, err := s.manager.Connection(input.Connection)
	if err != nil {
		return nil, listSchemasOutput{}, err
	}
	schemas, err := conn.ListSchemas(ctx, input.Filter, input.Limit)
	if err != nil {
		return nil, listSchemasOutput{}, err
	}
	output := listSchemasOutput{
		Connection: conn.Info().Name,
		Schemas:    schemas,
	}
	return textResult(fmt.Sprintf("Returned %d schema(s) from %s.", len(schemas), conn.Info().Name)), output, nil
}

func (s *service) listTables(ctx context.Context, _ *mcp.CallToolRequest, input listInput) (*mcp.CallToolResult, listTablesOutput, error) {
	conn, err := s.manager.Connection(input.Connection)
	if err != nil {
		return nil, listTablesOutput{}, err
	}
	tables, err := conn.ListTables(ctx, input.Schema, input.Filter, input.Limit)
	if err != nil {
		return nil, listTablesOutput{}, err
	}
	output := listTablesOutput{
		Connection: conn.Info().Name,
		Tables:     tables,
	}
	return textResult(fmt.Sprintf("Returned %d table/view entries from %s.", len(tables), conn.Info().Name)), output, nil
}

func (s *service) describeTable(ctx context.Context, _ *mcp.CallToolRequest, input describeTableInput) (*mcp.CallToolResult, describeTableOutput, error) {
	conn, err := s.manager.Connection(input.Connection)
	if err != nil {
		return nil, describeTableOutput{}, err
	}
	schema := input.Schema
	if schema == "" {
		schema = conn.Info().DefaultSchema
	}
	columns, err := conn.DescribeTable(ctx, schema, input.Table)
	if err != nil {
		return nil, describeTableOutput{}, err
	}
	output := describeTableOutput{
		Connection: conn.Info().Name,
		Schema:     strings.ToUpper(schema),
		Table:      strings.ToUpper(input.Table),
		Columns:    columns,
	}
	return textResult(fmt.Sprintf("Described %s.%s with %d column(s).", output.Schema, output.Table, len(columns))), output, nil
}

func (s *service) executeQuery(ctx context.Context, _ *mcp.CallToolRequest, input executeQueryInput) (*mcp.CallToolResult, executeQueryOutput, error) {
	conn, err := s.manager.Connection(input.Connection)
	if err != nil {
		return nil, executeQueryOutput{}, err
	}
	result, err := conn.ExecuteQuery(ctx, input.SQL, input.MaxRows)
	if err != nil {
		return nil, executeQueryOutput{}, err
	}
	output := executeQueryOutput{
		Connection: conn.Info().Name,
		Result:     result,
	}
	text := fmt.Sprintf("Returned %d row(s) with %d column(s) from %s in %s.", result.RowCount, len(result.Columns), conn.Info().Name, result.ExecutionTime)
	if result.Truncated {
		text += " Output was truncated: " + strings.Join(result.TruncationReasons, ", ") + "."
	}
	return textResult(text), output, nil
}

func textResult(text string) *mcp.CallToolResult {
	return &mcp.CallToolResult{
		Content: []mcp.Content{
			&mcp.TextContent{Text: text},
		},
	}
}
