package postgres

import (
	"context"
	"database/sql"
	"strings"
	"sync"
	"time"

	"db/internal/config"
	"db/internal/database"

	_ "github.com/jackc/pgx/v5/stdlib"
)

func init() {
	database.Register("postgres", openConnection)
}

type connection struct {
	name             string
	configuredSchema string
	defaultSchema    string
	db               *sql.DB
	limits           database.Limits
	mu               sync.RWMutex
}

func openConnection(ctx context.Context, cfg config.ConnectionConfig, limits database.Limits) (database.Connection, error) {
	_ = ctx
	db, err := sql.Open("pgx", cfg.Postgres.DSN)
	if err != nil {
		return nil, err
	}
	db.SetConnMaxLifetime(5 * time.Minute)
	db.SetMaxIdleConns(2)
	db.SetMaxOpenConns(4)

	defaultSchema := "public"
	configuredSchema := strings.TrimSpace(cfg.Postgres.Schema)
	if configuredSchema != "" {
		defaultSchema = configuredSchema
	}

	return &connection{
		name:             cfg.Name,
		configuredSchema: configuredSchema,
		defaultSchema:    defaultSchema,
		db:               db,
		limits:           limits,
	}, nil
}

func (c *connection) Info() database.ConnectionInfo {
	return database.ConnectionInfo{
		Name:          c.name,
		Driver:        "postgres",
		DefaultSchema: c.schema(),
	}
}

func (c *connection) Ping(ctx context.Context) error {
	pingCtx, cancel := c.limits.WithTimeout(ctx)
	defer cancel()
	if err := c.db.PingContext(pingCtx); err != nil {
		return err
	}
	return c.refreshDefaultSchema(pingCtx)
}

func (c *connection) schema() string {
	c.mu.RLock()
	defer c.mu.RUnlock()
	return c.defaultSchema
}

func (c *connection) refreshDefaultSchema(ctx context.Context) error {
	if c.configuredSchema != "" {
		if err := ensureSchemaVisible(ctx, c.db, c.configuredSchema); err != nil {
			return err
		}
		c.mu.Lock()
		c.defaultSchema = c.configuredSchema
		c.mu.Unlock()
		return nil
	}

	schema, err := fetchDefaultSchema(ctx, c.db)
	if err != nil {
		return err
	}
	c.mu.Lock()
	c.defaultSchema = schema
	c.mu.Unlock()
	return nil
}

func (c *connection) Close() error {
	return c.db.Close()
}
