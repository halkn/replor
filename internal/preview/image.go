package preview

import (
	"encoding/base64"
	"fmt"
	"image"
	"strings"

	"github.com/charmbracelet/x/mosaic"
)

// ImageMode is how images are put on screen.
type ImageMode int

const (
	ImageOff        ImageMode = iota // show the source / alt text only
	ImageHalfblocks                  // colored ▀ cells; works on any truecolor terminal
	ImageKitty                       // Kitty graphics with Unicode placeholders
)

func (m ImageMode) String() string {
	return [...]string{"off", "halfblocks", "kitty"}[m]
}

// DetectImageMode decides from the environment only. Querying the terminal
// would race with Bubble Tea's input reader, so detection happens before the
// program starts and without reading stdin.
func DetectImageMode(getenv func(string) string) ImageMode {
	term, prog := getenv("TERM"), getenv("TERM_PROGRAM")
	switch {
	case getenv("TMUX") != "":
		// tmux needs passthrough and placeholder support; not covered in v1.
		return ImageHalfblocks
	case getenv("KITTY_WINDOW_ID") != "", term == "xterm-kitty", term == "xterm-ghostty", strings.EqualFold(prog, "ghostty"):
		return ImageKitty
	case getenv("COLORTERM") == "truecolor" || getenv("COLORTERM") == "24bit" || getenv("WT_SESSION") != "":
		return ImageHalfblocks
	}
	return ImageOff
}

// FitCells returns the cell box for an image no wider than maxCols, assuming
// cells are twice as tall as wide.
func FitCells(b image.Rectangle, maxCols, maxRows int) (cols, rows int) {
	w, h := b.Dx(), b.Dy()
	if w <= 0 || h <= 0 || maxCols <= 0 {
		return 0, 0
	}
	cols = maxCols
	rows = max(1, (h*cols+w-1)/w/2)
	if maxRows > 0 && rows > maxRows {
		rows = maxRows
		cols = max(1, w*rows*2/h)
	}
	return cols, rows
}

// Halfblocks renders img as text lines of exactly cols cells.
func Halfblocks(img image.Image, cols, rows int) []string {
	// mosaic sizes are in pixels and it samples 2×2 pixel blocks per cell.
	m := mosaic.New().Width(cols * 2).Height(rows * 2)
	return strings.Split(strings.TrimRight(m.Render(img), "\n"), "\n")
}

const kittyPlaceholder = "\U0010EEEE"

// KittyTransmit returns the escape sequence that uploads a PNG and creates a
// virtual placement of cols×rows cells. It must be written outside the cell
// renderer (tea.Raw); the image only appears where KittyPlaceholders cells are.
func KittyTransmit(png []byte, id uint32, cols, rows int) string {
	data := base64.StdEncoding.EncodeToString(png)
	var b strings.Builder
	const chunk = 4096
	for i := 0; i < len(data); i += chunk {
		end := min(i+chunk, len(data))
		more := 0
		if end < len(data) {
			more = 1
		}
		if i == 0 {
			fmt.Fprintf(&b, "\x1b_Ga=T,f=100,U=1,i=%d,c=%d,r=%d,q=2,m=%d;%s\x1b\\", id, cols, rows, more, data[i:end])
		} else {
			fmt.Fprintf(&b, "\x1b_Gm=%d;%s\x1b\\", more, data[i:end])
		}
	}
	return b.String()
}

// KittyDelete frees the image and its placements in the terminal.
func KittyDelete(id uint32) string { return fmt.Sprintf("\x1b_Ga=d,d=I,i=%d,q=2\x1b\\", id) }

// KittyPlaceholders returns one line per row. Each cell is U+10EEEE with row
// and column diacritics; the foreground color carries the low 24 bits of the
// image ID (IDs are kept below 1<<24 so no third diacritic is needed).
func KittyPlaceholders(id uint32, cols, rows int) []string {
	color := fmt.Sprintf("\x1b[38;2;%d;%d;%dm", id>>16&0xff, id>>8&0xff, id&0xff)
	lines := make([]string, rows)
	for r := range rows {
		var b strings.Builder
		b.WriteString(color)
		for c := range cols {
			b.WriteString(kittyPlaceholder)
			b.WriteRune(kittyDiacritics[r])
			b.WriteRune(kittyDiacritics[c])
		}
		b.WriteString("\x1b[39m")
		lines[r] = b.String()
	}
	return lines
}

// MaxKittyCells bounds rows and columns to the diacritics table.
var MaxKittyCells = len(kittyDiacritics)

// kittyDiacritics is the start of the row/column encoding table from the
// Kitty graphics protocol (rowcolumn-diacritics.txt); 64 entries cover any
// image up to 64×64 cells, larger images are scaled down to fit.
var kittyDiacritics = []rune{
	0x0305, 0x030D, 0x030E, 0x0310, 0x0312, 0x033D, 0x033E, 0x033F, 0x0346, 0x034A,
	0x034B, 0x034C, 0x0350, 0x0351, 0x0352, 0x0357, 0x035B, 0x0363, 0x0364, 0x0365,
	0x0366, 0x0367, 0x0368, 0x0369, 0x036A, 0x036B, 0x036C, 0x036D, 0x036E, 0x036F,
	0x0483, 0x0484, 0x0485, 0x0486, 0x0487, 0x0592, 0x0593, 0x0594, 0x0595, 0x0597,
	0x0598, 0x0599, 0x059C, 0x059D, 0x059E, 0x059F, 0x05A0, 0x05A1, 0x05A8, 0x05A9,
	0x05AB, 0x05AC, 0x05AF, 0x05C4, 0x0610, 0x0611, 0x0612, 0x0613, 0x0614, 0x0615,
	0x0616, 0x0617, 0x0653, 0x0654,
}
