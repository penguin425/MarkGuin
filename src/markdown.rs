use eframe::egui::{
    self, Color32, FontFamily, FontId, RichText, Stroke, TextFormat, Ui, text::LayoutJob,
};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd, html};
use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};
use unicode_width::UnicodeWidthStr;

use mathjax_svg_rs::{HorizontalAlign, MathJax};

const EXPORT_CSS: &str = r#"
:root { color-scheme: light dark; --bg:#ffffff; --fg:#24292f; --muted:#57606a; --border:#d0d7de; --code:#f6f8fa; --accent:#0969da; }
@media (prefers-color-scheme:dark) { :root { --bg:#0d1117; --fg:#e6edf3; --muted:#8b949e; --border:#30363d; --code:#161b22; --accent:#58a6ff; } }
* { box-sizing:border-box; }
body { margin:0; background:var(--bg); color:var(--fg); font:16px/1.65 system-ui,-apple-system,"Segoe UI",sans-serif; }
main { max-width:860px; margin:0 auto; padding:48px 32px 80px; }
h1,h2,h3,h4,h5,h6 { line-height:1.25; margin:1.5em 0 .55em; }
h1,h2 { padding-bottom:.3em; border-bottom:1px solid var(--border); }
a { color:var(--accent); } img { max-width:100%; height:auto; }
blockquote { margin:1em 0; padding:.2em 1em; color:var(--muted); border-left:4px solid var(--border); }
code { padding:.16em .35em; border-radius:5px; background:var(--code); font-family:ui-monospace,SFMono-Regular,Consolas,monospace; }
pre { overflow:auto; padding:16px; border:1px solid var(--border); border-radius:8px; background:var(--code); }
pre code { padding:0; background:none; } table { width:100%; border-collapse:collapse; }
th,td { padding:7px 12px; border:1px solid var(--border); } th { background:var(--code); text-align:left; }
hr { height:1px; border:0; background:var(--border); } input[type=checkbox] { margin-right:.45em; }
.math-display { display:block; overflow:auto; margin:1.2em 0; text-align:center; }
.math-inline svg { display:inline-block; vertical-align:middle; }
.diagram { overflow:auto; margin:1.2em 0; text-align:center; }
.diagram svg { max-width:100%; height:auto; }
.diagram-error { color:#cf222e; }
"#;

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
enum DiagramLanguage {
    Mermaid,
    PlantUml,
}

impl DiagramLanguage {
    fn css_name(self) -> &'static str {
        match self {
            Self::Mermaid => "mermaid",
            Self::PlantUml => "plantuml",
        }
    }
}

fn diagram_language(language: &str) -> Option<DiagramLanguage> {
    match language
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "mermaid" | "mmd" => Some(DiagramLanguage::Mermaid),
        "plantuml" | "puml" | "uml" => Some(DiagramLanguage::PlantUml),
        _ => None,
    }
}

fn render_diagram_svg(language: DiagramLanguage, source: &str) -> Result<String, String> {
    static RENDER_LOCK: Mutex<()> = Mutex::new(());
    let _guard = RENDER_LOCK.lock().expect("diagram renderer lock poisoned");
    let output = match language {
        DiagramLanguage::Mermaid => {
            mermaid_rs_renderer::render(source).map_err(|error| error.to_string())?
        }
        DiagramLanguage::PlantUml => {
            let mermaid = plantuml_sequence_to_mermaid(source)?;
            mermaid_rs_renderer::render(&mermaid).map_err(|error| error.to_string())?
        }
    };
    extract_svg(&output)
}

fn plantuml_sequence_to_mermaid(source: &str) -> Result<String, String> {
    let mut output = String::from("sequenceDiagram\n");
    let mut messages = 0;
    for raw_line in source.lines() {
        let line = raw_line.trim();
        if line.is_empty()
            || line.starts_with('\'')
            || line.starts_with("@start")
            || line.starts_with("@end")
            || line.starts_with("skinparam")
        {
            continue;
        }
        if line.starts_with("participant ") || line.starts_with("actor ") {
            output.push_str("    ");
            output.push_str(line);
            output.push('\n');
            continue;
        }
        if let Some(translated) = translate_plantuml_message(line) {
            output.push_str("    ");
            output.push_str(&translated);
            output.push('\n');
            messages += 1;
            continue;
        }
        if [
            "activate ",
            "deactivate ",
            "note ",
            "alt ",
            "opt ",
            "loop ",
            "else",
            "end",
        ]
        .iter()
        .any(|prefix| line.starts_with(prefix))
        {
            output.push_str("    ");
            output.push_str(line);
            output.push('\n');
        }
    }
    if messages == 0 {
        Err("PlantUML preview currently supports sequence diagrams with message arrows".into())
    } else {
        Ok(output)
    }
}

fn translate_plantuml_message(line: &str) -> Option<String> {
    for (arrow, replacement) in [
        ("-->", "-->>"),
        ("->", "->>"),
        ("<--", "<<--"),
        ("<-", "<<-"),
    ] {
        if let Some(position) = line.find(arrow) {
            let left = line[..position].trim();
            let remainder = &line[position + arrow.len()..];
            let right_end = remainder.find(':').unwrap_or(remainder.len());
            let right = remainder[..right_end].trim();
            if left.is_empty() || right.is_empty() {
                return None;
            }
            let message = remainder[right_end..].trim();
            return Some(format!("{left}{replacement}{right}{message}"));
        }
    }
    None
}

fn extract_svg(output: &str) -> Result<String, String> {
    let start = output
        .find("<svg")
        .ok_or_else(|| "Renderer output did not contain SVG".to_owned())?;
    let end = output
        .rfind("</svg>")
        .map(|position| position + "</svg>".len())
        .ok_or_else(|| "Renderer output had an incomplete SVG element".to_owned())?;
    Ok(output[start..end]
        .replace("currentColor", "#dce3ee")
        .replace("<script", "<discarded-script"))
}

fn load_svg_with_fonts(svg: &[u8]) -> Result<egui::ColorImage, String> {
    use resvg::{
        tiny_skia::Pixmap,
        usvg::{Options, Tree, TreeParsing, TreeTextToPath},
    };

    static FONT_DATABASE: OnceLock<usvg::fontdb::Database> = OnceLock::new();
    let fonts = FONT_DATABASE.get_or_init(|| {
        let mut database = usvg::fontdb::Database::new();
        database.load_system_fonts();
        database.set_sans_serif_family("DejaVu Sans");
        database.set_serif_family("DejaVu Serif");
        database.set_monospace_family("DejaVu Sans Mono");
        database
    });
    let mut tree = Tree::from_data(svg, &Options::default()).map_err(|error| error.to_string())?;
    tree.convert_text(fonts);
    let size = tree.size.to_int_size();
    let mut pixmap =
        Pixmap::new(size.width(), size.height()).ok_or_else(|| "Invalid SVG size".to_owned())?;
    resvg::Tree::from_usvg(&tree).render(Default::default(), &mut pixmap.as_mut());
    Ok(egui::ColorImage::from_rgba_unmultiplied(
        [size.width() as usize, size.height() as usize],
        pixmap.data(),
    ))
}

#[derive(Clone)]
enum DiagramRender {
    Pending,
    Ready(String),
    Failed(String),
}

fn diagram_cache() -> &'static Mutex<HashMap<String, DiagramRender>> {
    static CACHE: OnceLock<Mutex<HashMap<String, DiagramRender>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn diagram_key(language: DiagramLanguage, source: &str) -> String {
    format!("{}:{source}", language.css_name())
}

fn request_diagram_svg(
    ctx: &egui::Context,
    language: DiagramLanguage,
    source: &str,
) -> Option<Result<String, String>> {
    let key = diagram_key(language, source);
    {
        let mut cache = diagram_cache().lock().expect("diagram cache poisoned");
        match cache.get(&key) {
            Some(DiagramRender::Ready(svg)) => return Some(Ok(svg.clone())),
            Some(DiagramRender::Failed(error)) => return Some(Err(error.clone())),
            Some(DiagramRender::Pending) => return None,
            None => {
                cache.insert(key.clone(), DiagramRender::Pending);
            }
        }
    }
    let source = source.to_owned();
    let ctx = ctx.clone();
    std::thread::Builder::new()
        .name(format!("markguin-{}-render", language.css_name()))
        .spawn(move || {
            let entry = match render_diagram_svg(language, &source) {
                Ok(svg) => DiagramRender::Ready(svg),
                Err(error) => DiagramRender::Failed(error),
            };
            diagram_cache()
                .lock()
                .expect("diagram cache poisoned")
                .insert(key, entry);
            ctx.request_repaint();
        })
        .expect("failed to spawn diagram rendering thread");
    None
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heading {
    pub level: usize,
    pub title: String,
    pub line: usize,
}

pub fn headings(source: &str) -> Vec<Heading> {
    let mut result = Vec::new();
    let mut current = None::<(usize, String, usize)>;
    for (event, range) in Parser::new_ext(source, Options::all()).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                current = Some((heading_level(level), String::new(), range.start));
            }
            Event::Text(text) | Event::Code(text) if current.is_some() => {
                current.as_mut().unwrap().1.push_str(&text);
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, title, offset)) = current.take() {
                    let title = title.trim().to_owned();
                    if !title.is_empty() {
                        result.push(Heading {
                            level,
                            title,
                            line: source[..offset]
                                .bytes()
                                .filter(|byte| *byte == b'\n')
                                .count()
                                + 1,
                        });
                    }
                }
            }
            _ => {}
        }
    }
    result
}

