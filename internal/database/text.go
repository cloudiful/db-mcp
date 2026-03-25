package database

func TruncateLines(lines []string, maxLines, maxTotalChars int) ([]string, bool) {
	if maxLines <= 0 {
		maxLines = len(lines)
	}
	if maxTotalChars <= 0 {
		maxTotalChars = 1 << 30
	}

	truncated := false
	if len(lines) > maxLines {
		lines = append([]string(nil), lines[:maxLines]...)
		truncated = true
	}

	total := 0
	result := make([]string, 0, len(lines))
	for _, line := range lines {
		lineLen := len(line)
		if total+lineLen > maxTotalChars {
			truncated = true
			break
		}
		total += lineLen
		result = append(result, line)
	}
	return result, truncated
}
