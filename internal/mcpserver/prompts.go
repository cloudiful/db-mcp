package mcpserver

import (
	"context"
	"fmt"
	"strings"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"db/internal/database"
)

func (s *service) exploreDatabasePrompt(_ context.Context, _ *mcp.GetPromptRequest) (*mcp.GetPromptResult, error) {
	defaultConnection := s.manager.DefaultConnectionName()
	return &mcp.GetPromptResult{
		Description: "A safe workflow for exploring an unfamiliar database with this MCP server.",
		Messages: []*mcp.PromptMessage{
			{
				Role: "user",
				Content: &mcp.TextContent{Text: fmt.Sprintf(
					"Use this database server cautiously and prefer discovery before ad hoc SQL. Start with `list_connections` or read the `db://connections` resource to confirm the available databases and note that the default connection is `%s`. If availability is uncertain, call `get_health` or read `db://health` before schema discovery. Then use `list_schemas`, `list_tables`, `search_columns`, `list_foreign_keys`, `list_indexes`, `get_table_comments`, and `describe_table` to understand structure before calling `execute_query`. If the client supports MCP resources, `db://table/{connection}/{schema}/{table}` gives a compact metadata bundle for one table. Keep queries read-only, prefer explicit columns over `SELECT *`, and keep result sets small with predicates or `maxRows`.",
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
					"%sInspect table `%s` in this order: 1. If availability is uncertain, call `get_health` or read `db://health` first. 2. If the schema is not certain, use `search_columns` or `list_tables` first. 3. If your client supports resources, prefer reading `db://table/{connection}/{schema}/{table}` for one compact overview. 4. Otherwise call `describe_table`, `list_foreign_keys`, `list_indexes`, and `get_table_comments` to understand structure, joins, and annotations. 5. Use `explain_query` when you need to reason about the access path before running a larger read. 6. Only then write an `execute_query` call with a read-only `SELECT` or `WITH` statement, explicit columns, and a small `maxRows`. 7. If the first result is truncated, narrow the query instead of broadening limits immediately.",
					contextHint,
					table,
				)},
			},
		},
	}, nil
}

func serverInstructions(manager *database.Manager) string {
	return fmt.Sprintf(
		"Use this server for safe read-only database exploration. Start with list_connections or db://connections. If availability is uncertain, call get_health or read db://health before schema work. Then use list_schemas, list_tables, search_columns, list_foreign_keys, list_indexes, get_table_comments, and describe_table before execute_query. For one table, the db://table/{connection}/{schema}/{table} resource provides a compact metadata bundle. Use explain_query before expensive reads when supported. The default connection is %q. Keep queries to SELECT or WITH, prefer explicit columns, and keep result sets small.",
		manager.DefaultConnectionName(),
	)
}
