package main

import (
	"context"
	"errors"
	"flag"
	"log"
	"net"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"github.com/modelcontextprotocol/go-sdk/mcp"

	"db/internal/config"
	"db/internal/database"
	_ "db/internal/database/oracle"
	"db/internal/mcpserver"
)

func main() {
	var (
		configPath string
		modeFlag   string
		httpAddr   string
		httpPath   string
	)

	flag.StringVar(&configPath, "config", "", "path to JSON config file; falls back to environment variables")
	flag.StringVar(&modeFlag, "transport", "", "transport override: stdio, http, or both")
	flag.StringVar(&httpAddr, "http-addr", "", "HTTP listen address override")
	flag.StringVar(&httpPath, "http-path", "", "HTTP path override")
	flag.Parse()

	cfg, err := config.Load(configPath)
	if err != nil {
		log.Fatalf("load config: %v", err)
	}
	if modeFlag != "" {
		cfg.Server.Transport = config.TransportMode(modeFlag)
	}
	if httpAddr != "" {
		cfg.Server.HTTPAddr = httpAddr
	}
	if httpPath != "" {
		cfg.Server.HTTPPath = httpPath
	}
	if err := cfg.Validate(); err != nil {
		log.Fatalf("validate config: %v", err)
	}

	ctx, stop := signal.NotifyContext(context.Background(), os.Interrupt, syscall.SIGTERM)
	defer stop()

	manager, err := database.NewManager(ctx, cfg)
	if err != nil {
		log.Fatalf("open database connections: %v", err)
	}
	defer manager.Close()

	logStartup(cfg, manager)

	server := mcpserver.New(cfg, manager)

	if err := run(ctx, cfg, server); err != nil && !errors.Is(err, context.Canceled) {
		log.Fatalf("run server: %v", err)
	}
}

func logStartup(cfg *config.Config, manager *database.Manager) {
	log.Printf("db-mcp starting: transport=%s", cfg.Server.Transport)
	for _, info := range manager.ConnectionInfos() {
		log.Printf(
			"database connected: name=%s driver=%s default_schema=%s status=ok",
			info.Name,
			info.Driver,
			info.DefaultSchema,
		)
	}

	switch cfg.Server.Transport {
	case config.TransportHTTP:
		logHTTPListen(cfg)
	case config.TransportBoth:
		logHTTPListen(cfg)
		log.Printf("stdio MCP enabled")
	case config.TransportStdio:
		log.Printf("stdio MCP enabled")
	}
}

func logHTTPListen(cfg *config.Config) {
	host, port := splitListenAddr(cfg.Server.HTTPAddr)
	log.Printf(
		"http MCP listening: host=%s port=%s path=%s healthz=/healthz",
		host,
		port,
		cfg.Server.HTTPPath,
	)
}

func splitListenAddr(addr string) (string, string) {
	if addr == "" {
		return "0.0.0.0", "8080"
	}
	host, port, err := net.SplitHostPort(addr)
	if err == nil {
		if host == "" {
			host = "0.0.0.0"
		}
		return host, port
	}
	return addr, ""
}

func run(ctx context.Context, cfg *config.Config, server *mcp.Server) error {
	switch cfg.Server.Transport {
	case config.TransportStdio:
		return server.Run(ctx, &mcp.StdioTransport{})
	case config.TransportHTTP:
		return runHTTP(ctx, cfg, server)
	case config.TransportBoth:
		return runBoth(ctx, cfg, server)
	default:
		return config.ErrInvalidTransport
	}
}

func runHTTP(ctx context.Context, cfg *config.Config, server *mcp.Server) error {
	handler := mcp.NewStreamableHTTPHandler(func(r *http.Request) *mcp.Server {
		return server
	}, &mcp.StreamableHTTPOptions{})

	mux := http.NewServeMux()
	mux.Handle(cfg.Server.HTTPPath, handler)
	mux.HandleFunc("/healthz", func(w http.ResponseWriter, _ *http.Request) {
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte("ok"))
	})

	httpServer := &http.Server{
		Addr:              cfg.Server.HTTPAddr,
		Handler:           mux,
		ReadHeaderTimeout: 10 * time.Second,
	}

	go func() {
		<-ctx.Done()
		shutdownCtx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		_ = httpServer.Shutdown(shutdownCtx)
	}()

	if err := httpServer.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
		return err
	}
	return nil
}

func runBoth(ctx context.Context, cfg *config.Config, server *mcp.Server) error {
	errCh := make(chan error, 2)

	go func() {
		errCh <- runHTTP(ctx, cfg, server)
	}()
	go func() {
		errCh <- server.Run(ctx, &mcp.StdioTransport{})
	}()

	select {
	case <-ctx.Done():
		return ctx.Err()
	case err := <-errCh:
		return err
	}
}
