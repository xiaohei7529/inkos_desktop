//! 全书导出：TXT / Markdown / EPUB（手写 EPUB3 容器，不引入大型依赖）。
//!
//! 灵感来源：Narcooo/inkos 的 `inkos export` 命令。
//!
//! 落到 `<novel_root>/导出/<书名>-<日期>.<ext>`。

use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::Local;
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

use crate::project::{NovelProject, ProjectStore};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Txt,
    Markdown,
    Epub,
}

impl ExportFormat {
    pub fn ext(&self) -> &'static str {
        match self {
            ExportFormat::Txt => "txt",
            ExportFormat::Markdown => "md",
            ExportFormat::Epub => "epub",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            ExportFormat::Txt => "TXT",
            ExportFormat::Markdown => "Markdown",
            ExportFormat::Epub => "EPUB",
        }
    }
}

pub fn export_root(novel_root: &Path) -> PathBuf {
    novel_root.join("导出")
}

pub fn export_book(
    store: &ProjectStore,
    project: &NovelProject,
    format: ExportFormat,
) -> Result<PathBuf> {
    fs::create_dir_all(export_root(store.root()))?;
    let title = if project.title.trim().is_empty() {
        store.root_dir_name()
    } else {
        project.title.trim().to_string()
    };
    let safe_title = sanitize_filename(&title);
    let date = Local::now().format("%Y%m%d-%H%M%S").to_string();
    let path = export_root(store.root()).join(format!("{safe_title}-{date}.{}", format.ext()));

    let mut chapters: Vec<_> = project.chapters.iter().collect();
    chapters.sort_by_key(|c| c.number);

    match format {
        ExportFormat::Txt => write_txt(&path, &title, project, &chapters, store)?,
        ExportFormat::Markdown => write_markdown(&path, &title, project, &chapters, store)?,
        ExportFormat::Epub => write_epub(&path, &title, project, &chapters, store)?,
    }
    Ok(path)
}

fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

fn write_txt(
    path: &Path,
    title: &str,
    project: &NovelProject,
    chapters: &[&crate::project::ChapterRecord],
    store: &ProjectStore,
) -> Result<()> {
    let mut buf = String::new();
    buf.push_str(&format!("{title}\n\n"));
    if !project.premise.trim().is_empty() {
        buf.push_str(&project.premise);
        buf.push_str("\n\n");
    }
    buf.push_str(&"-".repeat(40));
    buf.push_str("\n\n");

    for c in chapters {
        let (file_title, body) = store.load_chapter_content(c.number).unwrap_or_default();
        let t = if !file_title.is_empty() { file_title } else { c.title.clone() };
        buf.push_str(&format!("第{}章 {}\n\n", c.number, t));
        buf.push_str(body.trim());
        buf.push_str("\n\n");
    }

    fs::write(path, buf).with_context(|| format!("write {:?}", path))?;
    Ok(())
}

fn write_markdown(
    path: &Path,
    title: &str,
    project: &NovelProject,
    chapters: &[&crate::project::ChapterRecord],
    store: &ProjectStore,
) -> Result<()> {
    let mut buf = String::new();
    buf.push_str(&format!("# {title}\n\n"));
    if !project.genre.trim().is_empty() {
        buf.push_str(&format!("*题材：{}*\n\n", project.genre.trim()));
    }
    if !project.premise.trim().is_empty() {
        buf.push_str("> ");
        buf.push_str(&project.premise.replace('\n', "\n> "));
        buf.push_str("\n\n");
    }

    for c in chapters {
        let (file_title, body) = store.load_chapter_content(c.number).unwrap_or_default();
        let t = if !file_title.is_empty() { file_title } else { c.title.clone() };
        buf.push_str(&format!("## 第{}章 {}\n\n", c.number, t));
        buf.push_str(body.trim());
        buf.push_str("\n\n");
    }
    fs::write(path, buf).with_context(|| format!("write {:?}", path))?;
    Ok(())
}

