package database

import (
	"errors"
	"strings"
	"unicode"
)

var (
	ErrEmptyQuery       = errors.New("query is empty")
	ErrMultipleQueries  = errors.New("multiple statements are not allowed")
	ErrWriteQuery       = errors.New("only read-only SELECT or WITH queries are allowed")
	ErrRestrictedClause = errors.New("FOR UPDATE is not allowed")
)

func NormalizeReadOnlyQuery(query string) (string, error) {
	trimmed := strings.TrimSpace(query)
	if trimmed == "" {
		return "", ErrEmptyQuery
	}
	if hasStatementSeparator(trimmed) {
		return "", ErrMultipleQueries
	}
	trimmed = strings.TrimSuffix(trimmed, ";")
	trimmed = strings.TrimSpace(trimmed)
	first, tokens := scanSQLTokens(trimmed)
	switch first {
	case "SELECT", "WITH":
	default:
		return "", ErrWriteQuery
	}
	for _, token := range tokens {
		if token == "FOR" {
			return "", ErrRestrictedClause
		}
	}
	return trimmed, nil
}

func hasStatementSeparator(sql string) bool {
	inSingle := false
	inDouble := false
	inLineComment := false
	inBlockComment := false
	runes := []rune(sql)

	for i, r := range runes {
		next := rune(0)
		if i+1 < len(runes) {
			next = runes[i+1]
		}
		prev := rune(0)
		if i > 0 {
			prev = runes[i-1]
		}

		switch {
		case inLineComment:
			if r == '\n' {
				inLineComment = false
			}
			continue
		case inBlockComment:
			if prev == '*' && r == '/' {
				inBlockComment = false
			}
			continue
		case inSingle:
			if r == '\'' && prev != '\\' {
				inSingle = false
			}
			continue
		case inDouble:
			if r == '"' && prev != '\\' {
				inDouble = false
			}
			continue
		}

		if r == '-' && next == '-' {
			inLineComment = true
			continue
		}
		if r == '/' && next == '*' {
			inBlockComment = true
			continue
		}
		if r == '\'' {
			inSingle = true
			continue
		}
		if r == '"' {
			inDouble = true
			continue
		}
		if r == ';' && i != len(runes)-1 {
			return true
		}
	}
	return false
}

func scanSQLTokens(sql string) (string, []string) {
	var tokens []string
	var current strings.Builder
	flush := func() {
		if current.Len() == 0 {
			return
		}
		tokens = append(tokens, strings.ToUpper(current.String()))
		current.Reset()
	}

	inSingle := false
	inDouble := false
	inLineComment := false
	inBlockComment := false
	runes := []rune(sql)

	for i, r := range runes {
		next := rune(0)
		if i+1 < len(runes) {
			next = runes[i+1]
		}
		prev := rune(0)
		if i > 0 {
			prev = runes[i-1]
		}

		switch {
		case inLineComment:
			if r == '\n' {
				inLineComment = false
			}
			continue
		case inBlockComment:
			if prev == '*' && r == '/' {
				inBlockComment = false
			}
			continue
		case inSingle:
			if r == '\'' && prev != '\\' {
				inSingle = false
			}
			continue
		case inDouble:
			if r == '"' && prev != '\\' {
				inDouble = false
			}
			continue
		}

		if r == '-' && next == '-' {
			flush()
			inLineComment = true
			continue
		}
		if r == '/' && next == '*' {
			flush()
			inBlockComment = true
			continue
		}
		if r == '\'' {
			flush()
			inSingle = true
			continue
		}
		if r == '"' {
			flush()
			inDouble = true
			continue
		}
		if unicode.IsLetter(r) || unicode.IsDigit(r) || r == '_' {
			current.WriteRune(r)
			continue
		}
		flush()
	}
	flush()
	if len(tokens) == 0 {
		return "", nil
	}
	return tokens[0], tokens
}
