package pmtiles

import "testing"

func TestNativeDirectoryRejectsTruncation(t *testing.T) {
	if _, err := safeEntries([]byte{1}, NoCompression); err == nil {
		t.Fatal("truncated directory accepted")
	}
}