fn write_epub(
    path: &Path,
    title: &str,
    project: &NovelProject,
    chapters: &[&crate::project::ChapterRecord],
    store: &ProjectStore,
) -> Result<()> {
    let buffer: Vec<u8> = Vec::new();
    let mut zip = zip::ZipWriter::new(Cursor::new(buffer));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    // mimetype 必须无压缩且为第一个条目
    zip.start_file("mimetype", stored)?;
    zip.write_all(b"application/epub+zip")?;

    zip.start_file("META-INF/container.xml", deflated)?;
    zip.write_all(CONTAINER_XML.as_bytes())?;

    let book_id = format!("urn:inkos:{}", chrono::Local::now().timestamp());
    let safe_title = xml_escape(title);
    let author = if project.protagonists.trim().is_empty() {
        "InkOS Desktop".to_string()
    } else {
        format!("InkOS Desktop · {}", project.protagonists.trim())
    };
    let author_x = xml_escape(&author);
    let now = Local::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

    let mut manifest_items = String::new();
    let mut spine_items = String::new();
    let mut nav_items = String::new();
    manifest_items.push_str(
        r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item id="css" href="style.css" media-type="text/css"/>
<item id="cover" href="cover.xhtml" media-type="application/xhtml+xml"/>"#,
    );
    spine_items.push_str(r#"<itemref idref="cover"/>"#);

    for c in chapters {
        let (file_title, body) = store.load_chapter_content(c.number).unwrap_or_default();
        let t = if !file_title.is_empty() { file_title } else { c.title.clone() };
        let id = format!("ch{:04}", c.number);
        let href = format!("{id}.xhtml");
        manifest_items.push_str(&format!(
            "\n<item id=\"{id}\" href=\"{href}\" media-type=\"application/xhtml+xml\"/>"
        ));
        spine_items.push_str(&format!("\n<itemref idref=\"{id}\"/>"));
        nav_items.push_str(&format!(
            "\n<li><a href=\"{href}\">第{}章 {}</a></li>",
            c.number,
            xml_escape(&t)
        ));

        let body_html = paragraphs_to_xhtml(body.trim());
        let xhtml = format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xml:lang="zh-CN" lang="zh-CN">
<head>
<meta charset="utf-8"/>
<title>第{n}章 {t}</title>
<link rel="stylesheet" type="text/css" href="style.css"/>
</head>
<body>
<h2>第{n}章 {t}</h2>
{body}
</body>
</html>"#,
            n = c.number,
            t = xml_escape(&t),
            body = body_html
        );
        zip.start_file(format!("OEBPS/{href}"), deflated)?;
        zip.write_all(xhtml.as_bytes())?;
    }

    let cover = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xml:lang="zh-CN">
<head><meta charset="utf-8"/><title>{t}</title>
<link rel="stylesheet" type="text/css" href="style.css"/></head>
<body>
<h1 class="cover-title">{t}</h1>
<p class="cover-author">{a}</p>
<p class="cover-desc">{p}</p>
</body>
</html>"#,
        t = safe_title,
        a = author_x,
        p = xml_escape(project.premise.trim())
    );
    zip.start_file("OEBPS/cover.xhtml", deflated)?;
    zip.write_all(cover.as_bytes())?;

    let nav = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<!DOCTYPE html>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="zh-CN">
<head><meta charset="utf-8"/><title>目录</title></head>
<body>
<nav epub:type="toc" id="toc"><h1>目录</h1>
<ol>{nav_items}</ol>
</nav>
</body>
</html>"#,
        nav_items = nav_items
    );
    zip.start_file("OEBPS/nav.xhtml", deflated)?;
    zip.write_all(nav.as_bytes())?;

    let css = "body { font-family: serif; line-height: 1.7; padding: 1em; }
h1, h2 { text-align: center; }
p { text-indent: 2em; margin: 0.5em 0; }
.cover-title { font-size: 2em; }
.cover-author { text-align: center; color: #555; }
.cover-desc { color: #444; margin-top: 2em; }
";
    zip.start_file("OEBPS/style.css", deflated)?;
    zip.write_all(css.as_bytes())?;

    let opf = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="bookid" xml:lang="zh-CN">
<metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
<dc:identifier id="bookid">{id}</dc:identifier>
<dc:title>{t}</dc:title>
<dc:creator>{a}</dc:creator>
<dc:language>zh-CN</dc:language>
<meta property="dcterms:modified">{now}</meta>
</metadata>
<manifest>{manifest_items}</manifest>
<spine>{spine_items}</spine>
</package>"#,
        id = book_id,
        t = safe_title,
        a = author_x,
        now = now,
        manifest_items = manifest_items,
        spine_items = spine_items
    );
    zip.start_file("OEBPS/content.opf", deflated)?;
    zip.write_all(opf.as_bytes())?;

    let cursor = zip.finish()?;
    fs::write(path, cursor.into_inner()).with_context(|| format!("write {:?}", path))?;
    Ok(())
}

const CONTAINER_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
"#;

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn paragraphs_to_xhtml(body: &str) -> String {
    let mut out = String::new();
    for para in body.split("\n\n") {
        let p = para.trim();
        if p.is_empty() {
            continue;
        }
        out.push_str("<p>");
        out.push_str(&xml_escape(p).replace('\n', "<br/>"));
        out.push_str("</p>\n");
    }
    out
}