fn heading_level(level: HeadingLevel) -> usize {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

/// Creates a source-preserving layout job with lightweight Markdown syntax coloring.
pub fn highlight_source(source: &str) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut fenced = false;
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let color = if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            Color32::from_rgb(116, 190, 255)
        } else if fenced {
            Color32::from_rgb(196, 215, 190)
        } else if trimmed.starts_with('#') {
            Color32::from_rgb(130, 196, 255)
        } else if trimmed.starts_with('>') {
            Color32::from_rgb(157, 190, 173)
        } else if trimmed.starts_with("- ")
            || trimmed.starts_with("* ")
            || trimmed.starts_with("+ ")
        {
            Color32::from_rgb(222, 181, 112)
        } else if trimmed.starts_with('|') {
            Color32::from_rgb(201, 166, 222)
        } else {
            Color32::from_rgb(205, 210, 220)
        };
        job.append(
            line,
            0.0,
            TextFormat::simple(FontId::monospace(15.0), color),
        );
    }
    job
}

pub fn to_html_document(source: &str, title: &str) -> String {
    let mut body = String::new();
    html::push_html(&mut body, export_events(source).into_iter());
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n<title>{}</title>\n<style>{EXPORT_CSS}</style>\n</head>\n<body><main>\n{body}</main></body>\n</html>\n",
        escape_html(title)
    )
}

