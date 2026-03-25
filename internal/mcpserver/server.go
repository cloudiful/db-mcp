package mcpserver

import (
	"context"
	"encoding/json"
	"fmt"
	"net/url"
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

type searchColumnsInput struct {
	Connection string `json:"connection,omitempty" jsonschema:"optional connection name; default uses the first configured connection"`
	Schema     string `json:"schema,omitempty" jsonschema:"schema name; defaults to the connection's default schema"`
	Filter     string `json:"filter,omitempty" jsonschema:"optional case-insensitive contains filter for column names"`
	Limit      int    `json:"limit,omitempty" jsonschema:"optional result limit; clamped by the server"`
}

type listForeignKeysInput struct {
	Connection string `json:"connection,omitempty" jsonschema:"optional connection name; default uses the first configured connection"`
	Schema     string `json:"schema,omitempty" jsonschema:"schema name; defaults to the connection's default schema"`
	Table      string `json:"table,omitempty" jsonschema:"optional table name; when omitted, list foreign keys across the schema"`
	Limit      int    `json:"limit,omitempty" jsonschema:"optional result limit; clamped by the server"`
}

type listIndexesInput struct {
	Connection string `json:"connection,omitempty" jsonschema:"optional connection name; default uses the first configured connection"`
	Schema     string `json:"schema,omitempty" jsonschema:"schema name; defaults to the connection's default schema"`
	Table      string `json:"table,omitempty" jsonschema:"optional table name; when omitted, list indexes across the schema"`
	Limit      int    `json:"limit,omitempty" jsonschema:"optional result limit; clamped by the server"`
}

type getTableCommentsInput struct {
	Connection string `json:"connection,omitempty" jsonschema:"optional connection name; default uses the first configured connection"`
	Schema     string `json:"schema,omitempty" jsonschema:"schema name; defaults to the connection's default schema"`
	Table      string `json:"table" jsonschema:"table or view name"`
}

type executeQueryInput struct {
	Connection string `json:"connection,omitempty" jsonschema:"optional connection name; default uses the first configured connection"`
	SQL        string `json:"sql" jsonschema:"read-only SELECT or WITH query"`
	MaxRows    int    `json:"maxRows,omitempty" jsonschema:"optional row limit; clamped by the server"`
}

type explainQueryInput struct {
	Connection string `json:"connection,omitempty" jsonschema:"optional connection name; default uses the first configured connection"`
	SQL        string `json:"sql" jsonschema:"read-only SELECT or WITH query to explain"`
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

type searchColumnsOutput struct {
	Connection string                     `json:"connection"`
	Matches    []database.ColumnMatchInfo `json:"matches"`
}

type listForeignKeysOutput struct {
	Connection  string                    `json:"connection"`
	ForeignKeys []database.ForeignKeyInfo `json:"foreignKeys"`
}

type listIndexesOutput struct {
	Connection string               `json:"connection"`
	Indexes    []database.IndexInfo `json:"indexes"`
}

type getTableCommentsOutput struct {
	Connection string                     `json:"connection"`
	Comments   *database.TableCommentInfo `json:"comments"`
}

type executeQueryOutput struct {
	Connection string                `json:"connection"`
	Result     *database.QueryResult `json:"result"`
}

type explainQueryOutput struct {
	Connection string                  `json:"connection"`
	Explain    *database.ExplainResult `json:"explain"`
}

type tableResourceOutput struct {
	Connection  string                     `json:"connection"`
	Schema      string                     `json:"schema"`
	Table       string                     `json:"table"`
	Columns     []database.ColumnInfo      `json:"columns"`
	ForeignKeys []database.ForeignKeyInfo  `json:"foreignKeys"`
	Indexes     []database.IndexInfo       `json:"indexes"`
	Comments    *database.TableCommentInfo `json:"comments"`
}

func New(cfg *config.Config, manager *database.Manager) *mcp.Server {
	server := mcp.NewServer(&mcp.Implementation{
		Name:    cfg.Server.Name,
		Version: cfg.Server.Version,
	}, &mcp.ServerOptions{
		Instructions: serverInstructions(manager),
	})

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
		Name:        "search_columns",
		Description: "Search visible columns by name within a schema.",
	}, svc.searchColumns)

	mcp.AddTool(server, &mcp.Tool{
		Name:        "list_foreign_keys",
		Description: "List visible foreign key relationships for a schema or table.",
	}, svc.listForeignKeys)

	mcp.AddTool(server, &mcp.Tool{
		Name:        "list_indexes",
		Description: "List visible indexes for a schema or table.",
	}, svc.listIndexes)

	mcp.AddTool(server, &mcp.Tool{
		Name:        "get_table_comments",
		Description: "Get table and column comments for a table or view.",
	}, svc.getTableComments)

	mcp.AddTool(server, &mcp.Tool{
		Name:        "describe_table",
		Description: "Describe table or view columns.",
	}, svc.describeTable)

	mcp.AddTool(server, &mcp.Tool{
		Name:        "execute_query",
		Description: "Run a read-only SELECT or WITH query. Results are truncated to protect MCP context.",
	}, svc.executeQuery)

	mcp.AddTool(server, &mcp.Tool{
		Name:        "explain_query",
		Description: "Explain a read-only SELECT or WITH query without executing data changes.",
	}, svc.explainQuery)

	server.AddResource(&mcp.Resource{
		URI:         "db://connections",
		Name:        "connections",
		Title:       "Database Connections",
		Description: "Configured database connections and the default connection for this server.",
		MIMEType:    "application/json",
	}, svc.readConnectionsResource)

	server.AddResourceTemplate(&mcp.ResourceTemplate{
		URITemplate: "db://table/{connection}/{schema}/{table}",
		Name:        "table_overview",
		Title:       "Table Overview",
		Description: "Combined metadata for a table including columns, foreign keys, indexes, and comments.",
		MIMEType:    "application/json",
	}, svc.readTableResource)

	server.AddPrompt(&mcp.Prompt{
		Name:        "explore_database",
		Description: "Suggested step-by-step workflow for exploring an unfamiliar database safely.",
	}, svc.exploreDatabasePrompt)

	server.AddPrompt(&mcp.Prompt{
		Name:        "inspect_table",
		Description: "Suggested workflow for inspecting a table and writing a safe read-only query.",
		Arguments: []*mcp.PromptArgument{
			{
				Name:        "table",
				Description: "table or view name to inspect",
				Required:    true,
			},
			{
				Name:        "schema",
				Description: "schema name if known",
			},
			{
				Name:        "connection",
				Description: "connection name if multiple databases are configured",
			},
		},
	}, svc.inspectTablePrompt)

	return server
}

