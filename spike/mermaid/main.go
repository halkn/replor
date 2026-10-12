// Command mermaid is the P0-04 spike: render testdata/mermaid/*.mmd to SVG and
// PNG with go-mermaid and report timing and failure modes.
//
//	go run ./spike/mermaid -in testdata/mermaid -out .scratch/mermaid
package main

import (
	"errors"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	mermaid "github.com/zkrebbekx/go-mermaid"
	"github.com/zkrebbekx/go-mermaid/raster"
)

func main() {
	in := flag.String("in", "testdata/mermaid", "input directory")
	out := flag.String("out", ".scratch/mermaid", "output directory")
	scale := flag.Float64("scale", 2, "PNG scale")
	flag.Parse()

	if err := os.MkdirAll(*out, 0o755); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	files, _ := filepath.Glob(filepath.Join(*in, "*.mmd"))
	for _, f := range files {
		src, err := os.ReadFile(f)
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			continue
		}
		name := strings.TrimSuffix(filepath.Base(f), ".mmd")

		t0 := time.Now()
		svg, err := mermaid.Render(string(src))
		svgDur := time.Since(t0)
		if err != nil {
			fmt.Printf("%-14s svg  FAIL %-14s %v\n", name, kind(err), err)
			continue
		}
		_ = os.WriteFile(filepath.Join(*out, name+".svg"), svg, 0o644)

		t0 = time.Now()
		png, err := raster.RasterizeSVG(svg, *scale)
		pngDur := time.Since(t0)
		if err != nil {
			fmt.Printf("%-14s png  FAIL %v\n", name, err)
			continue
		}
		_ = os.WriteFile(filepath.Join(*out, name+".png"), png, 0o644)
		fmt.Printf("%-14s ok   svg=%6dB %8s  png=%7dB %8s  ja-in-svg=%v\n",
			name, len(svg), svgDur.Round(time.Microsecond), len(png), pngDur.Round(time.Microsecond), hasJapanese(string(svg)))
	}
}

func kind(err error) string {
	for _, k := range []struct {
		e error
		n string
	}{{mermaid.ErrParse, "ErrParse"}, {mermaid.ErrUnsupported, "ErrUnsupported"}, {mermaid.ErrLayout, "ErrLayout"}, {mermaid.ErrRender, "ErrRender"}} {
		if errors.Is(err, k.e) {
			return k.n
		}
	}
	return "other"
}

func hasJapanese(s string) bool {
	for _, r := range s {
		if r >= 0x3040 && r <= 0x30ff || r >= 0x4e00 && r <= 0x9fff {
			return true
		}
	}
	return false
}