fn export_events(source: &str) -> Vec<Event<'static>> {
    let mut events = Vec::new();
    let mut diagram = None::<DiagramLanguage>;
    let mut diagram_source = String::new();
    for event in Parser::new_ext(source, Options::all()) {
        match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(language)))
                if diagram_language(&language).is_some() =>
            {
                diagram = diagram_language(&language);
                diagram_source.clear();
            }
            Event::Text(text) if diagram.is_some() => diagram_source.push_str(&text),
            Event::End(TagEnd::CodeBlock) if diagram.is_some() => {
                let language = diagram.take().unwrap();
                let rendered = match render_diagram_svg(language, &diagram_source) {
                    Ok(svg) => format!(
                        "<div class=\"diagram diagram-{}\">{svg}</div>",
                        language.css_name()
                    ),
                    Err(error) => format!(
                        "<pre class=\"diagram-error\"><code>{}</code>\n{}</pre>",
                        escape_html(&diagram_source),
                        escape_html(&error)
                    ),
                };
                events.push(Event::Html(rendered.into()));
            }
            _ if diagram.is_some() => {}
            Event::InlineMath(tex) => events.push(math_html(&tex, false)),
            Event::DisplayMath(tex) => events.push(math_html(&tex, true)),
            event => events.push(event.into_static()),
        }
    }
    events
}

fn math_html(tex: &str, display: bool) -> Event<'static> {
    let class = if display {
        "math-display"
    } else {
        "math-inline"
    };
    let html = match render_math_svg(tex, display) {
        Ok(svg) => format!("<span class=\"math {class}\">{svg}</span>"),
        Err(_) => format!("<code class=\"math {class}\">{}</code>", escape_html(tex)),
    };
    Event::Html(html.into())
}

#[derive(Clone)]
enum MathRender {
    Pending,
    Ready(String),
    Failed(String),
}

fn math_cache() -> &'static Mutex<HashMap<String, MathRender>> {
    static CACHE: OnceLock<Mutex<HashMap<String, MathRender>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn math_renderer() -> &'static MathJax {
    static RENDERER: OnceLock<MathJax> = OnceLock::new();
    RENDERER.get_or_init(MathJax::new)
}

fn math_key(tex: &str, display: bool) -> String {
    format!("{}:{tex}", if display { 'd' } else { 'i' })
}

fn render_math_svg_uncached(tex: &str, display: bool) -> Result<String, String> {
    let options = mathjax_svg_rs::Options {
        font_size: if display { 20.0 } else { 16.0 },
        horizontal_align: if display {
            HorizontalAlign::Center
        } else {
            HorizontalAlign::Left
        },
    };
    let output = math_renderer()
        .render_tex(tex, &options)
        .map(|svg| svg.replace("currentColor", "#dce3ee"))?;
    let start = output
        .find("<svg")
        .ok_or_else(|| "MathJax output did not contain SVG".to_owned())?;
    let end = output
        .rfind("</svg>")
        .map(|position| position + "</svg>".len())
        .ok_or_else(|| "MathJax output had an incomplete SVG element".to_owned())?;
    let svg = output[start..end].to_owned();
    let svg = remove_svg_attribute(svg, "style");
    let svg = convert_svg_ex_dimension(svg, "width", options.font_size)?;
    convert_svg_ex_dimension(svg, "height", options.font_size)
}

fn remove_svg_attribute(mut svg: String, attribute: &str) -> String {
    let marker = format!(" {attribute}=\"");
    if let Some(start) = svg.find(&marker)
        && let Some(relative_end) = svg[start + marker.len()..].find('"')
    {
        let end = start + marker.len() + relative_end + 1;
        svg.replace_range(start..end, "");
    }
    svg
}

fn convert_svg_ex_dimension(
    mut svg: String,
    attribute: &str,
    font_size: f64,
) -> Result<String, String> {
    let marker = format!("{attribute}=\"");
    let start = svg
        .find(&marker)
        .map(|position| position + marker.len())
        .ok_or_else(|| format!("MathJax SVG had no {attribute}"))?;
    let end = svg[start..]
        .find('"')
        .map(|position| start + position)
        .ok_or_else(|| format!("MathJax SVG had an invalid {attribute}"))?;
    let value = svg[start..end]
        .strip_suffix("ex")
        .ok_or_else(|| format!("MathJax SVG {attribute} was not measured in ex"))?
        .parse::<f64>()
        .map_err(|error| format!("MathJax SVG had an invalid {attribute}: {error}"))?;
    // MathJax's ex unit is approximately half its configured font size.
    svg.replace_range(start..end, &format!("{:.3}", value * font_size * 0.5));
    Ok(svg)
}

fn render_math_svg(tex: &str, display: bool) -> Result<String, String> {
    let key = math_key(tex, display);
    if let Some(cached) = math_cache()
        .lock()
        .expect("math cache poisoned")
        .get(&key)
        .cloned()
    {
        match cached {
            MathRender::Ready(svg) => return Ok(svg),
            MathRender::Failed(error) => return Err(error),
            MathRender::Pending => {}
        }
    }
    let result = render_math_svg_uncached(tex, display);
    let entry = match &result {
        Ok(svg) => MathRender::Ready(svg.clone()),
        Err(error) => MathRender::Failed(error.clone()),
    };
    math_cache()
        .lock()
        .expect("math cache poisoned")
        .insert(key, entry);
    result
}

