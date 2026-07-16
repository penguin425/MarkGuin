# MarkGuin

MarkGuin is a focused native Markdown editor written in Rust. It combines the source-aware editing strengths of developer IDEs with the uncluttered reading and writing experience of dedicated Markdown apps.

## Highlights

- Write, split, and read modes
- Optional bidirectional relative scroll synchronization in split mode
- Live CommonMark/GFM preview
- Markdown-aware source coloring for headings, lists, quotes, tables, and code fences
- Styled preview for emphasis, strong text, links, strikethrough, and inline code
- Native MathJax SVG rendering for inline `$…$` and display `$$…$$` TeX equations
- Pure-Rust Mermaid and common PlantUML sequence-diagram SVG previews for fenced blocks
- Document outline generated from headings
- Selection-aware Markdown table, task, link, quote, code, and formatting helpers
- Configurable table builder supporting 1–12 columns and up to 50 body rows
- Unicode-aware table formatter with left, center, and right alignment preservation
- Insert or update a hierarchical table of contents, including Unicode and duplicate headings
- Drag-and-drop images and files to insert relative Markdown links automatically
- Local, relative, and remote image previews with alt-text tooltips
- Clickable web, file, and relative-document links in preview
- In-document search with match count, previous/next navigation, replace, and replace-all
- Clickable document outline for jumping to headings
- Focus mode for distraction-free writing
- Native open/save dialogs and unsaved-state indicator
- Save/discard/cancel protection before replacing or closing a changed document
- Automatic session persistence and crash recovery for unsaved documents
- Detects files changed or removed by another application, with safe reload/keep choices
- Restores the last document, view mode, outline, and focus-mode state
- Export to a standalone responsive HTML document with embedded light/dark styling
- HTML exports embed rendered equation SVGs and need no MathJax CDN or JavaScript
- Diagram exports are embedded SVGs with no Node.js, Java, server, or browser dependency
- Word and line counts
- Dark interface designed for long writing sessions

## Run

```sh
cargo run --release
```

On Debian/Ubuntu desktop systems, the native window backend requires the usual X11 keyboard runtime library:

```sh
sudo apt install libxkbcommon-x11-0
```

You can also open a file directly:

```sh
cargo run --release -- notes.md
```

## Keyboard shortcuts

| Action | macOS | Windows/Linux |
| --- | --- | --- |
| New | Cmd+N | Ctrl+N |
| Open | Cmd+O | Ctrl+O |
| Save | Cmd+S | Ctrl+S |
| Save as | Cmd+Shift+S | Ctrl+Shift+S |
| Find | Cmd+F | Ctrl+F |
| Find and replace | Cmd+H | Ctrl+H |
| Bold | Cmd+B | Ctrl+B |
| Italic | Cmd+I | Ctrl+I |
| Link | Cmd+K | Ctrl+K |
| Format tables | Cmd+Alt+L | Ctrl+Alt+L |

## Design direction

MarkGuin keeps Markdown portable: files remain ordinary UTF-8 text with no private database or workspace format. The application is intentionally native and focused, so documents and exports do not depend on a private workspace format or hosted rendering service.

## Development

```sh
cargo fmt --all -- --check
cargo test
cargo clippy --all-targets -- -D warnings
```

## Releases and licensing

MarkGuin source code is licensed under the [MIT License](LICENSE). A push or pull request runs the cross-platform release workflow; pushing a tag such as `v0.1.0` additionally creates a GitHub Release containing Linux, Windows, macOS Apple Silicon, and macOS Intel packages.

Every binary package includes the MarkGuin license, a generated dependency-license report, MathJax's bundled notices, and the MPL source-availability statement. Release checksums are published as `SHA256SUMS.txt`.
