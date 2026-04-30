package database

import (
	"context"
	"fmt"
	"time"
)

func (s *connectionSlot) info() ConnectionInfo {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return ConnectionInfo{
		Name:             s.health.Name,
		Driver:           s.health.Driver,
		DefaultSchema:    s.health.DefaultSchema,
		Status:           s.health.Status,
		Available:        s.health.Available,
		LastCheckedAt:    copyTimePtr(s.health.LastCheckedAt),
		LastSuccessfulAt: copyTimePtr(s.health.LastSuccessfulAt),
		LastError:        s.health.LastError,
	}
}

func (s *connectionSlot) snapshot() ConnectionHealth {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return ConnectionHealth{
		Name:             s.health.Name,
		Driver:           s.health.Driver,
		DefaultSchema:    s.health.DefaultSchema,
		Status:           s.health.Status,
		Available:        s.health.Available,
		LastCheckedAt:    copyTimePtr(s.health.LastCheckedAt),
		LastSuccessfulAt: copyTimePtr(s.health.LastSuccessfulAt),
		LastError:        s.health.LastError,
	}
}

func (s *connectionSlot) refresh(ctx context.Context) error {
	if err := s.conn.Ping(ctx); err != nil {
		s.markDown(err)
		return err
	}
	s.markUp()
	return nil
}

func (s *connectionSlot) ensureReady(ctx context.Context) error {
	s.mu.RLock()
	available := s.health.Available
	s.mu.RUnlock()
	if available {
		return nil
	}
	if err := s.refresh(ctx); err != nil {
		return unavailableError(s.health.Name, err)
	}
	return nil
}

func (s *connectionSlot) markUp() {
	info := s.conn.Info()
	now := time.Now().UTC()

	s.mu.Lock()
	defer s.mu.Unlock()
	s.health.Name = info.Name
	s.health.Driver = info.Driver
	s.health.DefaultSchema = info.DefaultSchema
	s.health.Status = StatusUp
	s.health.Available = true
	s.health.LastCheckedAt = copyTimePtr(&now)
	s.health.LastSuccessfulAt = copyTimePtr(&now)
	s.health.LastError = ""
}

func (s *connectionSlot) markDown(err error) {
	info := s.conn.Info()
	now := time.Now().UTC()

	s.mu.Lock()
	defer s.mu.Unlock()
	s.health.Name = info.Name
	s.health.Driver = info.Driver
	s.health.DefaultSchema = info.DefaultSchema
	s.health.Status = StatusDown
	s.health.Available = false
	s.health.LastCheckedAt = copyTimePtr(&now)
	if err != nil {
		s.health.LastError = err.Error()
	}
}

func (s *connectionSlot) handleOperationError(err error) error {
	pingCtx, cancel := context.WithTimeout(context.Background(), s.limits.QueryTimeout)
	defer cancel()
	if pingErr := s.conn.Ping(pingCtx); pingErr != nil {
		s.markDown(pingErr)
		return unavailableError(s.conn.Info().Name, pingErr)
	}
	s.markUp()
	return err
}

func overallStatus(connections []ConnectionHealth) string {
	if len(connections) == 0 {
		return StatusDown
	}
	available := 0
	for _, connection := range connections {
		if connection.Available {
			available++
		}
	}
	switch {
	case available == 0:
		return StatusDown
	case available == len(connections):
		return StatusUp
	default:
		return StatusDegraded
	}
}

func unavailableError(name string, err error) error {
	return fmt.Errorf("connection %q is unavailable: %w", name, err)
}

func copyTimePtr(value *time.Time) *time.Time {
	if value == nil {
		return nil
	}
	copied := *value
	return &copied
}