fn request_math_svg(
    ctx: &egui::Context,
    tex: &str,
    display: bool,
) -> Option<Result<String, String>> {
    let key = math_key(tex, display);
    {
        let mut cache = math_cache().lock().expect("math cache poisoned");
        match cache.get(&key) {
            Some(MathRender::Ready(svg)) => return Some(Ok(svg.clone())),
            Some(MathRender::Failed(error)) => return Some(Err(error.clone())),
            Some(MathRender::Pending) => return None,
            None => {
                cache.insert(key.clone(), MathRender::Pending);
            }
        }
    }
    let tex = tex.to_owned();
    let ctx = ctx.clone();
    std::thread::Builder::new()
        .name("markguin-math-render".into())
        .spawn(move || {
            let result = render_math_svg_uncached(&tex, display);
            let entry = match result {
                Ok(svg) => MathRender::Ready(svg),
                Err(error) => MathRender::Failed(error),
            };
            math_cache()
                .lock()
                .expect("math cache poisoned")
                .insert(key, entry);
            ctx.request_repaint();
        })
        .expect("failed to spawn math rendering thread");
    None
}

pub fn upsert_table_of_contents(source: &str) -> (String, bool) {
    const START: &str = "<!-- TOC -->";
    const END: &str = "<!-- /TOC -->";
    let block = format!("{START}\n{}{END}", table_of_contents(source));

    if let Some(start) = source.find(START)
        && let Some(relative_end) = source[start + START.len()..].find(END)
    {
        let end = start + START.len() + relative_end + END.len();
        let mut result = source.to_owned();
        result.replace_range(start..end, &block);
        return (result, true);
    }

    let insertion = source
        .lines()
        .next()
        .filter(|line| line.trim_start().starts_with("# "))
        .and_then(|_| source.find('\n').map(|position| position + 1))
        .unwrap_or(0);
    let mut result = source.to_owned();
    let surrounded = if insertion == 0 {
        format!("{block}\n\n")
    } else {
        format!("\n{block}\n")
    };
    result.insert_str(insertion, &surrounded);
    (result, false)
}

fn table_of_contents(source: &str) -> String {
    let mut anchors = HashMap::<String, usize>::new();
    headings(source)
        .into_iter()
        .map(|heading| {
            let base = heading_anchor(&heading.title);
            let occurrence = anchors.entry(base.clone()).or_default();
            let anchor = if *occurrence == 0 {
                base
            } else {
                format!("{base}-{occurrence}")
            };
            *occurrence += 1;
            let label = heading.title.replace('[', "\\[").replace(']', "\\]");
            format!(
                "{}- [{label}](#{anchor})\n",
                "  ".repeat(heading.level.saturating_sub(1))
            )
        })
        .collect()
}

fn heading_anchor(title: &str) -> String {
    let mut result = String::new();
    let mut pending_dash = false;
    for character in title.chars().flat_map(char::to_lowercase) {
        if character.is_alphanumeric() || character == '_' || character == '-' {
            if pending_dash && !result.is_empty() && !result.ends_with('-') {
                result.push('-');
            }
            pending_dash = false;
            result.push(character);
        } else if character.is_whitespace() {
            pending_dash = true;
        }
    }
    let result = result.trim_matches('-');
    if result.is_empty() {
        "section".to_owned()
    } else {
        result.to_owned()
    }
}

pub fn format_tables(source: &str) -> (String, usize) {
    let lines = source.lines().collect::<Vec<_>>();
    let mut output = Vec::<String>::with_capacity(lines.len());
    let mut index = 0;
    let mut formatted = 0;
    let mut fenced = false;

    while index < lines.len() {
        let trimmed = lines[index].trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            output.push(lines[index].to_owned());
            index += 1;
            continue;
        }
        if fenced {
            output.push(lines[index].to_owned());
            index += 1;
            continue;
        }
        let Some(header) = parse_table_row(lines[index]) else {
            output.push(lines[index].to_owned());
            index += 1;
            continue;
        };
        let Some(separator) = lines.get(index + 1).and_then(|line| parse_table_row(line)) else {
            output.push(lines[index].to_owned());
            index += 1;
            continue;
        };
        if header.len() != separator.len()
            || !separator.iter().all(|cell| table_alignment(cell).is_some())
        {
            output.push(lines[index].to_owned());
            index += 1;
            continue;
        }

        let mut rows = vec![header];
        let alignments = separator
            .iter()
            .map(|cell| table_alignment(cell).unwrap())
            .collect::<Vec<_>>();
        index += 2;
        while let Some(row) = lines.get(index).and_then(|line| parse_table_row(line)) {
            if row.len() != alignments.len() {
                break;
            }
            rows.push(row);
            index += 1;
        }

        let mut widths = vec![3usize; alignments.len()];
        for row in &rows {
            for (column, cell) in row.iter().enumerate() {
                widths[column] = widths[column].max(UnicodeWidthStr::width(cell.trim()));
            }
        }
        for (width, alignment) in widths.iter_mut().zip(&alignments) {
            *width = (*width).max(if *alignment == TableAlignment::Center {
                5
            } else {
                4
            });
        }

        output.push(format_table_row(&rows[0], &widths));
        output.push(format_separator_row(&alignments, &widths));
        for row in rows.iter().skip(1) {
            output.push(format_table_row(row, &widths));
        }
        formatted += 1;
    }

    let mut result = output.join("\n");
    if source.ends_with('\n') {
        result.push('\n');
    }
    (result, formatted)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TableAlignment {
    None,
    Left,
    Center,
    Right,
}

fn table_alignment(cell: &str) -> Option<TableAlignment> {
    let cell = cell.trim();
    let left = cell.starts_with(':');
    let right = cell.ends_with(':');
    let dashes = cell.trim_matches(':');
    if dashes.len() < 3 || !dashes.bytes().all(|byte| byte == b'-') {
        return None;
    }
    Some(match (left, right) {
        (true, true) => TableAlignment::Center,
        (true, false) => TableAlignment::Left,
        (false, true) => TableAlignment::Right,
        (false, false) => TableAlignment::None,
    })
}

