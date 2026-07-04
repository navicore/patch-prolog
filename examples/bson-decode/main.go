// Command bson-decode demonstrates the patch-prolog bson wire format with
// `--atoms`: it fetches the atom map from a compiled binary, runs a query as
// bson, and decodes the term values (TermBuf cells) using that map — producing
// readable output equivalent to `--format text`, but arrived at through the
// binary wire path.
//
// This is the reference consumer: no JSON in the engine, all decode is host-side.
//
// Usage:
//   go run . <compiled-binary> <query>
//
// Example:
//   plgc build examples/deps.pl -o /tmp/deps
//   cd examples/bson-decode && go run . /tmp/deps 'shares_dep(render, auth, D)'
//   # D = crypto
package main

import (
	"bytes"
	"encoding/binary"
	"fmt"
	"math"
	"os"
	"os/exec"
)

// ── bson document decode (minimal, for our envelope + atom map) ────────────

type bsonParser struct {
	b []byte
	i int
}

func (p *bsonParser) readI32() int32 {
	v := int32(binary.LittleEndian.Uint32(p.b[p.i:]))
	p.i += 4
	return v
}

func (p *bsonParser) readU64() uint64 {
	v := binary.LittleEndian.Uint64(p.b[p.i:])
	p.i += 8
	return v
}

func (p *bsonParser) readByte() byte {
	v := p.b[p.i]
	p.i++
	return v
}

func (p *bsonParser) readCString() string {
	start := p.i
	for p.b[p.i] != 0 {
		p.i++
	}
	s := string(p.b[start:p.i])
	p.i++ // skip null
	return s
}

func (p *bsonParser) readString() string {
	n := p.readI32()
	s := string(p.b[p.i : p.i+int(n)-1])
	p.i += int(n)
	return s
}

// parseDoc returns the fields as a map[string]any. Values are: string, int32,
// bool, []byte (binary BinData), []any (array), or map[string]any (nested doc).
func (p *bsonParser) parseDoc() map[string]any {
	start := p.i
	total := p.readI32()
	end := start + int(total)
	m := make(map[string]any)
	for p.i < end-1 {
		ty := p.readByte()
		key := p.readCString()
		m[key] = p.readValue(ty)
	}
	p.i++ // terminator
	return m
}

func (p *bsonParser) parseArray() []any {
	start := p.i
	total := p.readI32()
	end := start + int(total)
	var arr []any
	for p.i < end-1 {
		ty := p.readByte()
		p.readCString() // array keys "0","1",... ignored
		arr = append(arr, p.readValue(ty))
	}
	p.i++ // terminator
	return arr
}

func (p *bsonParser) readValue(ty byte) any {
	switch ty {
	case 0x02: // string
		return p.readString()
	case 0x03: // document
		return p.parseDoc()
	case 0x04: // array
		return p.parseArray()
	case 0x05: // binary
		n := p.readI32()
		p.readByte() // subtype (0x00 for TermBuf)
		data := make([]byte, n)
		copy(data, p.b[p.i:p.i+int(n)])
		p.i += int(n)
		return data
	case 0x08: // bool
		return p.readByte() != 0
	case 0x10: // int32
		return p.readI32()
	default:
		panic(fmt.Sprintf("unsupported bson type 0x%02x", ty))
	}
}

// ── run a compiled binary and capture stdout ──────────────────────────────

func runBinary(bin string, args ...string) []byte {
	var stdout bytes.Buffer
	cmd := exec.Command(bin, args...)
	cmd.Stdout = &stdout
	err := cmd.Run()
	if err != nil {
		if exitErr, ok := err.(*exec.ExitError); ok {
			code := exitErr.ExitCode()
			if code != 0 && code != 1 {
				// exit 2 (parse/usage error) or 3 (runtime error)
				fmt.Fprintf(os.Stderr, "error: %s exit %d: %s\n", bin, code, string(exitErr.Stderr))
				os.Exit(1)
			}
			// exit 1 = solutions found (the wire contract); stdout has the result
		} else {
			fmt.Fprintf(os.Stderr, "error running %s: %v\n", bin, err)
			os.Exit(1)
		}
	}
	return stdout.Bytes()
}

// ── fetch the atom map (--atoms --format bson) ────────────────────────────

func fetchAtoms(bin string) []string {
	raw := runBinary(bin, "--atoms", "--format", "bson")
	p := &bsonParser{b: raw}
	doc := p.parseDoc()
	arr, ok := doc["atoms"].([]any)
	if !ok {
		panic("atom map has no 'atoms' array")
	}
	names := make([]string, len(arr))
	for i, v := range arr {
		names[i] = v.(string)
	}
	return names
}

