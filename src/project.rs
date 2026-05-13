//! 小说目录、`project.json`、章节文件（对齐 `inkoswin.project_store`）。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::chapter_md::{
    compose_chapter_markdown, count_story_units, make_summary, parse_chapter_markdown,
    DEFAULT_CHAPTER_SUMMARY_LIMIT,
};

pub fn now_iso() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoGeneratePlan {
    pub enabled: bool,
    pub interval_minutes: i32,
    pub last_run_at: String,
}

impl Default for AutoGeneratePlan {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_minutes: 30,
            last_run_at: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChapterRecord {
    pub number: i32,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default)]
    pub word_count: i32,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

fn default_status() -> String {
    "draft".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NovelProject {
    pub title: String,
    pub genre: String,
    pub premise: String,
    pub protagonists: String,
    pub world_setting: String,
    pub writing_style: String,
    pub outline: String,
    pub extra_guidance: String,
    pub target_chapters: i32,
    pub chapter_word_goal: i32,
    pub auto_generate: AutoGeneratePlan,
    pub chapters: Vec<ChapterRecord>,
    pub updated_at: String,
}

impl Default for NovelProject {
    fn default() -> Self {
        Self {
            title: String::new(),
            genre: String::new(),
            premise: String::new(),
            protagonists: String::new(),
            world_setting: String::new(),
            writing_style: String::new(),
            outline: String::new(),
            extra_guidance: String::new(),
            target_chapters: 500,
            chapter_word_goal: 3000,
            auto_generate: AutoGeneratePlan::default(),
            chapters: Vec::new(),
            updated_at: now_iso(),
        }
    }
}

pub struct ProjectStore {
    root: PathBuf,
}

impl ProjectStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn root_dir_name(&self) -> String {
        self.root
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("novel")
            .to_string()
    }

    pub fn hidden_dir(&self) -> PathBuf {
        self.root.join(".inkoswin")
    }

    pub fn meta_path(&self) -> PathBuf {
        self.hidden_dir().join("project.json")
    }

    pub fn chapters_dir(&self) -> PathBuf {
        self.root.join("chapters")
    }

    pub fn state_dir(&self) -> PathBuf {
        self.root.join("story_state")
    }

    pub fn chapter_file(&self, number: i32) -> PathBuf {
        self.chapters_dir().join(format!("{number:03}.md"))
    }

    pub fn load_project(&self) -> Result<NovelProject> {
        let mut project = if self.meta_path().exists() {
            let raw = fs::read_to_string(self.meta_path())
                .with_context(|| format!("read {:?}", self.meta_path()))?;
            serde_json::from_str(&raw).unwrap_or_default()
        } else {
            let mut p = NovelProject::default();
            p.title = self.root_dir_name();
            p
        };

        if project.title.trim().is_empty() {
            project.title = self.root_dir_name();
        }
        if project.target_chapters <= 0 {
            project.target_chapters = 500;
        }
        if project.chapter_word_goal <= 0 {
            project.chapter_word_goal = 3000;
        }
        if project.auto_generate.interval_minutes <= 0 {
            project.auto_generate.interval_minutes = 30;
        }

        self.sync_with_files(&mut project)?;
        self.ensure_state_files(&project)?;
        Ok(project)
    }

    pub fn save_project(&self, project: &mut NovelProject) -> Result<()> {
        project.updated_at = now_iso();
        fs::create_dir_all(self.hidden_dir())?;
        let json = serde_json::to_string_pretty(project)?;
        fs::write(self.meta_path(), json)?;
        self.ensure_state_files(project)?;
        Ok(())
    }

    pub fn list_chapters(project: &NovelProject) -> Vec<&ChapterRecord> {
        let mut v: Vec<_> = project.chapters.iter().collect();
        v.sort_by_key(|c| c.number);
        v
    }

    pub fn get_chapter<'a>(project: &'a NovelProject, number: i32) -> Option<&'a ChapterRecord> {
        project.chapters.iter().find(|c| c.number == number)
    }

    pub fn ensure_chapter(&self, project: &mut NovelProject, number: i32, title: &str) -> Result<usize> {
        if let Some(i) = project.chapters.iter().position(|c| c.number == number) {
            if !title.trim().is_empty() && project.chapters[i].title.trim().is_empty() {
                project.chapters[i].title = title.trim().to_string();
            }
            return Ok(i);
        }
        let t = if title.trim().is_empty() {
            format!("第{number}章")
        } else {
            title.trim().to_string()
        };
        let now = now_iso();
        project.chapters.push(ChapterRecord {
            number,
            title: t,
            summary: String::new(),
            status: "draft".to_string(),
            word_count: 0,
            created_at: now.clone(),
            updated_at: now,
        });
        project.chapters.sort_by_key(|c| c.number);
        self.save_project(project)?;
        Ok(project.chapters.iter().position(|c| c.number == number).unwrap())
    }

    pub fn create_next_chapter(&self, project: &mut NovelProject, title: &str) -> Result<i32> {
        let next = project
            .chapters
            .iter()
            .map(|c| c.number)
            .max()
            .unwrap_or(0)
            + 1;
        self.ensure_chapter(project, next, title)?;
        Ok(next)
    }

    pub fn load_chapter_content(&self, number: i32) -> Result<(String, String)> {
        let path = self.chapter_file(number);
        if !path.exists() {
            return Ok((String::new(), String::new()));
        }
        let raw = fs::read_to_string(&path).with_context(|| format!("read {:?}", path))?;
        Ok(parse_chapter_markdown(&raw))
    }

    pub fn save_chapter(
        &self,
        project: &mut NovelProject,
        number: i32,
        title: &str,
        content: &str,
        status: &str,
        summary_override: &str,
    ) -> Result<()> {
        let idx = self.ensure_chapter(project, number, title)?;
        let chapter = &mut project.chapters[idx];

        let clean_title = if !title.trim().is_empty() {
            title.trim().to_string()
        } else if !chapter.title.trim().is_empty() {
            chapter.title.trim().to_string()
        } else {
            format!("第{number}章")
        };

        let clean_content = content.trim();
        let clean_summary = if summary_override.trim().is_empty() {
            make_summary(clean_content, DEFAULT_CHAPTER_SUMMARY_LIMIT)
        } else {
            summary_override.trim().to_string()
        };

        fs::create_dir_all(self.chapters_dir())?;
        let md = compose_chapter_markdown(&clean_title, clean_content);
        fs::write(self.chapter_file(number), md)?;

        let now = now_iso();
        chapter.title = clean_title;
        chapter.summary = clean_summary;
        chapter.status = if status.trim().is_empty() {
            "draft".to_string()
        } else {
            status.trim().to_string()
        };
        chapter.word_count = count_story_units(clean_content) as i32;
        chapter.updated_at = now.clone();
        if chapter.created_at.is_empty() {
            chapter.created_at = now;
        }

        self.save_project(project)?;
        Ok(())
    }

    /// 最近若干章节的摘要 + 节选，供 AI 刷新状态档案注入（对齐 inkoswin `build_chapter_digest`）。
    pub fn build_chapter_digest(&self, project: &NovelProject, limit: usize) -> String {
        let chapters = Self::list_chapters(project);
        let take = limit.max(1);
        let start = chapters.len().saturating_sub(take);
        let tail = &chapters[start..];
        let mut blocks: Vec<String> = Vec::new();
        for ch in tail {
            let (t, content) = self.load_chapter_content(ch.number).unwrap_or_default();
            let title_line = if !t.trim().is_empty() {
                t
            } else if !ch.title.trim().is_empty() {
                ch.title.trim().to_string()
            } else {
                format!("第{}章", ch.number)
            };
            let summary = if ch.summary.trim().is_empty() {
                make_summary(&content, DEFAULT_CHAPTER_SUMMARY_LIMIT)
            } else {
                ch.summary.trim().to_string()
            };
            let excerpt_raw = content.trim();
            let excerpt = if excerpt_raw.chars().count() > 500 {
                let cut: String = excerpt_raw.chars().take(500).collect();
                format!("{}...", cut.trim_end())
            } else {
                excerpt_raw.to_string()
            };
            blocks.push(format!(
                "第{}章：{}\n- 状态：{}\n- 摘要：{}\n- 节选：{}",
                ch.number,
                title_line,
                ch.status,
                summary,
                if excerpt.is_empty() {
                    "暂无正文".to_string()
                } else {
                    excerpt
                }
            ));
        }
        if blocks.is_empty() {
            "暂无章节内容。".to_string()
        } else {
            blocks.join("\n\n")
        }
    }

    /// 读入 `story_state/` 下标准 9 件套，用于刷新时维护内存中的档案快照。
    pub fn load_story_state_documents_map(&self, project: &NovelProject) -> Result<HashMap<String, String>> {
        self.ensure_state_files(project)?;
        let mut m = HashMap::new();
        for f in crate::state_refresh::STATE_DIR_FILENAMES {
            let path = self.state_dir().join(f);
            let text = fs::read_to_string(&path).unwrap_or_default();
            m.insert((*f).to_string(), text);
        }
        Ok(m)
    }

    /// 写入单个状态档案文件（路径限定在 `story_state/` 且文件名在白名单内）。
    pub fn write_story_state_file(&self, filename: &str, content: &str) -> Result<()> {
        if !crate::state_refresh::allowed_state_filename(filename) {
            anyhow::bail!("非法状态文件名: {filename}");
        }
        fs::create_dir_all(self.state_dir())?;
        let path = self.state_dir().join(filename);
        fs::write(path, format!("{}\n", content.trim_end()))?;
        Ok(())
    }

    pub fn build_chapter_summaries_document(&self, project: &NovelProject) -> String {
        /// 最近 N 章保留完整摘要，更早的章节归档到底部精简区段。
        const RECENT_CHAPTER_LIMIT: usize = 100;

        let mut lines: Vec<String> = vec![
            "# 各章摘要".to_string(),
            String::new(),
            "> 出场人物、关键事件、状态变化、伏笔动态".to_string(),
            "> 此文件由系统自动刷新，用于快速回顾章节进度。".to_string(),
            String::new(),
            format!(
                "- 小说名：{}",
                if project.title.trim().is_empty() {
                    self.root_dir_name()
                } else {
                    project.title.trim().to_string()
                }
            ),
            format!("- 已记录章节：{}", project.chapters.len()),
            format!("- 更新时间：{}", now_iso()),
            String::new(),
        ];

        let mut chs: Vec<_> = project.chapters.iter().collect();
        chs.sort_by_key(|c| c.number);

        if chs.is_empty() {
            lines.push("## 暂无章节".to_string());
            lines.push("- 还没有任何章节内容，保存章节后这里会自动生成摘要。".to_string());
            lines.push(String::new());
            return lines.join("\n").trim_end().to_string() + "\n";
        }

        let total = chs.len();
        let (archived_chs, recent_chs) = if total > RECENT_CHAPTER_LIMIT {
            chs.split_at(total - RECENT_CHAPTER_LIMIT)
        } else {
            (&[][..], &chs[..])
        };

        // 归档区段（超出阈值的早期章节，只保留一行简要）
        if !archived_chs.is_empty() {
            lines.push(format!(
                "## 早期章节归档（第 1 章 ～ 第 {} 章，共 {} 章）",
                archived_chs.last().map(|c| c.number).unwrap_or(0),
                archived_chs.len()
            ));
            lines.push(String::new());
            for chapter in archived_chs {
                let sum = chapter.summary.trim();
                let title_part = if chapter.title.trim().is_empty() {
                    format!("第{}章", chapter.number)
                } else {
                    chapter.title.trim().to_string()
                };
                if sum.is_empty() {
                    lines.push(format!("- 第{}章《{}》", chapter.number, title_part));
                } else {
                    let short: String = sum.chars().take(60).collect();
                    let short = if sum.chars().count() > 60 {
                        format!("{short}…")
                    } else {
                        short
                    };
                    lines.push(format!("- 第{}章《{}》：{short}", chapter.number, title_part));
                }
            }
            lines.push(String::new());
            lines.push(format!(
                "## 最近 {} 章详细摘要",
                recent_chs.len()
            ));
            lines.push(String::new());
        }

        // 最近章节（含完整摘要）
        for chapter in recent_chs {
            let (t, c) = self
                .load_chapter_content(chapter.number)
                .unwrap_or_default();
            let title_line = if t.is_empty() {
                chapter.title.clone()
            } else {
                t
            };
            let sum = if chapter.summary.trim().is_empty() {
                make_summary(&c, DEFAULT_CHAPTER_SUMMARY_LIMIT)
            } else {
                chapter.summary.clone()
            };
            lines.extend([
                format!(
                    "## 第{}章 {}",
                    chapter.number,
                    if title_line.is_empty() {
                        format!("第{}章", chapter.number)
                    } else {
                        title_line.clone()
                    }
                ),
                format!("- 状态：{}", chapter.status),
                format!("- 字数：{}", chapter.word_count),
                format!("- 摘要：{sum}"),
                format!("- 文件：{:03}.md", chapter.number),
                String::new(),
            ]);
        }
        lines.join("\n").trim_end().to_string() + "\n"
    }

    fn sync_with_files(&self, project: &mut NovelProject) -> Result<()> {
        let mut known: std::collections::HashSet<i32> =
            project.chapters.iter().map(|c| c.number).collect();

        if self.chapters_dir().exists() {
            for path in fs::read_dir(self.chapters_dir())? {
                let path = path?.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                let Some(num) = number_from_path(&path) else {
                    continue;
                };
                if known.contains(&num) {
                    continue;
                }
                let raw = fs::read_to_string(&path).unwrap_or_default();
                let (t, c) = parse_chapter_markdown(&raw);
                let mtime = mtime_iso(&path).unwrap_or_else(now_iso);
                project.chapters.push(ChapterRecord {
                    number: num,
                    title: if t.is_empty() {
                        format!("第{num}章")
                    } else {
                        t
                    },
                    summary: make_summary(&c, DEFAULT_CHAPTER_SUMMARY_LIMIT),
                    status: if c.trim().is_empty() {
                        "draft".to_string()
                    } else {
                        "generated".to_string()
                    },
                    word_count: count_story_units(&c) as i32,
                    created_at: mtime.clone(),
                    updated_at: mtime,
                });
                known.insert(num);
            }
        }

        for chapter in &mut project.chapters {
            let path = self.chapter_file(chapter.number);
            if !path.exists() {
                chapter.word_count = 0;
                continue;
            }
            let raw = fs::read_to_string(&path).unwrap_or_default();
            let (t, c) = parse_chapter_markdown(&raw);
            if !t.is_empty() {
                chapter.title = t;
            } else if chapter.title.trim().is_empty() {
                chapter.title = format!("第{}章", chapter.number);
            }
            chapter.word_count = count_story_units(&c) as i32;
            if chapter.summary.trim().is_empty() {
                chapter.summary = make_summary(&c, DEFAULT_CHAPTER_SUMMARY_LIMIT);
            }
            chapter.updated_at = mtime_iso(&path).unwrap_or_else(now_iso);
        }

        project.chapters.sort_by_key(|c| c.number);
        Ok(())
    }

    fn ensure_state_files(&self, project: &NovelProject) -> Result<()> {
        fs::create_dir_all(self.state_dir())?;
        for spec in STATE_SPECS {
            let path = self.state_dir().join(spec.filename);
            if spec.filename == "chapter_summaries.md" {
                let body = self.build_chapter_summaries_document(project);
                fs::write(&path, body)?;
                continue;
            }
            if spec.filename == "novel_brief.md" {
                if !path.exists() {
                    fs::write(&path, compose_novel_brief_from_project(project))?;
                }
                continue;
            }
            if spec.filename == "book_rules.md" {
                // book_rules.md 含 YAML frontmatter，模板原样落盘，不拼上下文 footer，避免破坏结构。
                if !path.exists() {
                    fs::write(&path, spec.template)?;
                }
                continue;
            }
            if !path.exists() {
                let mut body = spec.template.trim_end().to_string();
                body.push_str(&default_state_footer(project, self));
                fs::write(&path, body)?;
            }
        }
        Ok(())
    }
}