fn parse_table_row(line: &str) -> Option<Vec<String>> {
    if !line.contains('|') {
        return None;
    }
    let mut cells = Vec::new();
    let mut cell = String::new();
    let mut escaped = false;
    for character in line.chars() {
        if escaped {
            cell.push(character);
            escaped = false;
        } else if character == '\\' {
            cell.push(character);
            escaped = true;
        } else if character == '|' {
            cells.push(std::mem::take(&mut cell));
        } else {
            cell.push(character);
        }
    }
    cells.push(cell);
    if cells.first().is_some_and(|cell| cell.trim().is_empty()) {
        cells.remove(0);
    }
    if cells.last().is_some_and(|cell| cell.trim().is_empty()) {
        cells.pop();
    }
    (!cells.is_empty()).then_some(cells)
}

fn format_table_row(cells: &[String], widths: &[usize]) -> String {
    let cells = cells
        .iter()
        .zip(widths)
        .map(|(cell, width)| {
            let cell = cell.trim();
            let padding = width.saturating_sub(UnicodeWidthStr::width(cell));
            format!("{cell}{}", " ".repeat(padding))
        })
        .collect::<Vec<_>>()
        .join(" | ");
    format!("| {cells} |")
}

fn format_separator_row(alignments: &[TableAlignment], widths: &[usize]) -> String {
    let cells = alignments
        .iter()
        .zip(widths)
        .map(|(alignment, width)| match alignment {
            TableAlignment::None => "-".repeat(*width),
            TableAlignment::Left => format!(":{}", "-".repeat(width - 1)),
            TableAlignment::Right => format!("{}:", "-".repeat(width - 1)),
            TableAlignment::Center => format!(":{}:", "-".repeat(width - 2)),
        })
        .collect::<Vec<_>>()
        .join(" | ");
    format!("| {cells} |")
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[derive(Default)]
struct InlineStyle {
    emphasis: usize,
    strong: usize,
    strike: usize,
    link: usize,
}

impl InlineStyle {
    fn format(&self, heading: Option<HeadingLevel>, code: bool) -> TextFormat {
        let size = match heading {
            Some(HeadingLevel::H1) => 30.0,
            Some(HeadingLevel::H2) => 24.0,
            Some(HeadingLevel::H3) => 20.0,
            Some(_) => 17.0,
            None => 15.0,
        };
        let mut format = TextFormat::simple(
            FontId::new(
                size,
                if code {
                    FontFamily::Monospace
                } else {
                    FontFamily::Proportional
                },
            ),
            if self.link > 0 {
                Color32::from_rgb(112, 189, 255)
            } else if self.strong > 0 || heading.is_some() {
                Color32::from_rgb(239, 242, 248)
            } else {
                Color32::from_rgb(205, 210, 220)
            },
        );
        format.italics = self.emphasis > 0;
        if self.strong > 0 {
            format.extra_letter_spacing = 0.35;
        }
        if self.strike > 0 {
            format.strikethrough = Stroke::new(1.0_f32, format.color);
        }
        if self.link > 0 {
            format.underline = Stroke::new(1.0_f32, format.color);
        }
        if code {
            format.background = Color32::from_rgb(38, 43, 53);
        }
        format
    }
}

pub fn render(ui: &mut Ui, source: &str, base_dir: Option<&Path>) {
    let parser = Parser::new_ext(source, Options::all());
    let mut job = LayoutJob::default();
    let mut heading = None;
    let mut quote_depth = 0usize;
    let mut list_depth = 0usize;
    let mut code_language = None::<String>;
    let mut code = String::new();
    let mut style = InlineStyle::default();
    let mut link = None::<(String, String)>;
    let mut image = None::<(String, String)>;

    for event in parser {
        match event {
            Event::Start(Tag::Heading { level, .. }) => {
                flush_job(ui, &mut job, quote_depth);
                heading = Some(level);
            }
            Event::End(TagEnd::Heading(_)) => {
                flush_job(ui, &mut job, quote_depth);
                heading = None;
                ui.add_space(5.0);
            }
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                flush_job(ui, &mut job, quote_depth);
                ui.add_space(6.0);
            }
            Event::Start(Tag::Strong) => style.strong += 1,
            Event::End(TagEnd::Strong) => style.strong = style.strong.saturating_sub(1),
            Event::Start(Tag::Emphasis) => style.emphasis += 1,
            Event::End(TagEnd::Emphasis) => style.emphasis = style.emphasis.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => style.strike += 1,
            Event::End(TagEnd::Strikethrough) => style.strike = style.strike.saturating_sub(1),
            Event::Start(Tag::Link { dest_url, .. }) => {
                flush_job(ui, &mut job, quote_depth);
                link = Some((dest_url.into_string(), String::new()));
            }
            Event::End(TagEnd::Link) => {
                if let Some((target, label)) = link.take() {
                    let target = resolved_link_target(&target, base_dir);
                    let label = if label.is_empty() {
                        target.clone()
                    } else {
                        label
                    };
                    ui.hyperlink_to(label, target);
                }
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                flush_job(ui, &mut job, quote_depth);
                image = Some((dest_url.into_string(), String::new()));
            }
            Event::End(TagEnd::Image) => {
                if let Some((target, alt)) = image.take() {
                    render_image(ui, &target, &alt, base_dir);
                }
            }
            Event::Start(Tag::BlockQuote(_)) => quote_depth += 1,
            Event::End(TagEnd::BlockQuote(_)) => {
                flush_job(ui, &mut job, quote_depth);
                quote_depth = quote_depth.saturating_sub(1);
            }
            Event::Start(Tag::List(_)) => list_depth += 1,
            Event::End(TagEnd::List(_)) => {
                list_depth = list_depth.saturating_sub(1);
                ui.add_space(3.0);
            }
            Event::Start(Tag::Item) => job.append(
                &format!("{}• ", "  ".repeat(list_depth.saturating_sub(1))),
                0.0,
                style.format(heading, false),
            ),
            Event::End(TagEnd::Item) => flush_job(ui, &mut job, quote_depth),
            Event::Start(Tag::CodeBlock(kind)) => {
                flush_job(ui, &mut job, quote_depth);
                code_language = Some(match kind {
                    CodeBlockKind::Fenced(lang) => lang.into_string(),
                    CodeBlockKind::Indented => String::new(),
                });
            }
            Event::End(TagEnd::CodeBlock) => {
                let language = code_language.take().unwrap_or_default();
                if let Some(diagram) = diagram_language(&language) {
                    render_diagram(ui, diagram, &code);
                } else {
                    render_code(ui, language, &code);
                }
                code.clear();
            }
            Event::Start(Tag::Table(_)) => {
                flush_job(ui, &mut job, quote_depth);
                ui.separator();
            }
            Event::End(TagEnd::Table) => {
                flush_job(ui, &mut job, quote_depth);
                ui.separator();
            }
            Event::End(TagEnd::TableRow) => flush_job(ui, &mut job, quote_depth),
            Event::End(TagEnd::TableCell) => job.append(
                "  │  ",
                0.0,
                TextFormat::simple(FontId::monospace(15.0), Color32::DARK_GRAY),
            ),
            Event::Rule => {
                flush_job(ui, &mut job, quote_depth);
                ui.separator();
            }
            Event::Text(value) if code_language.is_some() => code.push_str(&value),
            Event::Text(value) if image.is_some() => {
                image.as_mut().unwrap().1.push_str(&value);
            }
            Event::Text(value) if link.is_some() => {
                link.as_mut().unwrap().1.push_str(&value);
            }
            Event::Text(value) => job.append(&value, 0.0, style.format(heading, false)),
            Event::Code(value) if link.is_some() => {
                link.as_mut().unwrap().1.push_str(&value);
            }
            Event::Code(value) => job.append(&value, 1.0, style.format(heading, true)),
            Event::InlineMath(value) => {
                flush_job(ui, &mut job, quote_depth);
                render_math(ui, &value, false);
            }
            Event::DisplayMath(value) => {
                flush_job(ui, &mut job, quote_depth);
                render_math(ui, &value, true);
                ui.add_space(6.0);
            }
            Event::SoftBreak => job.append("\n", 0.0, style.format(heading, false)),
            Event::HardBreak => {
                job.append("\n", 0.0, style.format(heading, false));
                flush_job(ui, &mut job, quote_depth);
            }
            Event::TaskListMarker(done) => job.append(
                if done { "☑ " } else { "☐ " },
                0.0,
                style.format(heading, false),
            ),
            Event::Html(value) | Event::InlineHtml(value) => {
                job.append(&value, 0.0, style.format(heading, true));
            }
            Event::FootnoteReference(value) => {
                job.append(&format!("[^{value}]"), 0.0, style.format(heading, false))
            }
            _ => {}
        }
    }
    flush_job(ui, &mut job, quote_depth);
}

