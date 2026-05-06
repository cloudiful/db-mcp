package mcpserver

import (
	"github.com/modelcontextprotocol/go-sdk/mcp"

	"db/internal/config"
	"db/internal/database"
)

type service struct {
	cfg     *config.Config
	manager *database.Manager
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
		Name:        "list_indexes",
		Description: "List visible indexes for a schema or table.",
	}, svc.listIndexes)

	mcp.AddTool(server, &mcp.Tool{
		Name:        "list_constraints",
		Description: "List visible table constraints for a schema or table.",
	}, svc.listConstraints)

	mcp.AddTool(server, &mcp.Tool{
		Name:        "get_table_comments",
		Description: "Get table and column comments for a table or view.",
	}, svc.getTableComments)

	mcp.AddTool(server, &mcp.Tool{
		Name:        "describe_table",
		Description: "Describe table or view columns.",
	}, svc.describeTable)

	mcp.AddTool(server, &mcp.Tool{
		Name:        "get_table_ddl",
		Description: "Return a table or view DDL definition in a normalized read-only form.",
	}, svc.getTableDDL)

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
		Description: "Combined metadata for a table including columns, constraints, indexes, and comments.",
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