struct StateSpec {
    filename: &'static str,
    template: &'static str,
}

const STATE_SPECS: &[StateSpec] = &[
    StateSpec {
        filename: "book_rules.md",
        template: include_str!("../assets/state/book_rules.md"),
    },
    StateSpec {
        filename: "current_state.md",
        template: include_str!("../assets/state/current_state.md"),
    },
    StateSpec {
        filename: "particle_ledger.md",
        template: include_str!("../assets/state/particle_ledger.md"),
    },
    StateSpec {
        filename: "pending_hooks.md",
        template: include_str!("../assets/state/pending_hooks.md"),
    },
    StateSpec {
        filename: "chapter_summaries.md",
        template: include_str!("../assets/state/chapter_summaries.md"),
    },
    StateSpec {
        filename: "novel_brief.md",
        template: include_str!("../assets/state/novel_brief.md"),
    },
    StateSpec {
        filename: "outline.md",
        template: include_str!("../assets/state/outline.md"),
    },
    StateSpec {
        filename: "subplot_board.md",
        template: include_str!("../assets/state/subplot_board.md"),
    },
    StateSpec {
        filename: "emotional_arcs.md",
        template: include_str!("../assets/state/emotional_arcs.md"),
    },
    StateSpec {
        filename: "character_matrix.md",
        template: include_str!("../assets/state/character_matrix.md"),
    },
];