fn is_external_target(target: &str) -> bool {
    target.contains("://") || target.starts_with("mailto:") || target.starts_with("data:")
}

fn resolved_local_path(target: &str, base_dir: Option<&Path>) -> PathBuf {
    let path = PathBuf::from(target);
    if path.is_absolute() {
        path
    } else {
        base_dir.unwrap_or_else(|| Path::new(".")).join(path)
    }
}

fn resolved_link_target(target: &str, base_dir: Option<&Path>) -> String {
    if target.starts_with('#') || is_external_target(target) {
        return target.to_owned();
    }
    let path = resolved_local_path(target, base_dir)
        .to_string_lossy()
        .replace('\\', "/");
    format!("file://{path}")
}

fn render_image(ui: &mut Ui, target: &str, alt: &str, base_dir: Option<&Path>) {
    let max_width = ui.available_width().max(120.0);
    let image = if is_external_target(target) {
        egui::Image::from_uri(target)
    } else {
        let path = resolved_local_path(target, base_dir);
        let Ok(bytes) = std::fs::read(&path) else {
            ui.colored_label(
                Color32::from_rgb(235, 130, 130),
                format!("Image not found: {}", path.display()),
            );
            return;
        };
        egui::Image::from_bytes(format!("bytes://{}", path.display()), bytes)
    };
    let response = ui.add(image.max_width(max_width).max_height(720.0));
    if !alt.is_empty() {
        response.on_hover_text(alt);
    }
    ui.add_space(7.0);
}

fn flush_job(ui: &mut Ui, job: &mut LayoutJob, quote_depth: usize) {
    if job.text.trim().is_empty() {
        *job = LayoutJob::default();
        return;
    }
    let mut content = std::mem::take(job);
    content.wrap.max_width = ui.available_width().max(80.0);
    if quote_depth > 0 {
        egui::Frame::new()
            .fill(Color32::from_rgb(34, 42, 54))
            .inner_margin(10)
            .show(ui, |ui| {
                ui.label(content);
            });
    } else {
        ui.label(content);
    }
}

