package pmtiles

import (
	"context"
	"io"
)

// The native Rust owner supplies immutable, validated ranges. This helper has no HTTP client.
type Bucket interface {
	NewRangeReader(context.Context, string, int64, int64) (io.ReadCloser, error)
}