fn compose_novel_brief_from_project(project: &NovelProject) -> String {
    let p = |s: &str| -> String {
        let t = s.trim();
        if t.is_empty() {
            "待补充".to_string()
        } else {
            t.to_string()
        }
    };
    format!(
        r#"# 小说设定摘要

> 本文件属于长期状态档案的一部分，可与写作工作台「小说设定」表单互相载入、回写。

## 故事核心
{}

## 主角与关键角色
{}

## 世界观与背景
{}

## 文风与节奏要求
{}

## 大纲与章节方向
{}

## 额外提示词
{}
"#,
        p(&project.premise),
        p(&project.protagonists),
        p(&project.world_setting),
        p(&project.writing_style),
        p(&project.outline),
        p(&project.extra_guidance),
    )
}

fn default_state_footer(project: &NovelProject, store: &ProjectStore) -> String {
    let title = project.title.trim();
    let title = if title.is_empty() {
        store.root_dir_name()
    } else {
        title.to_string()
    };
    format!(
        "\n\n## 项目上下文\n- 小说名：{title}\n- 题材：{}\n- 当前已记录章节数：{}\n\n",
        if project.genre.trim().is_empty() {
            "未填写"
        } else {
            project.genre.trim()
        },
        project.chapters.len(),
    )
}

fn number_from_path(path: &Path) -> Option<i32> {
    let stem = path.file_stem()?.to_str()?;
    let digits: String = stem.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

fn mtime_iso(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    let st = meta.modified().ok()?;
    let secs = st.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
    let dt = chrono::DateTime::from_timestamp(secs, 0)?;
    Some(
        dt.with_timezone(&chrono::Local)
            .format("%Y-%m-%dT%H:%M:%S")
            .to_string(),
    )
}