fn render_code(ui: &mut Ui, language: String, code: &str) {
    egui::Frame::new()
        .fill(Color32::from_rgb(24, 28, 36))
        .corner_radius(6)
        .inner_margin(12)
        .show(ui, |ui| {
            if !language.is_empty() {
                ui.label(
                    RichText::new(language)
                        .small()
                        .color(Color32::from_rgb(112, 189, 255)),
                );
            }
            ui.label(
                RichText::new(code.trim_end())
                    .monospace()
                    .color(Color32::from_rgb(224, 228, 236)),
            );
        });
    ui.add_space(7.0);
}

fn render_math(ui: &mut Ui, tex: &str, display: bool) {
    match request_math_svg(ui.ctx(), tex, display) {
        Some(Ok(svg)) => {
            let mut hasher = DefaultHasher::new();
            tex.hash(&mut hasher);
            display.hash(&mut hasher);
            let texture_id = egui::Id::new(("markguin-math", hasher.finish()));
            let max_width = ui.available_width().max(80.0);
            let texture = if let Some(texture) = ui
                .ctx()
                .data_mut(|data| data.get_temp::<egui::TextureHandle>(texture_id))
            {
                Some(texture)
            } else {
                load_svg_with_fonts(svg.as_bytes()).ok().map(|image| {
                    let texture = ui.ctx().load_texture(
                        format!("markguin-math-{}", hasher.finish()),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(texture_id, texture.clone()));
                    texture
                })
            };
            let Some(texture) = texture else {
                ui.label(
                    RichText::new(format!("${tex}$"))
                        .monospace()
                        .color(Color32::from_rgb(235, 130, 130)),
                )
                .on_hover_text("Could not rasterize the rendered equation");
                return;
            };
            let image = || egui::Image::new(&texture).max_width(max_width);
            if display {
                ui.horizontal_centered(|ui| {
                    ui.add(image());
                });
            } else {
                ui.add(image());
            }
        }
        Some(Err(error)) => {
            ui.label(
                RichText::new(format!("${tex}$"))
                    .monospace()
                    .color(Color32::from_rgb(235, 130, 130)),
            )
            .on_hover_text(format!("Could not render TeX: {error}"));
        }
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new("Rendering equation…")
                        .italics()
                        .color(Color32::GRAY),
                );
            });
        }
    }
}

fn render_diagram(ui: &mut Ui, language: DiagramLanguage, source: &str) {
    match request_diagram_svg(ui.ctx(), language, source) {
        Some(Ok(svg)) => {
            let mut hasher = DefaultHasher::new();
            language.hash(&mut hasher);
            source.hash(&mut hasher);
            let hash = hasher.finish();
            let texture_id = egui::Id::new(("markguin-diagram", hash));
            let texture = if let Some(texture) = ui
                .ctx()
                .data_mut(|data| data.get_temp::<egui::TextureHandle>(texture_id))
            {
                Some(texture)
            } else {
                load_svg_with_fonts(svg.as_bytes()).ok().map(|image| {
                    let texture = ui.ctx().load_texture(
                        format!("markguin-diagram-{hash}"),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(texture_id, texture.clone()));
                    texture
                })
            };
            if let Some(texture) = texture {
                let max_width = ui.available_width().max(120.0);
                ui.horizontal_centered(|ui| {
                    ui.add(
                        egui::Image::new(&texture)
                            .max_width(max_width)
                            .max_height(640.0),
                    );
                });
            } else {
                render_diagram_error(ui, language, source, "Could not rasterize diagram SVG");
            }
        }
        Some(Err(error)) => render_diagram_error(ui, language, source, &error),
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("Rendering {} diagram…", language.css_name()));
            });
        }
    }
    ui.add_space(7.0);
}

