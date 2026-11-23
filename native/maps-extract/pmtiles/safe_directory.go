package pmtiles

import (
	"bytes"
	"compress/gzip"
	"encoding/binary"
	"fmt"
	"io"
	"math"
)

const maxDirectoryBytes = 8 << 20
const maxExtractEntries = 500000

func safeEntries(data []byte, compression Compression) ([]EntryV3, error) {
	if len(data) > maxDirectoryBytes {
		return nil, fmt.Errorf("map directory exceeds memory budget")
	}
	if compression == Gzip {
		reader, err := gzip.NewReader(bytes.NewReader(data))
		if err != nil {
			return nil, err
		}
		decoded, err := io.ReadAll(io.LimitReader(reader, maxDirectoryBytes+1))
		reader.Close()
		if err != nil {
			return nil, err
		}
		data = decoded
	} else if compression != NoCompression {
		return nil, fmt.Errorf("unsupported map directory compression")
	}
	if len(data) > maxDirectoryBytes {
		return nil, fmt.Errorf("map directory exceeds memory budget")
	}
	reader := bytes.NewReader(data)
	count, err := binary.ReadUvarint(reader)
	if err != nil || count > maxExtractEntries || count > uint64(reader.Len()/4) {
		return nil, fmt.Errorf("invalid map directory count")
	}
	entries := make([]EntryV3, int(count))
	var last uint64
	for i := range entries {
		delta, err := binary.ReadUvarint(reader)
		if err != nil || delta > math.MaxUint64-last {
			return nil, fmt.Errorf("invalid map tile ID")
		}
		last += delta
		entries[i].TileID = last
	}
	for i := range entries {
		value, err := binary.ReadUvarint(reader)
		if err != nil || value > math.MaxUint32 {
			return nil, fmt.Errorf("invalid map run")
		}
		entries[i].RunLength = uint32(value)
	}
	for i := range entries {
		value, err := binary.ReadUvarint(reader)
		if err != nil || value == 0 || value > math.MaxUint32 {
			return nil, fmt.Errorf("invalid map tile length")
		}
		entries[i].Length = uint32(value)
	}
	for i := range entries {
		value, err := binary.ReadUvarint(reader)
		if err != nil || (i == 0 && value == 0) {
			return nil, fmt.Errorf("invalid map tile offset")
		}
		if value == 0 {
			previous := entries[i-1]
			if previous.Offset > math.MaxUint64-uint64(previous.Length) {
				return nil, fmt.Errorf("map tile offset overflow")
			}
			entries[i].Offset = previous.Offset + uint64(previous.Length)
		} else {
			entries[i].Offset = value - 1
		}
	}
	if reader.Len() != 0 {
		return nil, fmt.Errorf("unexpected map directory data")
	}
	return entries, nil
}