func (s *service) listConnections(_ context.Context, _ *mcp.CallToolRequest, _ struct{}) (*mcp.CallToolResult, *listConnectionsOutput, error) {
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

func (s *service) listForeignKeys(ctx context.Context, _ *mcp.CallToolRequest, input listForeignKeysInput) (*mcp.CallToolResult, *listForeignKeysOutput, error) {
	conn, err := s.manager.Connection(input.Connection)
	if err != nil {
		return errorResult(err), nil, nil
	}
	schema := input.Schema
	if schema == "" {
		schema = conn.Info().DefaultSchema
	}
	foreignKeys, err := conn.ListForeignKeys(ctx, schema, input.Table, input.Limit)
	if err != nil {
		return errorResult(err), nil, nil
	}
	output := &listForeignKeysOutput{
		Connection:  conn.Info().Name,
		ForeignKeys: foreignKeys,
	}
	return textResult(fmt.Sprintf("Returned %d foreign key row(s) from %s.", len(foreignKeys), conn.Info().Name)), output, nil
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
	output := &listIndexesOutput{
		Connection: conn.Info().Name,
		Indexes:    indexes,
	}
	return textResult(fmt.Sprintf("Returned %d index row(s) from %s.", len(indexes), conn.Info().Name)), output, nil
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
	output := &getTableCommentsOutput{
		Connection: conn.Info().Name,
		Comments:   comments,
	}
	return textResult(fmt.Sprintf("Loaded comments for %s.%s from %s.", comments.Schema, comments.Table, conn.Info().Name)), output, nil
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
	output := &describeTableOutput{
		Connection: conn.Info().Name,
		Schema:     schema,
		Table:      input.Table,
		Columns:    columns,
	}
	return textResult(fmt.Sprintf("Described %s.%s with %d column(s).", output.Schema, output.Table, len(columns))), output, nil
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

func (s *service) readConnectionsResource(_ context.Context, req *mcp.ReadResourceRequest) (*mcp.ReadResourceResult, error) {
	if req.Params.URI != "db://connections" {
		return nil, mcp.ResourceNotFoundError(req.Params.URI)
	}
	body, err := json.MarshalIndent(&listConnectionsOutput{
		DefaultConnection: s.manager.DefaultConnectionName(),
		Connections:       s.manager.ConnectionInfos(),
	}, "", "  ")
	if err != nil {
		return nil, err
	}
	return &mcp.ReadResourceResult{
		Contents: []*mcp.ResourceContents{{
			URI:      req.Params.URI,
			MIMEType: "application/json",
			Text:     string(body),
		}},
	}, nil
}

func (s *service) readTableResource(ctx context.Context, req *mcp.ReadResourceRequest) (*mcp.ReadResourceResult, error) {
	connection, schema, table, err := parseTableResourceURI(req.Params.URI)
	if err != nil {
		return nil, mcp.ResourceNotFoundError(req.Params.URI)
	}
	body, err := s.buildTableResource(ctx, connection, schema, table)
	if err != nil {
		return nil, err
	}
	return &mcp.ReadResourceResult{
		Contents: []*mcp.ResourceContents{{
			URI:      req.Params.URI,
			MIMEType: "application/json",
			Text:     body,
		}},
	}, nil
}

func (s *service) exploreDatabasePrompt(_ context.Context, _ *mcp.GetPromptRequest) (*mcp.GetPromptResult, error) {
	defaultConnection := s.manager.DefaultConnectionName()
	return &mcp.GetPromptResult{
		Description: "A safe workflow for exploring an unfamiliar database with this MCP server.",
		Messages: []*mcp.PromptMessage{
			{
				Role: "user",
				Content: &mcp.TextContent{Text: fmt.Sprintf(
					"Use this database server cautiously and prefer discovery before ad hoc SQL. Start with `list_connections` or read the `db://connections` resource to confirm the available databases and note that the default connection is `%s`. Then use `list_schemas`, `list_tables`, `search_columns`, `list_foreign_keys`, `list_indexes`, `get_table_comments`, and `describe_table` to understand structure before calling `execute_query`. If the client supports MCP resources, `db://table/{connection}/{schema}/{table}` gives a compact metadata bundle for one table. Keep queries read-only, prefer explicit columns over `SELECT *`, and keep result sets small with predicates or `maxRows`.",
					defaultConnection,
				)},
			},
		},
	}, nil
}

func (s *service) inspectTablePrompt(_ context.Context, req *mcp.GetPromptRequest) (*mcp.GetPromptResult, error) {
	args := req.Params.Arguments
	table := strings.TrimSpace(args["table"])
	if table == "" {
		table = "<table>"
	}
	schema := strings.TrimSpace(args["schema"])
	connection := strings.TrimSpace(args["connection"])
	var contextParts []string
	if connection != "" {
		contextParts = append(contextParts, fmt.Sprintf("connection `%s`", connection))
	}
	if schema != "" {
		contextParts = append(contextParts, fmt.Sprintf("schema `%s`", schema))
	}
	contextHint := ""
	if len(contextParts) > 0 {
		contextHint = "Use " + strings.Join(contextParts, " and ") + ". "
	}

	return &mcp.GetPromptResult{
		Description: "A safe checklist for understanding one table before querying it.",
		Messages: []*mcp.PromptMessage{
			{
				Role: "user",
				Content: &mcp.TextContent{Text: fmt.Sprintf(
					"%sInspect table `%s` in this order: 1. If the schema is not certain, use `search_columns` or `list_tables` first. 2. If your client supports resources, prefer reading `db://table/{connection}/{schema}/{table}` for one compact overview. 3. Otherwise call `describe_table`, `list_foreign_keys`, `list_indexes`, and `get_table_comments` to understand structure, joins, and annotations. 4. Use `explain_query` when you need to reason about the access path before running a larger read. 5. Only then write an `execute_query` call with a read-only `SELECT` or `WITH` statement, explicit columns, and a small `maxRows`. 6. If the first result is truncated, narrow the query instead of broadening limits immediately.",
					contextHint,
					table,
				)},
			},
		},
	}, nil
}

func serverInstructions(manager *database.Manager) string {
	return fmt.Sprintf(
		"Use this server for safe read-only database exploration. Start with list_connections or db://connections, then use list_schemas, list_tables, search_columns, list_foreign_keys, list_indexes, get_table_comments, and describe_table before execute_query. For one table, the db://table/{connection}/{schema}/{table} resource provides a compact metadata bundle. Use explain_query before expensive reads when supported. The default connection is %q. Keep queries to SELECT or WITH, prefer explicit columns, and keep result sets small.",
		manager.DefaultConnectionName(),
	)
}

func (s *service) buildTableResource(ctx context.Context, connectionName, schema, table string) (string, error) {
	conn, err := s.manager.Connection(connectionName)
	if err != nil {
		return "", mcp.ResourceNotFoundError("db://table/" + connectionName + "/" + schema + "/" + table)
	}
	if schema == "" {
		schema = conn.Info().DefaultSchema
	}
	columns, err := conn.DescribeTable(ctx, schema, table)
	if err != nil {
		return "", err
	}
	foreignKeys, err := conn.ListForeignKeys(ctx, schema, table, s.manager.Limits().HardMaxRows)
	if err != nil {
		return "", err
	}
	indexes, err := conn.ListIndexes(ctx, schema, table, s.manager.Limits().HardMaxRows)
	if err != nil {
		return "", err
	}
	comments, err := conn.GetTableComments(ctx, schema, table)
	if err != nil {
		return "", err
	}
	body, err := json.MarshalIndent(&tableResourceOutput{
		Connection:  conn.Info().Name,
		Schema:      schema,
		Table:       table,
		Columns:     columns,
		ForeignKeys: foreignKeys,
		Indexes:     indexes,
		Comments:    comments,
	}, "", "  ")
	if err != nil {
		return "", err
	}
	return string(body), nil
}

func parseTableResourceURI(raw string) (connection string, schema string, table string, err error) {
	parsed, err := url.Parse(raw)
	if err != nil {
		return "", "", "", err
	}
	if parsed.Scheme != "db" || parsed.Host != "table" {
		return "", "", "", fmt.Errorf("unsupported resource uri %q", raw)
	}
	parts := strings.Split(strings.TrimPrefix(parsed.Path, "/"), "/")
	if len(parts) != 3 {
		return "", "", "", fmt.Errorf("invalid table resource uri %q", raw)
	}
	return parts[0], parts[1], parts[2], nil
}

func textResult(text string) *mcp.CallToolResult {
	return &mcp.CallToolResult{
		Content: []mcp.Content{
			&mcp.TextContent{Text: text},
		},
	}
}

func errorResult(err error) *mcp.CallToolResult {
	return &mcp.CallToolResult{
		IsError: true,
		Content: []mcp.Content{
			&mcp.TextContent{Text: err.Error()},
		},
	}
}
