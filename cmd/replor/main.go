// Command replor is a read-only terminal explorer for local files,
// documents and git changes.
package main

import (
	"flag"
	"fmt"
	"io"
	"os"
	"runtime/debug"
	"strings"

	tea "charm.land/bubbletea/v2"

	"github.com/halkn/replor/internal/app"
	"github.com/halkn/replor/internal/preview"
	"github.com/halkn/replor/internal/workspace"
)

// version is set with -ldflags "-X main.version=..." for releases.
var version = ""

func main() {
	os.Exit(run(os.Args[1:], os.Stdout, os.Stderr))
}

func run(args []string, stdout, stderr io.Writer) int {
	fs := flag.NewFlagSet("replor", flag.ContinueOnError)
	fs.SetOutput(stderr)
	fs.Usage = func() {
		fmt.Fprintln(stderr, "usage: replor [flags] [PATH]")
		fmt.Fprintln(stderr, "\nOpens PATH (a directory, or a file to select; default .) read-only.")
		fs.PrintDefaults()
	}
	images := fs.String("images", "auto", "inline images: auto, kitty, halfblocks or off")
	noWatch := fs.Bool("no-watch", false, "do not reload when files or git state change (r still reloads)")
	base := fs.String("base", "", "branch or commit Branch Changes compare with (default: what refs/remotes/origin/HEAD points to)")
	showVersion := fs.Bool("version", false, "print the version and exit")
	if err := fs.Parse(args); err != nil {
		return 2
	}
	if *showVersion {
		fmt.Fprintln(stdout, "replor", buildVersion())
		return 0
	}
	if fs.NArg() > 1 {
		fs.Usage()
		return 2
	}
	mode, ok := imageMode(*images)
	if !ok {
		fmt.Fprintf(stderr, "replor: unknown -images value %q\n", *images)
		return 2
	}

	// git would take it for an option.
	if strings.HasPrefix(*base, "-") {
		fmt.Fprintf(stderr, "replor: invalid -base value %q\n", *base)
		return 2
	}

	ws, err := workspace.Open(fs.Arg(0))
	if err != nil {
		fmt.Fprintln(stderr, "replor:", err)
		return 1
	}
	defer ws.Close()

	if _, err := tea.NewProgram(app.New(ws, app.Options{Images: mode, Watch: !*noWatch, Base: *base})).Run(); err != nil {
		fmt.Fprintln(stderr, "replor:", err)
		return 1
	}
	return 0
}

func imageMode(s string) (preview.ImageMode, bool) {
	switch s {
	case "auto":
		return preview.DetectImageMode(os.Getenv), true
	case "kitty":
		return preview.ImageKitty, true
	case "halfblocks":
		return preview.ImageHalfblocks, true
	case "off":
		return preview.ImageOff, true
	}
	return 0, false
}

func buildVersion() string {
	if version != "" {
		return version
	}
	if info, ok := debug.ReadBuildInfo(); ok && info.Main.Version != "" {
		return info.Main.Version
	}
	return "devel"
}