fn render_diagram_error(ui: &mut Ui, language: DiagramLanguage, source: &str, error: &str) {
    egui::Frame::new()
        .fill(Color32::from_rgb(45, 29, 33))
        .corner_radius(6)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!("Could not render {}: {error}", language.css_name()))
                    .color(Color32::from_rgb(245, 154, 164)),
            );
            ui.label(RichText::new(source.trim_end()).monospace().small());
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_atx_headings() {
        assert_eq!(
            headings("# One\ntext\n### Three")
                .iter()
                .map(|h| (&*h.title, h.level, h.line))
                .collect::<Vec<_>>(),
            vec![("One", 1, 1), ("Three", 3, 3)]
        );
    }

    #[test]
    fn heading_extraction_ignores_code_and_supports_setext() {
        let source = "Title\n=====\n\n```md\n# Not a heading\n```\n\n## Real\n";
        let found = headings(source);
        assert_eq!(
            found
                .iter()
                .map(|heading| (&*heading.title, heading.level, heading.line))
                .collect::<Vec<_>>(),
            vec![("Title", 1, 1), ("Real", 2, 8)]
        );
    }

    #[test]
    fn source_highlighting_preserves_every_byte() {
        let source = "# 日本語\n\n- **item**\n```rust\nfn main() {}\n```\n";
        assert_eq!(highlight_source(source).text, source);
    }

    #[test]
    fn source_highlighting_creates_distinct_sections() {
        let job = highlight_source("# Heading\nplain\n> quote\n");
        assert_eq!(job.sections.len(), 3);
        assert_ne!(job.sections[0].format.color, job.sections[1].format.color);
    }

    #[test]
    fn html_export_is_standalone_and_escapes_title() {
        let html = to_html_document("# 日本語\n\n**bold**", "A <B> & C");
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("<meta charset=\"utf-8\">"));
        assert!(html.contains("<style>"));
        assert!(html.contains("<title>A &lt;B&gt; &amp; C</title>"));
        assert!(html.contains("<h1>日本語</h1>"));
        assert!(html.contains("<strong>bold</strong>"));
    }

    #[test]
    fn html_export_supports_gfm_tables_and_tasks() {
        let source = "| A | B |\n|---|---|\n| 1 | 2 |\n\n- [x] done";
        let html = to_html_document(source, "Test");
        assert!(html.contains("<table>"));
        assert!(html.contains("type=\"checkbox\""));
    }

    #[test]
    fn toc_supports_unicode_hierarchy_and_duplicate_anchors() {
        let source = "# 文書\n## はじめに\n### Details Here\n## はじめに\n";
        let toc = table_of_contents(source);
        assert!(toc.contains("- [文書](#文書)"));
        assert!(toc.contains("  - [はじめに](#はじめに)"));
        assert!(toc.contains("    - [Details Here](#details-here)"));
        assert!(toc.contains("  - [はじめに](#はじめに-1)"));
        assert_eq!(heading_anchor("✨"), "section");
    }

    #[test]
    fn toc_is_inserted_after_title_then_updated_in_place() {
        let source = "# Title\n\n## First\n";
        let (inserted, was_update) = upsert_table_of_contents(source);
        assert!(!was_update);
        assert!(inserted.starts_with("# Title\n\n<!-- TOC -->"));
        let changed = inserted.replace("## First", "## Renamed");
        let (updated, was_update) = upsert_table_of_contents(&changed);
        assert!(was_update);
        assert_eq!(updated.matches("<!-- TOC -->").count(), 1);
        assert!(updated.contains("[Renamed](#renamed)"));
        assert!(!updated.contains("[First](#first)"));
    }

    #[test]
    fn table_formatter_handles_unicode_alignment_and_escaped_pipes() {
        let source = "|名前|Score| Note |\n|:---|---:|:---:|\n|猫|10|a\\|b|\n\nafter\n";
        let (formatted, count) = format_tables(source);
        assert_eq!(count, 1);
        assert_eq!(
            formatted,
            "| 名前 | Score | Note  |\n| :--- | ----: | :---: |\n| 猫   | 10    | a\\|b  |\n\nafter\n"
        );
    }

    #[test]
    fn table_formatter_leaves_non_tables_byte_for_byte() {
        let source = "prose | with a pipe\n--- not a separator\n\n```\n| code | only |\n| --- | --- |\n```\n";
        let (formatted, count) = format_tables(source);
        // A fenced code example is intentionally not treated as a document table.
        assert_eq!(count, 0);
        assert_eq!(formatted, source);
    }

    #[test]
    fn html_export_embeds_rendered_inline_and_display_math() {
        let html = to_html_document(
            "Inline $x^2 + y^2$ equation.\n\n$$\\int_0^1 x\\,dx$$",
            "Math",
        );
        assert!(html.contains("class=\"math math-inline\""));
        assert!(html.contains("class=\"math math-display\""));
        assert!(html.matches("<svg").count() >= 2);
        assert!(!html.contains("currentColor"));
        assert!(!html.contains("<mjx-container"));
        assert!(!html.contains("ex\""));
        let svg_start = html.find("<svg").unwrap();
        let svg_end = html[svg_start..].find("</svg>").unwrap() + svg_start + 6;
        load_svg_with_fonts(&html.as_bytes()[svg_start..svg_end]).unwrap();
    }

    #[test]
    fn diagram_languages_and_aliases_are_recognized() {
        assert_eq!(diagram_language("mermaid"), Some(DiagramLanguage::Mermaid));
        assert_eq!(
            diagram_language("mmd title=Flow"),
            Some(DiagramLanguage::Mermaid)
        );
        assert_eq!(
            diagram_language("plantuml"),
            Some(DiagramLanguage::PlantUml)
        );
        assert_eq!(diagram_language("puml"), Some(DiagramLanguage::PlantUml));
        assert_eq!(diagram_language("rust"), None);
    }

    #[test]
    fn relative_preview_targets_resolve_from_document_directory() {
        let base = Path::new("/notes/project");
        assert_eq!(
            resolved_local_path("images/plot.png", Some(base)),
            PathBuf::from("/notes/project/images/plot.png")
        );
        assert_eq!(
            resolved_link_target("chapter.md", Some(base)),
            "file:///notes/project/chapter.md"
        );
        assert_eq!(
            resolved_link_target("https://example.com", Some(base)),
            "https://example.com"
        );
        assert_eq!(resolved_link_target("#result", Some(base)), "#result");
    }

    #[test]
    fn mermaid_and_plantuml_render_to_loadable_svg() {
        let mermaid = render_diagram_svg(DiagramLanguage::Mermaid, "flowchart LR\nA-->B").unwrap();
        let plantuml = render_diagram_svg(
            DiagramLanguage::PlantUml,
            "@startuml\nAlice -> Bob: Hello\n@enduml",
        )
        .unwrap();
        for svg in [mermaid, plantuml] {
            assert!(svg.starts_with("<svg"));
            assert!(svg.ends_with("</svg>"));
            load_svg_with_fonts(svg.as_bytes()).unwrap();
        }
    }

    #[test]
    fn html_export_embeds_diagrams_without_javascript() {
        let source = "```mermaid\nflowchart LR\nA-->B\n```\n\n```plantuml\nAlice -> Bob: Hi\n```";
        let html = to_html_document(source, "Diagrams");
        assert!(html.contains("diagram-mermaid"));
        assert!(html.contains("diagram-plantuml"));
        assert_eq!(html.matches("<svg").count(), 2);
        assert!(!html.contains("language-mermaid"));
        assert!(!html.contains("<script"));
    }
}
