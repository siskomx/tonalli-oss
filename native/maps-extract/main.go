package main

/*
#include <stdint.h>
typedef uint64_t (*range_open_fn)(void*, int64_t, int64_t);
typedef int (*plan_fn)(void*, uint64_t, uint64_t);
static inline uint64_t open_range(uintptr_t f, void *context, int64_t offset, int64_t length) {
 return ((range_open_fn)f)(context, offset, length);
}
static inline int check_plan(uintptr_t f, void *context, uint64_t output, uint64_t transfer) {
 return ((plan_fn)f)(context, output, transfer);
}
*/
import "C"

import (
	"context"
	"fmt"
	"io"
	"log"
	"os"
	"runtime/debug"
	"tonalli.local/maps/pmtiles"
	"unsafe"
)

type nativeBucket struct {
	owner unsafe.Pointer
	open  C.uintptr_t
}

func (b nativeBucket) NewRangeReader(ctx context.Context, _ string, offset, length int64) (io.ReadCloser, error) {
	if err := ctx.Err(); err != nil {
		return nil, err
	}
	if offset < 0 || length < 0 {
		return nil, fmt.Errorf("invalid source range")
	}
	handle := C.open_range(b.open, b.owner, C.int64_t(offset), C.int64_t(length))
	if uint64(handle) == ^uint64(0) {
		return nil, fmt.Errorf("native source unavailable")
	}
	file := os.NewFile(uintptr(handle), "country-map-range")
	if file == nil {
		return nil, fmt.Errorf("invalid native source handle")
	}
	return file, nil
}

// All network, storage authority and cancellation stay with the native Rust owner.
//
//export TonalliExtractMap
func TonalliExtractMap(owner unsafe.Pointer, open C.uintptr_t, plan C.uintptr_t, region *C.char, bbox *C.char, output *C.char, zoom C.int, dry C.int, outputBytes *C.uint64_t, transferBytes *C.uint64_t) (status C.int) {
	status = 2
	defer func() {
		if recover() != nil {
			status = 2
		}
		debug.FreeOSMemory()
	}()
	if owner == nil || open == 0 || plan == 0 || region == nil || bbox == nil || output == nil || zoom < 0 || zoom > 15 || outputBytes == nil || transferBytes == nil {
		return 2
	}
	debug.SetMemoryLimit(192 << 20)
	native := nativeBucket{owner, open}
	err := pmtiles.Extract(context.Background(), log.New(io.Discard, "", 0), native, "source", 0, int8(zoom), C.GoString(region), C.GoString(bbox), C.GoString(output), 2, .05, dry != 0, func(size, transfer uint64) error {
		*outputBytes = C.uint64_t(size)
		*transferBytes = C.uint64_t(transfer)
		if C.check_plan(plan, owner, C.uint64_t(size), C.uint64_t(transfer)) != 0 {
			return fmt.Errorf("native preflight refused")
		}
		return nil
	})
	if err != nil {
		return 1
	}
	return 0
}

func main() {}