// ── decode a TermBuf BinData into a readable string ───────────────────────
//
// Cell ABI (plg-shared::cell): tag = word & 7, payload = word >> 3.
//   ATOM=1 (id → atoms[id])  INT=2 (i61 immediate)  STR=3 (functor+args)
//   LST=4 (head+tail)        FLT=5 (f64 bits)       BIG=6 (i64)
//   REF=0 (unbound)          cycles cut to "_"

const (
	tagREF  = 0
	tagATOM = 1
	tagINT  = 2
	tagSTR  = 3
	tagLST  = 4
	tagFLT  = 5
	tagBIG  = 6
)

func decodeTerm(data []byte, atoms []string) string {
	p := &bsonParser{b: data}
	ver := p.readByte()
	if ver != 1 {
		return fmt.Sprintf("?(termbuf version %d)", ver)
	}
	cellCount := uint32(p.readI32())
	p.i = 5 // skip version(1) + count(4)
	root := p.readU64()
	cells := make([]uint64, cellCount)
	for i := range cells {
		cells[i] = p.readU64()
	}
	return renderWord(root, cells, atoms, map[int]bool{})
}

func renderWord(w uint64, cells []uint64, atoms []string, visiting map[int]bool) string {
	tag := w & 7
	payload := w >> 3
	switch tag {
	case tagATOM:
		id := int(payload)
		if id < len(atoms) {
			name := atoms[id]
			if name == "[]" {
				return "[]"
			}
			return name
		}
		return fmt.Sprintf("?atom(%d)", id)
	case tagINT:
		v := int64(w) >> 3 // arithmetic shift (sign-preserving)
		return fmt.Sprintf("%d", v)
	case tagFLT:
		idx := int(payload)
		if idx < len(cells) {
			return fmt.Sprintf("%g", math.Float64frombits(cells[idx]))
		}
		return "?float"
	case tagBIG:
		idx := int(payload)
		if idx < len(cells) {
			return fmt.Sprintf("%d", int64(cells[idx]))
		}
		return "?bigint"
	case tagSTR:
		idx := int(payload)
		if visiting[idx] {
			return "_" // cycle cut
		}
		visiting[idx] = true
		header := cells[idx]
		functorID := int(header >> 32)
		arity := int(header & 0xFFFFFFFF)
		functor := "?"
		if functorID < len(atoms) {
			functor = atoms[functorID]
		}
		args := ""
		for k := 0; k < arity; k++ {
			if k > 0 {
				args += ", "
			}
			args += renderWord(cells[idx+1+k], cells, atoms, visiting)
		}
		delete(visiting, idx)
		if arity == 0 {
			return functor
		}
		return fmt.Sprintf("%s(%s)", functor, args)
	case tagLST:
		elems := ""
		cur := w
		first := true
		for {
			if cur&7 != tagLST {
				break
			}
			ci := int(cur >> 3)
			if visiting[ci] {
				return "[" + elems + "|_]" // cycle
			}
			visiting[ci] = true
			if !first {
				elems += ", "
			}
			elems += renderWord(cells[ci], cells, atoms, visiting)
			first = false
			cur = cells[ci+1] // tail
			delete(visiting, ci)
		}
		// cur is the tail: nil atom → proper list, else improper.
		if cur&7 == tagATOM {
			id := int(cur >> 3)
			if id < len(atoms) && atoms[id] == "[]" {
				return "[" + elems + "]"
			}
		}
		return "[" + elems + "|" + renderWord(cur, cells, atoms, visiting) + "]"
	case tagREF:
		return "_" // unbound
	default:
		return fmt.Sprintf("?tag(%d)", tag)
	}
}

// ── main: fetch atoms, run query, decode, print ───────────────────────────

func main() {
	if len(os.Args) < 3 {
		fmt.Fprintf(os.Stderr, "usage: go run . <compiled-binary> <query>\n")
		os.Exit(1)
	}
	bin := os.Args[1]
	query := os.Args[2]

	// 1. Fetch the atom map (one-time; program atoms only).
	atoms := fetchAtoms(bin)
	fmt.Fprintf(os.Stderr, "fetched %d atoms\n", len(atoms))

	// 2. Run the query as bson (--atoms embeds the map in the result too,
	//    but we already have it from step 1 — this just demonstrates both paths).
	raw := runBinary(bin, "--query", query, "--format", "bson")
	p := &bsonParser{b: raw}
	doc := p.parseDoc()

	count := doc["count"].(int32)
	exhausted := doc["exhausted"].(bool)
	solutions := doc["solutions"].([]any)

	fmt.Fprintf(os.Stderr, "count=%d exhausted=%v\n", count, exhausted)
	for _, sol := range solutions {
		solMap := sol.(map[string]any)
		for name, val := range solMap {
			data := val.([]byte) // BinData TermBuf
			fmt.Printf("%s = %s\n", name, decodeTerm(data, atoms))
		}
	}
}
