//! 现代风格 UI：左侧导航 + 卡片化页面；多服务商；流式 AI 审计/助手/定时写作；状态档案并入小说设定。

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Instant, SystemTime};

use chrono::NaiveDate;
use eframe::egui::{self, Color32, ComboBox, CornerRadius, Margin, RichText, Stroke, TextEdit};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

use crate::audit_log::{self, AuditRecord};
use crate::chapter_md::parse_chapter_markdown;
use crate::config::{unique_paths, AppSettings, ConfigPaths, LlmConfig, VendorConfig};
use crate::fonts::install_cjk_fonts;
use crate::history::{self, ChapterRevision};
use crate::inkoswin_prompt;
use crate::llm::{spawn_chat, spawn_ping, ChatMessage, LlmTask};
use crate::oplog::{self, OpLogEntry};
use crate::project::{self, NovelProject, ProjectStore};
use crate::state_sync::{self, StateFileChange};
use crate::theme::{self, color, dim_label, page_header, section_label};
use crate::vendors::{find as find_vendor, VendorPreset, VENDORS};

#[derive(Copy, Clone, PartialEq, Eq)]
enum Section {
    Project,
    Writing,
    Review,
    Assistant,
    NovelMeta,
    OpLog,
    Tools,
    Settings,
}

impl Section {
    fn header(self) -> (&'static str, &'static str) {
        match self {
            Section::Project => ("项目", "选择并管理本地小说目录、定时写作。"),
            Section::Writing => ("写作", "撰写章节、查看 Markdown 预览。"),
            Section::Review => ("审核", "选择章节与 AI，让模型审计或改写写作内容。"),
            Section::Assistant => ("写作助手", "与「写作 LLM」围绕当前小说协作。"),
            Section::NovelMeta => ("小说设定", "基础信息、状态档案、作者意图与当前焦点。"),
            Section::OpLog => ("操作日志", "按日期回看所有关键事件，落盘到「操作日志」文件夹。"),
            Section::Tools => (
                "工具箱",
                "全书搜索、导入、导出 EPUB、改名、写作管线、文风、AIGC、数据分析、同人向导。",
            ),
            Section::Settings => ("设置", "服务商管理、写作/审计 LLM 配置、字数治理、Agent 路由、通知。"),
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum ToolsTab {
    Search,
    Import,
    Export,
    Rename,
    Pipeline,
    Style,
    Aigc,
    Analytics,
    Fanfic,
}

impl ToolsTab {
    fn label(self) -> &'static str {
        match self {
            ToolsTab::Search => "全书搜索",
            ToolsTab::Import => "导入章节",
            ToolsTab::Export => "导出全书",
            ToolsTab::Rename => "实体改名",
            ToolsTab::Pipeline => "写作管线",
            ToolsTab::Style => "文风指纹",
            ToolsTab::Aigc => "AIGC 检测",
            ToolsTab::Analytics => "数据分析",
            ToolsTab::Fanfic => "同人向导",
        }
    }
}

#[derive(Default, Clone, PartialEq)]
enum AutoGenPhase {
    #[default]
    Idle,
    /// 链式工作流第一步：审计上一章。
    AuditingPrev { prev_n: i32, next_n: i32 },
    /// 第二步（或非链式时的唯一一步）：写下一章。
    Writing { next_n: i32 },
}

#[derive(Default, Clone, PartialEq)]
#[allow(dead_code)]
enum StateSyncPhase {
    #[default]
    Idle,
    /// 替换原文成功后，正在让 LLM 产出状态档案差异。
    Running { chapter_no: i32 },
}

/// AI 顺序刷新 `story_state/`（对齐 inkoswin `_state_documents_worker`）。
struct StateRefreshBatch {
    queue: Vec<String>,
    index: usize,
    state_documents: HashMap<String, String>,
    chapter_digest: String,
    project_snapshot: NovelProject,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum NovelMetaTab {
    Basics,
    StateDocs,
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum ReviewMode {
    Audit,
    Rewrite,
}

const STATE_FILES: &[&str] = &[
    "book_rules.md",
    "current_state.md",
    "particle_ledger.md",
    "pending_hooks.md",
    "chapter_summaries.md",
    "novel_brief.md",
    "subplot_board.md",
    "emotional_arcs.md",
    "character_matrix.md",
];

/// 「story/」子目录下的扩展状态档案（致敬 Narcooo/inkos 的 Input Governance / Style / Fanfic 层）。
/// 与 `STATE_FILES` 一起出现在「小说设定 → 状态档案」面板的下拉框中。
const STORY_FILES: &[(&str, &str)] = &[
    ("story/author_intent.md", "作者长期意图"),
    ("story/current_focus.md", "当前焦点（近 1-3 章）"),
    ("story/style_fingerprint.md", "文风指纹"),
    ("story/fanfic_brief.md", "同人书草案"),
];

const CHAPTER_STATUSES: &[(&str, &str)] = &[
    ("draft", "草稿"),
    ("writing", "写作中"),
    ("review", "待审核"),
    ("completed", "已完成"),
    ("archived", "已归档"),
    ("generated", "已生成"),
];

fn status_label(value: &str) -> &'static str {
    CHAPTER_STATUSES
        .iter()
        .find(|(k, _)| *k == value)
        .map(|(_, l)| *l)
        .unwrap_or("自定义")
}

fn status_color(value: &str) -> Color32 {
    match value {
        "completed" => color::SUCCESS,
        "review" => color::WARNING,
        "writing" => color::ACCENT_HI,
        "archived" => color::TEXT_FAINT,
        "generated" => color::ACCENT,
        _ => color::TEXT_DIM,
    }
}

const GENRES: &[&str] = &[
    "玄幻", "奇幻", "武侠", "仙侠", "都市", "现实", "历史", "军事", "游戏", "体育", "科幻",
    "悬疑", "轻小说", "言情", "校园", "推理", "灵异", "恐怖", "商战", "职场", "其他",
];

pub struct InkOsApp {
    paths: ConfigPaths,
    settings: AppSettings,
    section: Section,
    init_done: bool,

    global_llm: LlmConfig,
    novel_llm: LlmConfig,

    novel_path: Option<PathBuf>,
    store: Option<ProjectStore>,
    project: Option<NovelProject>,

    selected_chapter: Option<i32>,
    chapter_title: String,
    chapter_body: String,
    chapter_status: String,
    chapter_summary: String,
    chapter_dirty: bool,
    chapter_summary_dirty: bool,

    preview_md: String,
    preview_deadline: Option<f64>,
    cm_cache: CommonMarkCache,

    nm_tab: NovelMetaTab,
    selected_state_file: String,
    state_doc_body: String,
    state_doc_dirty: bool,
    state_doc_loaded_for: Option<String>,

    review_mode: ReviewMode,
    review_target: Option<i32>,
    review_target_original: String,
    review_compare: bool,
    review_result: String,
    review_for_chapter: Option<i32>,
    review_task: Option<LlmTask>,
    /// 切换 tab 时缓存的「审计」结果（Audit 模式下不在 review_result 时存这里）。
    audit_buf: String,
    audit_buf_for_chapter: Option<i32>,
    /// 切换 tab 时缓存的「改写」结果（Rewrite 模式下不在 review_result 时存这里）。
    rewrite_buf: String,
    rewrite_buf_for_chapter: Option<i32>,
    /// 当前小说的所有审计/改写历史记录（最新在前）。
    audit_records: Vec<AuditRecord>,
    /// 审计记录侧栏选中的 timestamp，None 表示未选中（显示当次最新的实时结果）。
    selected_audit_ts: Option<String>,

    /// 替换原文后链式触发的「状态档案同步」LLM 任务。
    state_sync_task: Option<LlmTask>,
    state_sync_phase: StateSyncPhase,
    /// 最近一次状态同步执行的简明日志（每条一行），UI 在审核页底部展示。
    state_sync_log: Vec<String>,

    assistant_input: String,
    assistant_log: Vec<(String, String)>,
    assistant_task: Option<LlmTask>,

    auto_gen_task: Option<LlmTask>,
    auto_gen_phase: AutoGenPhase,
    auto_gen_audit_text: String,
    auto_gen_log: Vec<String>,
    auto_gen_last_tick: Instant,

    // 手动「AI 生成本章」/「AI 生成下一章」：不自动落盘，生成完成后只填入编辑器等用户确认保存
    manual_gen_task: Option<LlmTask>,
    manual_gen_target: Option<i32>,
    /// 若当前章节已有正文需要二次确认覆盖，写入此字段；UI 渲染 modal 让用户确认。
    pending_gen_confirm: Option<i32>,

    // 「AI 生成 book_rules.md」专用任务（对齐 Narcooo/inkos 的 architect.bookRulesPrompt）。
    // 与 manual_gen_task 隔离，避免与正文生成互相阻塞。
    book_rules_gen_task: Option<LlmTask>,
    /// 若 `book_rules.md` 已被编辑过、再次触发生成时弹覆盖确认。
    pending_book_rules_confirm: bool,

    /// AI 刷新状态档案（当前文件 / 全部文件）；结果直接落盘，章节仍由用户手动保存。
    state_refresh_task: Option<LlmTask>,
    state_refresh_batch: Option<StateRefreshBatch>,
    /// 「生成下一章」流程中，若「章节保存后自动刷新长期记忆」被触发，
    /// 则将生成动作挂起到刷新完成后再执行，确保下一章能读到最新档案。
    pending_next_chapter_after_refresh: bool,

    history_open_for: Option<i32>,
    history_revisions: Vec<ChapterRevision>,
    history_selected_file: Option<String>,
    history_selected_body: String,

    oplog_dates: Vec<NaiveDate>,
    oplog_selected_date: Option<NaiveDate>,
    oplog_entries: Vec<OpLogEntry>,
    oplog_loaded_for: Option<NaiveDate>,

    pending_hooks_summary: Vec<String>,
    pending_hooks_mtime: Option<SystemTime>,
    pending_hooks_collapsed: bool,

    edit_vendor_id: Option<String>,
    edit_vendor_buf: VendorConfig,
    vendor_test_task: Option<LlmTask>,
    vendor_test_msg: String,

    // ---- 工具箱（Tools）----
    tools_tab: ToolsTab,
    // 全书搜索
    search_query: String,
    search_case_insensitive: bool,
    search_results: Vec<crate::search::SearchHit>,
    search_msg: String,
    // 导入
    import_source: String,
    import_text: String,
    import_regex: String,
    import_start_no: i32,
    import_overwrite: bool,
    import_report: Option<crate::import_chapters::ImportReport>,
    import_msg: String,
    // 导出
    export_format: crate::export::ExportFormat,
    export_msg: String,
    export_last_path: Option<PathBuf>,
    // 改名
    rename_from: String,
    rename_to: String,
    rename_report: Option<crate::rename::RenameReport>,
    rename_msg: String,
    // 写作管线
    pipeline_stage: crate::pipeline::PipelineStage,
    pipeline_chapter_no: i32,
    pipeline_prompt: String,
    pipeline_msg: String,
    // 文风
    style_input: String,
    style_source_label: String,
    style_fp: Option<crate::style::StyleFingerprint>,
    style_msg: String,
    // AIGC
    aigc_input: String,
    aigc_report: Option<crate::aigc::AigcReport>,
    aigc_use_current_chapter: bool,
    aigc_llm_prompt: String,
    // Analytics
    analytics_report: Option<crate::analytics::AnalyticsReport>,
    // Fanfic
    fanfic_sample: String,
    fanfic_brief: crate::fanfic::FanficBrief,
    fanfic_batch_size: i32,
    fanfic_total: i32,
    fanfic_minutes: i32,
    fanfic_plan: Option<crate::fanfic::BatchPlan>,
    fanfic_msg: String,

    status_message: String,
}

impl InkOsApp {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let paths = ConfigPaths::new();
        let (mut settings, load_warn) = paths.load_settings_with_warning();
        if settings.source_strategy.is_empty() {
            settings.source_strategy = "studio".into();
        }

        // 首启迁移：把遗留的 ~/.inkos/.env 作为一个 Studio vendor 导入；
        // 成功后 .env 改名归档，下次启动不再读取。
        let imported_vendor = paths.migrate_global_env_if_needed(&mut settings);

        // global_llm 保留字段以便「小说 .env 表单」的 resolved_with 兜底使用；
        // 迁移完成后此路径一般已归档，load_global_config 会返回 default（全空）。
        let global_llm = paths.load_global_config();

        eprintln!(
            "[inkos] settings.json path={} vendors={} migrated_global_env={}",
            paths.settings_path().display(),
            settings.vendors.len(),
            settings.migrated_global_env
        );

        let mut app = Self {
            paths,
            settings,
            section: Section::Project,
            init_done: false,
            global_llm,
            novel_llm: LlmConfig::default(),
            novel_path: None,
            store: None,
            project: None,
            selected_chapter: None,
            chapter_title: String::new(),
            chapter_body: String::new(),
            chapter_status: "draft".into(),
            chapter_summary: String::new(),
            chapter_dirty: false,
            chapter_summary_dirty: false,
            preview_md: String::new(),
            preview_deadline: None,
            cm_cache: CommonMarkCache::default(),
            nm_tab: NovelMetaTab::Basics,
            selected_state_file: STATE_FILES[0].into(),
            state_doc_body: String::new(),
            state_doc_dirty: false,
            state_doc_loaded_for: None,
            review_mode: ReviewMode::Audit,
            review_target: None,
            review_target_original: String::new(),
            review_compare: true,
            review_result: String::new(),
            review_for_chapter: None,
            review_task: None,
            audit_buf: String::new(),
            audit_buf_for_chapter: None,
            rewrite_buf: String::new(),
            rewrite_buf_for_chapter: None,
            audit_records: Vec::new(),
            selected_audit_ts: None,
            state_sync_task: None,
            state_sync_phase: StateSyncPhase::Idle,
            state_sync_log: Vec::new(),
            assistant_input: String::new(),
            assistant_log: Vec::new(),
            assistant_task: None,
            auto_gen_task: None,
            auto_gen_phase: AutoGenPhase::Idle,
            auto_gen_audit_text: String::new(),
            auto_gen_log: Vec::new(),
            auto_gen_last_tick: Instant::now(),
            manual_gen_task: None,
            manual_gen_target: None,
            pending_gen_confirm: None,
            book_rules_gen_task: None,
            pending_book_rules_confirm: false,
            state_refresh_task: None,
            state_refresh_batch: None,
            pending_next_chapter_after_refresh: false,
            history_open_for: None,
            history_revisions: Vec::new(),
            history_selected_file: None,
            history_selected_body: String::new(),
            oplog_dates: Vec::new(),
            oplog_selected_date: None,
            oplog_entries: Vec::new(),
            oplog_loaded_for: None,
            pending_hooks_summary: Vec::new(),
            pending_hooks_mtime: None,
            pending_hooks_collapsed: false,
            edit_vendor_id: None,
            edit_vendor_buf: VendorConfig::default(),
            vendor_test_task: None,
            vendor_test_msg: String::new(),

            tools_tab: ToolsTab::Search,
            search_query: String::new(),
            search_case_insensitive: false,
            search_results: Vec::new(),
            search_msg: String::new(),
            import_source: String::new(),
            import_text: String::new(),
            import_regex: crate::import_chapters::default_split_regex().to_string(),
            import_start_no: 1,
            import_overwrite: false,
            import_report: None,
            import_msg: String::new(),
            export_format: crate::export::ExportFormat::Epub,
            export_msg: String::new(),
            export_last_path: None,
            rename_from: String::new(),
            rename_to: String::new(),
            rename_report: None,
            rename_msg: String::new(),
            pipeline_stage: crate::pipeline::PipelineStage::Plan,
            pipeline_chapter_no: 1,
            pipeline_prompt: String::new(),
            pipeline_msg: String::new(),
            style_input: String::new(),
            style_source_label: String::new(),
            style_fp: None,
            style_msg: String::new(),
            aigc_input: String::new(),
            aigc_report: None,
            aigc_use_current_chapter: true,
            aigc_llm_prompt: String::new(),
            analytics_report: None,
            fanfic_sample: String::new(),
            fanfic_brief: crate::fanfic::FanficBrief::default(),
            fanfic_batch_size: 5,
            fanfic_total: 20,
            fanfic_minutes: 6,
            fanfic_plan: None,
            fanfic_msg: String::new(),

            status_message: "就绪".into(),
        };

        if !app.settings.default_novel_path.is_empty() {
            if let Some(p) = crate::config::normalize_path(&app.settings.default_novel_path) {
                if let Err(e) = app.load_novel_dir(p) {
                    app.status_message = format!("启动时加载默认目录失败：{e}");
                }
            }
        }

        // 启动诊断：若有迁移或 settings 损坏警告，优先展示。
        if let Some(w) = load_warn {
            app.status_message = w;
        } else if let Some(id) = imported_vendor {
            let label = Self::vendor_label(&id);
            app.status_message = format!("已从 ~/.inkos/.env 导入服务商：{label}");
        }

        app
    }

    fn load_novel_dir(&mut self, path: PathBuf) -> anyhow::Result<()> {
        let store = ProjectStore::new(path.clone());
        let project = store.load_project()?;
        self.novel_llm = self.paths.load_novel_config(store.root());
        self.novel_path = Some(path.clone());

        let latest = project.chapters.iter().map(|c| c.number).max();
        self.store = Some(store);
        self.project = Some(project);

        self.chapter_title.clear();
        self.chapter_body.clear();
        self.chapter_summary.clear();
        self.chapter_status = "draft".into();
        self.chapter_dirty = false;
        self.chapter_summary_dirty = false;
        self.preview_md.clear();
        self.preview_deadline = None;
        self.state_doc_body.clear();
        self.state_doc_dirty = false;
        self.state_doc_loaded_for = None;
        self.selected_chapter = None;
        self.auto_gen_log.clear();
        self.auto_gen_phase = AutoGenPhase::Idle;
        self.auto_gen_audit_text.clear();
        self.manual_gen_task = None;
        self.manual_gen_target = None;
        self.pending_gen_confirm = None;
        self.book_rules_gen_task = None;
        self.pending_book_rules_confirm = false;
        self.state_refresh_task = None;
        self.state_refresh_batch = None;
        self.pending_next_chapter_after_refresh = false;
        self.review_result.clear();
        self.review_target_original.clear();
        self.review_for_chapter = None;
        self.audit_buf.clear();
        self.audit_buf_for_chapter = None;
        self.rewrite_buf.clear();
        self.rewrite_buf_for_chapter = None;
        self.selected_audit_ts = None;
        self.audit_records = audit_log::list_all(&path);
        self.state_sync_task = None;
        self.state_sync_phase = StateSyncPhase::Idle;
        self.state_sync_log.clear();
        self.history_open_for = None;
        self.history_revisions.clear();
        self.history_selected_file = None;
        self.history_selected_body.clear();
        self.oplog_dates.clear();
        self.oplog_selected_date = None;
        self.oplog_entries.clear();
        self.oplog_loaded_for = None;
        self.pending_hooks_summary.clear();
        self.pending_hooks_mtime = None;

        if let Some(n) = latest {
            self.select_chapter(n);
        }

        // 兼容 Narcooo/inkos 的 Input Governance：在新版项目里默认创建 story/ 子目录。
        if let Some(ref root) = self.novel_path {
            let _ = crate::intent::ensure_default_templates(root);
        }
        self.search_results.clear();
        self.search_msg.clear();
        self.import_report = None;
        self.import_msg.clear();
        self.export_msg.clear();
        self.export_last_path = None;
        self.rename_report = None;
        self.rename_msg.clear();
        self.pipeline_prompt.clear();
        self.pipeline_msg.clear();
        self.style_fp = if let Some(ref root) = self.novel_path {
            crate::style::load(root)
        } else {
            None
        };
        self.style_msg.clear();
        self.aigc_report = None;
        self.aigc_input.clear();
        self.aigc_llm_prompt.clear();
        self.analytics_report = None;
        self.fanfic_brief = crate::fanfic::FanficBrief::default();
        self.fanfic_plan = None;
        self.fanfic_msg.clear();

        self.settings.default_novel_path = self
            .novel_path
            .as_ref()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let mut recent = vec![self.settings.default_novel_path.clone()];
        recent.extend(self.settings.recent_novels.clone());
        self.settings.recent_novels = unique_paths(&recent);
        self.settings.recent_novels.truncate(12);
        self.paths.save_settings(&self.settings)?;

        self.status_message = format!("已载入：{}", self.novel_path.as_ref().unwrap().display());
        if let Some(ref root) = self.novel_path {
            oplog::try_append(Some(root), "打开项目", &root.display().to_string());
        }
        self.refresh_pending_hooks();
        Ok(())
    }

    #[allow(dead_code)]
    fn save_global_config_ui(&mut self) {
        // 废弃：`~/.inkos/.env` 的 LLM 覆盖已取消，首启会迁移到 Studio vendor 并归档原文件。
        // 保留函数供可能的手工回滚使用，不再有 UI 调用点。
        match self.paths.save_global_config(&self.global_llm) {
            Ok(()) => self.status_message = "已保存全局配置".into(),
            Err(e) => self.status_message = format!("保存全局配置失败：{e}"),
        }
    }

    fn save_novel_config_ui(&mut self) {
        let Some(ref root) = self.novel_path else {
            self.status_message = "未打开小说目录".into();
            return;
        };
        match self.paths.save_novel_config(root, &self.novel_llm) {
            Ok(()) => self.status_message = "已保存小说目录 .env".into(),
            Err(e) => self.status_message = format!("保存小说配置失败：{e}"),
        }
    }

    fn save_novel_meta(&mut self) {
        let (Some(store), Some(project)) = (&self.store, &mut self.project) else {
            self.status_message = "未打开项目".into();
            return;
        };
        match store.save_project(project) {
            Ok(()) => self.status_message = "已保存小说资料".into(),
            Err(e) => self.status_message = format!("保存失败：{e}"),
        }
    }

    fn select_chapter(&mut self, n: i32) {
        let Some(ref store) = self.store else { return };
        self.selected_chapter = Some(n);
        match store.load_chapter_content(n) {
            Ok((t, b)) => {
                self.chapter_title = t;
                self.chapter_body = b;
                if let Some(ref p) = self.project {
                    if let Some(rec) = p.chapters.iter().find(|c| c.number == n) {
                        self.chapter_status = rec.status.clone();
                        self.chapter_summary = rec.summary.clone();
                    } else {
                        self.chapter_status = "draft".into();
                        self.chapter_summary.clear();
                    }
                }
                self.chapter_dirty = false;
                self.chapter_summary_dirty = false;
                self.preview_md = self.chapter_body.clone();
                self.preview_deadline = None;
            }
            Err(e) => self.status_message = format!("读取章节失败：{e}"),
        }
    }

    fn save_current_chapter(&mut self) {
        let (Some(store), Some(project), Some(n)) =
            (&self.store, &mut self.project, self.selected_chapter)
        else {
            self.status_message = "请先选择章节".into();
            return;
        };
        // 若磁盘已存在内容且与新内容不同，先 snapshot
        let novel_root = self.novel_path.clone();
        let body_changed = self.chapter_dirty;
        if body_changed {
            if let Some(ref root) = novel_root {
                if let Ok((old_title, old_body)) = store.load_chapter_content(n) {
                    if !old_body.is_empty() && old_body.trim() != self.chapter_body.trim() {
                        if let Err(e) = history::snapshot_chapter(
                            root,
                            n,
                            &old_title,
                            &old_body,
                            &self.chapter_body,
                            "manual",
                            "保存章节前自动备份原版",
                        ) {
                            self.status_message = format!("自动备份失败：{e}");
                        }
                    }
                }
            }
        }
        match store.save_chapter(
            project,
            n,
            &self.chapter_title,
            &self.chapter_body,
            &self.chapter_status,
            &self.chapter_summary,
        ) {
            Ok(()) => {
                self.chapter_dirty = false;
                self.chapter_summary_dirty = false;
                self.preview_md = self.chapter_body.clone();
                if let Some(ref p) = self.project {
                    if let Some(rec) = p.chapters.iter().find(|c| c.number == n) {
                        self.chapter_summary = rec.summary.clone();
                    }
                }
                self.status_message = format!("已保存第 {n} 章");
                let words = self.chapter_body.chars().filter(|c| !c.is_whitespace()).count();
                oplog::try_append(
                    novel_root.as_deref(),
                    "保存章节",
                    &format!("第 {n} 章 · {} 字", words),
                );

                // 保存成功后自动同步「长期记忆档案」（对齐 inkoswin 的连续性前提）。
                // 否则 [连续性档案] 会滞后于实际章节推进，导致「生成下一章」与前文对不上。
                if self.settings.auto_refresh_state_after_chapter_save {
                    self.auto_refresh_state_after_chapter_saved(n);
                }
            }
            Err(e) => self.status_message = format!("保存章节失败：{e}"),
        }
    }

    /// 保存章节后尝试自动触发「AI 刷新全部长期记忆档案」。
    ///
    /// 策略：
    /// - 若没打开项目 / 写作 LLM 未配置 / 其它 LLM 任务在跑 / 状态档案有未保存改动，
    ///   则**静默跳过**（仅 OpLog 记一条跳过原因），避免打断用户。
    /// - 否则复用 `try_start_state_refresh(REFRESH_ALL_ORDER)` 链路，
    ///   在后台顺序刷新 novel_brief / current_state / pending_hooks / subplot_board /
    ///   emotional_arcs / character_matrix / particle_ledger，并本地重建
    ///   chapter_summaries.md（`book_rules.md` 显式排除）。
    fn auto_refresh_state_after_chapter_saved(&mut self, n: i32) {
        let novel_root = self.novel_path.clone();

        if let Some(reason) = self.state_refresh_prereq_reason() {
            oplog::try_append(
                novel_root.as_deref(),
                "AI 刷新长期记忆 · 自动跳过",
                &format!("第 {n} 章保存后：{reason}"),
            );
            return;
        }
        let vendor_id = self.settings.writing_vendor.clone();
        if vendor_id.is_empty() {
            oplog::try_append(
                novel_root.as_deref(),
                "AI 刷新长期记忆 · 自动跳过",
                "未选择写作 LLM",
            );
            return;
        }
        let cfg_ok = self
            .vendor_config(&vendor_id)
            .map(|c| c.is_configured())
            .unwrap_or(false);
        if !cfg_ok {
            oplog::try_append(
                novel_root.as_deref(),
                "AI 刷新长期记忆 · 自动跳过",
                "写作 LLM 服务商未完整配置",
            );
            return;
        }

        oplog::try_append(
            novel_root.as_deref(),
            "AI 刷新长期记忆 · 自动触发",
            &format!("第 {n} 章保存后"),
        );
        let queue: Vec<String> = crate::state_refresh::REFRESH_ALL_ORDER
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        self.try_start_state_refresh(queue);
    }

    /// 写作工具栏「🔁 保存并同步记忆」按钮入口：
    /// 1) 保存当前章节；
    /// 2) 无论 `auto_refresh_state_after_chapter_save` 如何，都尝试触发一次
    ///    「AI 刷新全部长期记忆档案」（前提检查不通过时会将原因写到 status + OpLog）。
    fn save_and_sync_memory(&mut self) {
        let had_dirty = self.chapter_dirty || self.chapter_summary_dirty;
        let prev_auto = self.settings.auto_refresh_state_after_chapter_save;
        if had_dirty {
            // 临时关闭自动刷新，避免 save_current_chapter 里再触发一次（下面会统一走一条）。
            self.settings.auto_refresh_state_after_chapter_save = false;
            self.save_current_chapter();
            self.settings.auto_refresh_state_after_chapter_save = prev_auto;
            if self.chapter_dirty || self.chapter_summary_dirty {
                // 保存失败，状态栏已有提示，不再继续。
                return;
            }
        }
        let Some(n) = self.selected_chapter else {
            self.status_message = "请先选择章节".into();
            return;
        };
        // 走前置检查；若不满足，显式抛出原因（区别于自动模式的静默跳过）。
        if let Some(reason) = self.state_refresh_prereq_reason() {
            self.status_message = format!("无法同步长期记忆：{reason}");
            oplog::try_append(
                self.novel_path.as_deref(),
                "AI 刷新长期记忆 · 手动跳过",
                &format!("第 {n} 章保存后：{reason}"),
            );
            return;
        }
        oplog::try_append(
            self.novel_path.as_deref(),
            "AI 刷新长期记忆 · 手动触发",
            &format!("第 {n} 章 · 保存并同步"),
        );
        let queue: Vec<String> = crate::state_refresh::REFRESH_ALL_ORDER
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        self.try_start_state_refresh(queue);
    }

    fn new_chapter(&mut self) {
        let (Some(store), Some(project)) = (&self.store, &mut self.project) else { return };
        match store.create_next_chapter(project, "") {
            Ok(n) => {
                oplog::try_append(self.novel_path.as_deref(), "新建章节", &format!("第 {n} 章"));
                self.select_chapter(n);
                self.chapter_dirty = true;
                self.status_message = format!("已新建第 {n} 章");
            }
            Err(e) => self.status_message = format!("新建章节失败：{e}"),
        }
    }

    /// 把 `selected_state_file` 解析成实际磁盘路径。
    /// - `book_rules.md` 等裸文件名 → `<root>/story_state/`
    /// - `story/...` 前缀 → `<root>/story/...`
    fn state_doc_path(&self) -> Option<PathBuf> {
        let store = self.store.as_ref()?;
        let name = self.selected_state_file.trim();
        if name.is_empty() {
            return None;
        }
        if let Some(rel) = name.strip_prefix("story/") {
            Some(store.root().join("story").join(rel))
        } else {
            Some(store.state_dir().join(name))
        }
    }

    fn load_state_doc(&mut self) {
        let Some(path) = self.state_doc_path() else {
            self.status_message = "未打开小说目录".into();
            return;
        };
        // 若是 story/ 文件且不存在，则用模板初始化（不会覆盖已存在文件）。
        if path.starts_with(self.novel_path.as_deref().unwrap_or(std::path::Path::new("")))
            && !path.exists()
        {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let initial = match self.selected_state_file.as_str() {
                "story/author_intent.md" => crate::intent::AUTHOR_INTENT_TEMPLATE,
                "story/current_focus.md" => crate::intent::CURRENT_FOCUS_TEMPLATE,
                _ => "",
            };
            if !initial.is_empty() {
                let _ = std::fs::write(&path, initial);
            }
        }
        match fs::read_to_string(&path) {
            Ok(text) => {
                self.state_doc_body = text;
                self.state_doc_dirty = false;
                self.state_doc_loaded_for = Some(self.selected_state_file.clone());
                self.status_message = format!("已载入：{}", path.display());
            }
            Err(_) => {
                self.state_doc_body.clear();
                self.state_doc_dirty = false;
                self.state_doc_loaded_for = Some(self.selected_state_file.clone());
            }
        }
    }

    fn save_state_doc(&mut self) {
        let Some(path) = self.state_doc_path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let body = self.state_doc_body.trim_end().to_string() + "\n";
        match fs::write(&path, body) {
            Ok(()) => {
                self.state_doc_dirty = false;
                self.status_message = format!("已保存：{}", path.display());
                oplog::try_append(
                    self.novel_path.as_deref(),
                    "保存状态档案",
                    &self.selected_state_file,
                );
                if self.selected_state_file == "pending_hooks.md" {
                    self.refresh_pending_hooks();
                    let n = self.pending_hooks_summary.len();
                    self.status_message = format!("📌 伏笔提醒已更新：当前共 {n} 条待回收");
                }
            }
            Err(e) => self.status_message = format!("保存状态文件失败：{e}"),
        }
    }

    fn state_refresh_prereq_reason(&self) -> Option<String> {
        if self.store.is_none() || self.project.is_none() {
            return Some("未打开项目".into());
        }
        if self.state_refresh_task.is_some() || self.state_refresh_batch.is_some() {
            return Some("状态档案刷新正在进行".into());
        }
        if self.manual_gen_task.is_some() {
            return Some("请先完成或取消章节生成".into());
        }
        if self.book_rules_gen_task.is_some() {
            return Some("请先完成或取消 book_rules 生成".into());
        }
        if self.state_sync_task.is_some() {
            return Some("请先完成状态档案同步任务".into());
        }
        if self.auto_gen_task.is_some() {
            return Some("请先完成或取消定时写作".into());
        }
        if self.chapter_dirty || self.chapter_summary_dirty {
            return Some("请先保存当前章节".into());
        }
        if self.state_doc_dirty {
            return Some("请先保存或重新载入当前状态档案".into());
        }
        None
    }

    fn cancel_state_refresh(&mut self) {
        if self.state_refresh_task.is_none() && self.state_refresh_batch.is_none() {
            return;
        }
        self.state_refresh_task = None;
        self.state_refresh_batch = None;
        let had_pending = self.pending_next_chapter_after_refresh;
        self.pending_next_chapter_after_refresh = false;
        self.status_message = if had_pending {
            "已取消状态档案刷新；「生成下一章」挂起任务也一并取消".into()
        } else {
            "已取消状态档案刷新".into()
        };
        oplog::try_append(
            self.novel_path.as_deref(),
            "AI 刷新状态档案 · 取消",
            "",
        );
    }

    fn abort_state_refresh(&mut self, msg: impl Into<String>) {
        let s = msg.into();
        self.state_refresh_task = None;
        self.state_refresh_batch = None;
        self.pending_next_chapter_after_refresh = false;
        self.status_message = format!("✗ {s}");
        oplog::try_append(
            self.novel_path.as_deref(),
            "AI 刷新状态档案 · 失败",
            &s,
        );
    }

    fn start_state_refresh_current(&mut self) {
        if !crate::state_refresh::is_ai_refreshable(&self.selected_state_file) {
            self.status_message =
                "当前文件不支持 AI 刷新（book_rules 与 story/ 下文件请手动维护）".into();
            return;
        }
        self.try_start_state_refresh(vec![self.selected_state_file.clone()]);
    }

    fn start_state_refresh_all(&mut self) {
        let queue: Vec<String> = crate::state_refresh::REFRESH_ALL_ORDER
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        self.try_start_state_refresh(queue);
    }

    fn try_start_state_refresh(&mut self, queue: Vec<String>) {
        if let Some(msg) = self.state_refresh_prereq_reason() {
            self.status_message = msg;
            return;
        }
        let vendor_id = self.settings.writing_vendor.clone();
        if vendor_id.is_empty() {
            self.status_message = "未选择写作 LLM（设置 → 写作）".into();
            return;
        }
        let Some(cfg) = self.vendor_config(&vendor_id) else {
            self.status_message = format!("服务商「{vendor_id}」配置缺失");
            return;
        };
        if !cfg.is_configured() {
            self.status_message = "写作 LLM 服务商未完整配置".into();
            return;
        }

        self.save_novel_meta();
        let Some(store) = self.store.as_ref() else {
            self.status_message = "未打开项目".into();
            return;
        };
        let Some(project_snapshot) = self.project.as_ref().map(|p| p.clone()) else {
            self.status_message = "未打开项目".into();
            return;
        };
        let chapter_digest = store.build_chapter_digest(&project_snapshot, 12);
        let state_documents = match store.load_story_state_documents_map(&project_snapshot) {
            Ok(m) => m,
            Err(e) => {
                self.status_message = format!("载入状态档案失败：{e}");
                return;
            }
        };

        oplog::try_append(
            self.novel_path.as_deref(),
            "AI 刷新状态档案 · 开始",
            &queue.join(", "),
        );
        self.state_refresh_batch = Some(StateRefreshBatch {
            queue,
            index: 0,
            state_documents,
            chapter_digest,
            project_snapshot,
        });
        self.kick_state_refresh_step();
    }

    fn kick_state_refresh_step(&mut self) {
        if self.state_refresh_task.is_some() {
            return;
        }
        let Some(store) = self.store.as_ref() else {
            self.abort_state_refresh("内部错误：无 ProjectStore");
            return;
        };

        let done = self
            .state_refresh_batch
            .as_ref()
            .map(|b| b.index >= b.queue.len())
            .unwrap_or(true);
        if done {
            if self.state_refresh_batch.is_some() {
                self.finish_state_refresh_batch_success();
            }
            return;
        }

        let fname = {
            let b = self.state_refresh_batch.as_ref().unwrap();
            b.queue[b.index].clone()
        };

        if fname == "chapter_summaries.md" {
            let body = {
                let snap = &self.state_refresh_batch.as_ref().unwrap().project_snapshot;
                store.build_chapter_summaries_document(snap)
            };
            if let Err(e) = store.write_story_state_file("chapter_summaries.md", &body) {
                self.abort_state_refresh(format!("写入 chapter_summaries.md 失败：{e}"));
                return;
            }
            {
                let b = self.state_refresh_batch.as_mut().unwrap();
                b.state_documents.insert(fname.clone(), body);
                b.index += 1;
            }
            oplog::try_append(
                self.novel_path.as_deref(),
                "AI 刷新状态档案 · 单文件",
                &format!("{fname}（本地重建，未调用 LLM）"),
            );
            self.kick_state_refresh_step();
            return;
        }

        let Some(spec) = crate::state_refresh::spec_for(&fname) else {
            self.abort_state_refresh(format!("未知状态文件：{fname}"));
            return;
        };

        let (system_prompt, user_prompt, idx_one_based, total) = {
            let b = self.state_refresh_batch.as_ref().unwrap();
            let current_content = b.state_documents.get(&fname).cloned().unwrap_or_default();
            let (sys, user) = crate::state_refresh::build_state_document_prompts(
                &b.project_snapshot,
                spec,
                &current_content,
                &b.chapter_digest,
                &b.state_documents,
            );
            (sys, user, b.index + 1, b.queue.len())
        };

        let vendor_id = self.settings.writing_vendor.clone();
        let Some(cfg) = self.vendor_config(&vendor_id) else {
            self.abort_state_refresh(format!("服务商「{vendor_id}」配置缺失"));
            return;
        };
        if !cfg.is_configured() {
            self.abort_state_refresh("写作 LLM 未完整配置");
            return;
        }
        let messages = vec![
            ChatMessage::system(&system_prompt),
            ChatMessage::user(user_prompt),
        ];
        let model = cfg.model.clone();
        self.state_refresh_task = Some(spawn_chat(
            cfg,
            vendor_id,
            model,
            messages,
            false,
        ));
        self.status_message =
            format!("🔄 AI 刷新档案：{fname}（{idx_one_based}/{total}）");
    }

    fn handle_state_refresh_task_done(&mut self) {
        let Some(task) = self.state_refresh_task.take() else {
            return;
        };
        let Some(fname) = self.state_refresh_batch.as_ref().and_then(|b| {
            if b.index >= b.queue.len() {
                None
            } else {
                Some(b.queue[b.index].clone())
            }
        }) else {
            return;
        };

        if let Some(err) = task.error.clone() {
            self.abort_state_refresh(format!("{fname}：{err}"));
            return;
        }
        let raw = task.accumulated.trim();
        if raw.is_empty() {
            self.abort_state_refresh(format!("{fname}：模型返回为空"));
            return;
        }
        let cleaned = crate::state_refresh::sanitize_model_markdown(raw);
        let Some(store) = self.store.as_ref() else {
            self.abort_state_refresh("内部错误：无 ProjectStore");
            return;
        };
        let mut skipped_write = false;
        {
            let b = self.state_refresh_batch.as_mut().unwrap();
            if b
                .state_documents
                .get(&fname)
                .map(|old| old.trim() == cleaned.trim())
                .unwrap_or(false)
            {
                skipped_write = true;
            } else {
                if let Err(e) = store.write_story_state_file(&fname, &cleaned) {
                    self.abort_state_refresh(format!("写入 {fname} 失败：{e}"));
                    return;
                }
            }
            b.state_documents.insert(fname.clone(), cleaned);
            b.index += 1;
        }
        oplog::try_append(
            self.novel_path.as_deref(),
            "AI 刷新状态档案 · 单文件",
            &format!("{fname}（LLM{}）", if skipped_write { "，内容无变化跳过写盘" } else { "" }),
        );
        self.kick_state_refresh_step();
    }

    fn finish_state_refresh_batch_success(&mut self) {
        let queue: Vec<String> = self
            .state_refresh_batch
            .as_ref()
            .map(|b| b.queue.clone())
            .unwrap_or_default();
        let files_label = queue.join(", ");
        let selected = self.selected_state_file.clone();

        self.state_refresh_task = None;
        self.state_refresh_batch = None;

        if let Some(store) = self.store.as_ref() {
            if let Ok(p) = store.load_project() {
                self.project = Some(p);
            }
        }
        if queue.iter().any(|f| f == &selected) {
            self.load_state_doc();
        }
        self.status_message = format!("✓ 状态档案刷新完成：{files_label}");
        oplog::try_append(
            self.novel_path.as_deref(),
            "AI 刷新状态档案 · 完成",
            &files_label,
        );
        self.refresh_pending_hooks();

        // 若之前「生成下一章」被刷新任务挂起，则此时恢复生成。
        if self.pending_next_chapter_after_refresh {
            self.pending_next_chapter_after_refresh = false;
            oplog::try_append(
                self.novel_path.as_deref(),
                "AI 生成下一章 · 恢复",
                "长期记忆已刷新完成",
            );
            self.resume_generate_next_chapter();
        }
    }

    /// 有效 vendor 配置：基础是 `settings.vendors[id]`，若当前打开了小说且
    /// `<novel>/.env` 里有非空字段，则字段级覆盖到 Studio vendor 之上。
    ///
    /// 这是运行时分层的核心：Studio = 全局，小说 .env = 项目级覆盖。
    fn vendor_config(&self, id: &str) -> Option<VendorConfig> {
        let mut cfg = self.settings.vendors.get(id).cloned()?;
        if self.novel_path.is_some() {
            cfg.overlay_llm(&self.novel_llm);
        }
        Some(cfg)
    }

    fn configured_vendor_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .settings
            .vendors
            .iter()
            .filter(|(_, v)| v.is_configured())
            .map(|(k, _)| k.clone())
            .collect();
        ids.sort();
        ids
    }

    fn vendor_label(id: &str) -> String {
        find_vendor(id).map(|v| v.name.to_string()).unwrap_or_else(|| id.to_string())
    }
}

impl eframe::App for InkOsApp {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        let ctx = &ctx;

        if !self.init_done {
            install_cjk_fonts(ctx);
            theme::apply(ctx);
            self.init_done = true;
        }

        let time = ctx.input(|i| i.time);
        if let Some(deadline) = self.preview_deadline {
            if time >= deadline {
                self.preview_md = self.chapter_body.clone();
                self.preview_deadline = None;
            }
        }

        self.poll_llm_tasks(ctx);
        self.tick_auto_gen(ctx);

        egui::Panel::bottom("status_bar")
            .frame(
                egui::Frame::default()
                    .fill(color::PANEL)
                    .inner_margin(Margin::symmetric(16, 6))
                    .stroke(Stroke::new(1.0, color::BORDER)),
            )
            .show_inside(root, |ui| {
                ui.horizontal(|ui| {
                    let dirty = self.chapter_dirty || self.state_doc_dirty || self.chapter_summary_dirty;
                    let dot = if dirty {
                        ("●  有未保存改动", color::WARNING)
                    } else {
                        ("●  就绪", color::SUCCESS)
                    };
                    ui.label(RichText::new(dot.0).color(dot.1).size(11.5));
                    ui.add_space(12.0);
                    ui.label(RichText::new(&self.status_message).color(color::TEXT_DIM).size(11.5));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new("InkOS Desktop · v0.1.0").color(color::TEXT_FAINT).size(11.0));
                    });
                });
            });

        egui::Panel::left("nav_bar")
            .exact_size(220.0)
            .resizable(false)
            .frame(
                egui::Frame::default()
                    .fill(color::PANEL)
                    .inner_margin(Margin::symmetric(14, 16))
                    .stroke(Stroke::new(1.0, color::BORDER)),
            )
            .show_inside(root, |ui| self.ui_sidebar(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::default().fill(color::BG).inner_margin(Margin::same(28)))
            .show_inside(root, |ui| {
                let (title, sub) = self.section.header();
                page_header(ui, title, sub);

                match self.section {
                    Section::Project => self.ui_project(ui),
                    Section::Writing => self.ui_writing(ui, time),
                    Section::Review => self.ui_review(ui),
                    Section::Assistant => self.ui_assistant(ui),
                    Section::NovelMeta => self.ui_novel_meta(ui),
                    Section::OpLog => self.ui_oplog(ui),
                    Section::Tools => self.ui_tools(ui),
                    Section::Settings => self.ui_settings(ui),
                }
            });

        self.show_vendor_editor(ctx);
        self.show_chapter_history_modal(ctx);
        self.show_gen_confirm_modal(ctx);
        self.show_book_rules_confirm_modal(ctx);
    }
}

impl InkOsApp {
    fn poll_llm_tasks(&mut self, ctx: &egui::Context) {
        let mut any_running = false;

        if let Some(task) = &mut self.review_task {
            task.drain();
            // 始终把累计内容同步到 review_result（修复完成时丢失最后一帧增量的 bug）
            self.review_result = task.accumulated.clone();
            if task.done {
                if let Some(e) = task.error.clone() {
                    if self.review_result.trim().is_empty() {
                        self.review_result = format!("（请求失败）{e}");
                    } else {
                        self.review_result.push_str(&format!("\n\n（请求中断）{e}"));
                    }
                }
                let mode_label = match self.review_mode {
                    ReviewMode::Audit => "AI 审计",
                    ReviewMode::Rewrite => "AI 改写",
                };
                let mode_key = match self.review_mode {
                    ReviewMode::Audit => "audit",
                    ReviewMode::Rewrite => "rewrite",
                };
                let n = self.review_for_chapter.unwrap_or(0);
                let chars = self.review_result.chars().count();
                let elapsed = task.elapsed_secs();
                let task_error = task.error.clone();
                let vendor_id = task.vendor_id.clone();
                let model = task.model.clone();
                let detail = if let Some(ref e) = task_error {
                    format!("第 {n} 章 失败：{e}")
                } else {
                    format!("第 {n} 章 完成 · {chars} 字 · {elapsed:.1}s")
                };
                oplog::try_append(self.novel_path.as_deref(), mode_label, &detail);
                self.status_message = format!("{mode_label} 完成：{detail}");
                // 写入审计/改写历史记录（仅在成功且有内容时）
                if task_error.is_none() && !self.review_result.trim().is_empty() && n > 0 {
                    if let Some(ref root) = self.novel_path {
                        let vendor_label = Self::vendor_label(&vendor_id);
                        match audit_log::append(
                            root,
                            mode_key,
                            n,
                            &vendor_id,
                            &vendor_label,
                            &model,
                            elapsed,
                            &self.review_result,
                        ) {
                            Ok(rec) => {
                                self.audit_records.insert(0, rec);
                            }
                            Err(e) => {
                                self.status_message =
                                    format!("{mode_label} 完成，但写入审计记录失败：{e}");
                            }
                        }
                    }
                }
                self.review_task = None;
            } else {
                any_running = true;
            }
        }
        if let Some(task) = &mut self.state_sync_task {
            task.drain();
            if task.done {
                self.handle_state_sync_done();
            } else {
                any_running = true;
            }
        }
        if let Some(task) = &mut self.assistant_task {
            task.drain();
            if task.done {
                let acc = std::mem::take(&mut task.accumulated);
                let err = task.error.clone();
                let final_msg = if let Some(e) = err {
                    if acc.is_empty() {
                        format!("（请求失败）{e}")
                    } else {
                        format!("{acc}\n\n（请求中断）{e}")
                    }
                } else {
                    acc
                };
                self.assistant_log.push(("assistant".into(), final_msg));
                self.assistant_task = None;
            } else {
                any_running = true;
            }
        }
        if let Some(task) = &mut self.auto_gen_task {
            task.drain();
            if task.done {
                self.handle_auto_gen_done();
            } else {
                any_running = true;
            }
        }
        if let Some(task) = &mut self.manual_gen_task {
            task.drain();
            let stream_snapshot = if !task.done && !task.accumulated.is_empty() {
                Some(task.accumulated.clone())
            } else {
                None
            };
            if task.done {
                self.handle_manual_gen_done();
            } else {
                any_running = true;
            }
            if let Some(raw) = stream_snapshot {
                // 流式预览也用 inkoswin 解析器；这样「标题：/摘要：/正文：」格式能在一开始就把
                // 标题与摘要及时展示出来，正文部分逐步填充。
                let fallback_title = if !self.chapter_title.trim().is_empty() {
                    self.chapter_title.clone()
                } else if let Some(n) = self.manual_gen_target {
                    format!("第{n}章")
                } else {
                    String::new()
                };
                let result = inkoswin_prompt::parse_generation_output(raw.trim(), &fallback_title);
                if !result.title.trim().is_empty() {
                    self.chapter_title = result.title;
                }
                self.chapter_body = result.content;
                // 流式阶段的摘要只做预览，不覆盖用户未保存摘要（避免抖动）；完成时再一次性写入。
                self.preview_md = self.chapter_body.clone();
            }
        }
        if let Some(task) = &mut self.book_rules_gen_task {
            task.drain();
            let snapshot = if !task.done && !task.accumulated.is_empty() {
                Some(task.accumulated.clone())
            } else {
                None
            };
            if task.done {
                self.handle_book_rules_gen_done();
            } else {
                any_running = true;
            }
            if let Some(raw) = snapshot {
                // 流式：实时把 LLM 当前累积的内容剥掉前言/代码围栏后落到编辑器，
                // 用户可立刻看到 frontmatter 一行一行长出来。
                let cleaned = crate::book_rules::parse_generation_output(&raw);
                self.state_doc_body = cleaned;
                self.state_doc_dirty = true;
            }
        }
        if let Some(task) = &mut self.state_refresh_task {
            task.drain();
            if task.done {
                self.handle_state_refresh_task_done();
            } else {
                any_running = true;
            }
        }
        if let Some(task) = &mut self.vendor_test_task {
            task.drain();
            if task.done {
                self.vendor_test_msg = if let Some(e) = task.error.clone() {
                    format!("✗  失败：{e}")
                } else {
                    format!("✓  成功：{}", first_line(&task.accumulated))
                };
                self.vendor_test_task = None;
            } else {
                any_running = true;
            }
        }

        if any_running {
            ctx.request_repaint_after(std::time::Duration::from_millis(120));
        }
    }

    fn collect_state_docs_for_prompt(&self, max_chars_each: usize) -> String {
        let Some(store) = self.store.as_ref() else {
            return String::new();
        };
        let mut out = String::new();
        for f in STATE_FILES {
            // 摘要单独传，避免重复
            if *f == "chapter_summaries.md" {
                continue;
            }
            let path = store.state_dir().join(f);
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let trimmed = text.trim();
            if trimmed.is_empty() {
                continue;
            }
            let truncated = if trimmed.chars().count() > max_chars_each {
                let cut: String = trimmed.chars().take(max_chars_each).collect();
                format!("{cut}\n…（已截断）")
            } else {
                trimmed.to_string()
            };
            out.push_str(&format!("\n### {f}\n{truncated}\n"));
        }
        out
    }

    fn refresh_pending_hooks(&mut self) {
        self.pending_hooks_summary.clear();
        self.pending_hooks_mtime = None;
        let Some(store) = self.store.as_ref() else { return };
        let path = store.state_dir().join("pending_hooks.md");
        let mtime = fs::metadata(&path).and_then(|m| m.modified()).ok();
        let Ok(text) = fs::read_to_string(&path) else { return };
        let mut items: Vec<String> = Vec::new();
        for line in text.lines() {
            let l = line.trim_start();
            if let Some(rest) = l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")) {
                let r = rest.trim();
                if !r.is_empty() && r.chars().count() > 1 {
                    items.push(r.to_string());
                    if items.len() >= 12 {
                        break;
                    }
                }
            }
        }
        self.pending_hooks_summary = items;
        self.pending_hooks_mtime = mtime;
    }

    fn maybe_refresh_pending_hooks(&mut self) {
        let Some(store) = self.store.as_ref() else { return };
        let path = store.state_dir().join("pending_hooks.md");
        let now_mtime = fs::metadata(&path).and_then(|m| m.modified()).ok();
        if now_mtime != self.pending_hooks_mtime {
            let was = self.pending_hooks_mtime.is_some();
            let prev = self.pending_hooks_summary.clone();
            self.refresh_pending_hooks();
            if was && self.pending_hooks_summary != prev {
                let count = self.pending_hooks_summary.len();
                oplog::try_append(
                    self.novel_path.as_deref(),
                    "伏笔提醒",
                    &format!("pending_hooks.md 已更新，当前共 {count} 条待回收"),
                );
                self.status_message =
                    format!("📌 伏笔提醒已刷新：当前共 {count} 条待回收");
                self.pending_hooks_collapsed = false;
            }
        }
    }

    fn refresh_oplog(&mut self) {
        let Some(root) = self.novel_path.as_ref() else {
            self.oplog_dates.clear();
            self.oplog_entries.clear();
            return;
        };
        self.oplog_dates = oplog::list_dates(root);
        if self.oplog_selected_date.is_none() {
            self.oplog_selected_date = self.oplog_dates.first().copied();
        }
        if let Some(d) = self.oplog_selected_date {
            if self.oplog_loaded_for != Some(d) {
                self.oplog_entries = oplog::read_day(root, d);
                self.oplog_loaded_for = Some(d);
            }
        }
    }

    fn ui_sidebar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            egui::Frame::default()
                .fill(color::ACCENT)
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    ui.label(RichText::new("Ink").color(Color32::WHITE).size(15.0).strong());
                });
            ui.vertical(|ui| {
                ui.label(RichText::new("InkOS Desktop").color(color::TEXT).size(14.0).strong());
                ui.label(RichText::new("Novel Workbench").color(color::TEXT_FAINT).size(10.5));
            });
        });

        ui.add_space(14.0);
        self.ui_project_card(ui);

        ui.add_space(10.0);
        section_label(ui, "Workspace");
        Self::nav_item(ui, &mut self.section, Section::Project, "项目");
        Self::nav_item(ui, &mut self.section, Section::Writing, "写作");
        Self::nav_item(ui, &mut self.section, Section::Review, "审核");
        Self::nav_item(ui, &mut self.section, Section::Assistant, "写作助手");

        ui.add_space(6.0);
        section_label(ui, "Project");
        Self::nav_item(ui, &mut self.section, Section::NovelMeta, "小说设定");
        Self::nav_item(ui, &mut self.section, Section::OpLog, "操作日志");
        Self::nav_item(ui, &mut self.section, Section::Tools, "工具箱");

        let bottom_h = 60.0;
        let avail = ui.available_height();
        if avail > bottom_h {
            ui.add_space(avail - bottom_h);
        }
        ui.separator();
        ui.add_space(4.0);
        Self::nav_item(ui, &mut self.section, Section::Settings, "设置");
    }

    fn ui_project_card(&self, ui: &mut egui::Ui) {
        egui::Frame::default()
            .fill(color::SURFACE)
            .stroke(Stroke::new(1.0, color::BORDER))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::same(10))
            .show(ui, |ui| {
                if let Some(ref project) = self.project {
                    ui.label(
                        RichText::new(if project.title.is_empty() {
                            "未命名".into()
                        } else {
                            project.title.clone()
                        })
                        .color(color::TEXT)
                        .size(13.5)
                        .strong(),
                    );
                    if !project.genre.is_empty() {
                        ui.label(RichText::new(&project.genre).color(color::TEXT_DIM).size(11.0));
                    }
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        theme::pill(
                            ui,
                            &format!("{} 章", project.chapters.len()),
                            color::ACCENT_HI,
                            color::ACCENT_DIM,
                        );
                        let total: i32 = project.chapters.iter().map(|c| c.word_count).sum();
                        theme::pill(ui, &format!("{} 字", total), color::TEXT, color::SURFACE_HI);
                        if project.auto_generate.enabled {
                            theme::pill(ui, "定时写作 ON", Color32::WHITE, color::SUCCESS);
                        }
                    });
                } else {
                    ui.label(RichText::new("尚未打开项目").color(color::TEXT_DIM).size(12.5));
                    ui.label(
                        RichText::new("在「项目」中选择小说目录")
                            .color(color::TEXT_FAINT)
                            .size(11.0),
                    );
                }
            });
    }

    fn nav_item(ui: &mut egui::Ui, current: &mut Section, target: Section, label: &str) {
        let selected = *current == target;
        let bg = if selected { color::SURFACE_HI } else { Color32::TRANSPARENT };
        let fg = if selected { color::TEXT } else { color::TEXT_DIM };

        let resp = egui::Frame::default()
            .fill(bg)
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (rect, _) =
                        ui.allocate_exact_size(egui::vec2(3.0, 16.0), egui::Sense::hover());
                    if selected {
                        ui.painter().rect_filled(rect, CornerRadius::same(2), color::ACCENT);
                    }
                    ui.add_space(2.0);
                    ui.label(RichText::new(label).color(fg).size(13.5));
                });
            })
            .response
            .interact(egui::Sense::click());

        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            *current = target;
        }
    }

    // ---------- 项目页 ----------
    fn ui_project(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_salt("project_scroll")
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                self.ui_project_inner(ui);
            });
    }

    fn ui_project_inner(&mut self, ui: &mut egui::Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("当前项目").size(13.5).strong());
                    if let Some(ref p) = self.novel_path {
                        ui.label(
                            RichText::new(p.display().to_string()).color(color::TEXT_DIM).size(12.5),
                        );
                    } else {
                        dim_label(ui, "尚未选择小说目录");
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("📁  选择目录…").clicked() {
                        if let Some(dir) = rfd::FileDialog::new().pick_folder() {
                            if let Err(e) = self.load_novel_dir(dir) {
                                self.status_message = format!("加载失败：{e}");
                            }
                        }
                    }
                });
            });
        });

        let overview = self.project.as_ref().map(|p| {
            let total: i32 = p.chapters.iter().map(|c| c.word_count).sum();
            let done = p.chapters.iter().filter(|c| c.status == "completed").count();
            (p.chapters.len(), total, p.target_chapters, p.chapter_word_goal, done)
        });
        if let Some((n_ch, total, tgt, gw, done)) = overview {
            ui.add_space(12.0);
            theme::card_frame().show(ui, |ui| {
                ui.label(RichText::new("项目概览").size(13.5).strong());
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    Self::stat_box(ui, "章节数", &format!("{n_ch}"));
                    Self::stat_box(ui, "总字数", &format!("{total}"));
                    Self::stat_box(ui, "目标章节", &format!("{tgt}"));
                    Self::stat_box(ui, "目标字/章", &format!("{gw}"));
                    Self::stat_box(ui, "已完成", &format!("{done}"));
                });
            });
        }

        if self.project.is_some() {
            ui.add_space(12.0);
            self.ui_auto_gen_card(ui);

            ui.add_space(12.0);
            self.ui_recent_chapters_card(ui);
        }

        ui.add_space(12.0);
        theme::card_frame().show(ui, |ui| {
            ui.label(RichText::new("最近打开").size(13.5).strong());
            ui.add_space(6.0);
            if self.settings.recent_novels.is_empty() {
                dim_label(ui, "还没有最近打开的项目。");
                return;
            }
            for path in self.settings.recent_novels.clone() {
                let resp = egui::Frame::default()
                    .fill(color::SURFACE_HI)
                    .stroke(Stroke::new(1.0, color::BORDER))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("📁").size(13.0));
                            ui.label(RichText::new(path.clone()).color(color::TEXT).size(12.5));
                        });
                    })
                    .response
                    .interact(egui::Sense::click());
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if resp.clicked() {
                    if let Some(pb) = crate::config::normalize_path(&path) {
                        if let Err(e) = self.load_novel_dir(pb) {
                            self.status_message = format!("加载失败：{e}");
                        }
                    }
                }
                ui.add_space(4.0);
            }
        });
    }

    fn ui_recent_chapters_card(&mut self, ui: &mut egui::Ui) {
        let recent: Vec<(i32, String, String, i32)> = self
            .project
            .as_ref()
            .map(|p| {
                let mut v: Vec<_> = p.chapters.iter().collect();
                v.sort_by(|a, b| b.number.cmp(&a.number));
                v.into_iter()
                    .take(6)
                    .map(|c| (c.number, c.title.clone(), c.status.clone(), c.word_count))
                    .collect()
            })
            .unwrap_or_default();

        let mut click: Option<i32> = None;
        theme::card_frame().show(ui, |ui| {
            ui.label(RichText::new("最近章节").size(13.5).strong());
            ui.add_space(6.0);
            if recent.is_empty() {
                dim_label(ui, "暂无章节，去「写作」页新建第一章。");
                return;
            }
            for (num, title, status, words) in &recent {
                let resp = egui::Frame::default()
                    .fill(color::SURFACE_HI)
                    .stroke(Stroke::new(1.0, color::BORDER))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(12, 8))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("第 {} 章", num))
                                    .color(color::TEXT)
                                    .strong(),
                            );
                            ui.label(
                                RichText::new(format!("· {}", title)).color(color::TEXT_DIM),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    theme::pill(
                                        ui,
                                        status_label(status),
                                        Color32::WHITE,
                                        status_color(status),
                                    );
                                    theme::pill(
                                        ui,
                                        &format!("{} 字", words),
                                        color::TEXT,
                                        color::SURFACE,
                                    );
                                },
                            );
                        });
                    })
                    .response
                    .interact(egui::Sense::click());
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if resp.clicked() {
                    click = Some(*num);
                }
                ui.add_space(4.0);
            }
        });
        if let Some(n) = click {
            self.select_chapter(n);
            self.section = Section::Writing;
        }
    }

    fn ui_auto_gen_card(&mut self, ui: &mut egui::Ui) {
        let writing_vendor = self.settings.writing_vendor.clone();
        let configured = self.configured_vendor_ids();
        let mut start_now = false;
        let mut save_meta = false;
        let mut clear_log = false;

        let (enabled_now, interval_now, last_run, busy) = {
            let p = self.project.as_ref().unwrap();
            (
                p.auto_generate.enabled,
                p.auto_generate.interval_minutes,
                p.auto_generate.last_run_at.clone(),
                self.auto_gen_task.is_some(),
            )
        };

        let mut new_enabled = enabled_now;
        let mut new_interval = interval_now;

        theme::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("⏱  定时写作").size(13.5).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if busy {
                        let label = match self.auto_gen_phase {
                            AutoGenPhase::AuditingPrev { prev_n, .. } => {
                                format!("审计第 {prev_n} 章…")
                            }
                            AutoGenPhase::Writing { next_n } => format!("写作第 {next_n} 章…"),
                            AutoGenPhase::Idle => "请求中…".to_string(),
                        };
                        theme::pill(ui, &label, Color32::WHITE, color::ACCENT);
                    } else if enabled_now {
                        theme::pill(ui, "已启用", Color32::WHITE, color::SUCCESS);
                    } else {
                        theme::pill(ui, "未启用", color::TEXT, color::SURFACE_HI);
                    }
                });
            });
            ui.add_space(6.0);
            dim_label(ui, "由「写作 LLM」按设定间隔生成下一章；可随时手动触发。");
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                if ui.checkbox(&mut new_enabled, "启用定时写作").changed() {
                    save_meta = true;
                }
                ui.add_space(20.0);
                ui.label(RichText::new("间隔（分钟）").size(11.5).color(color::TEXT_DIM));
                if ui
                    .add(
                        egui::DragValue::new(&mut new_interval)
                            .speed(1.0)
                            .range(1..=24 * 60),
                    )
                    .changed()
                {
                    save_meta = true;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !busy
                        && ui
                            .add_enabled(
                                !writing_vendor.is_empty()
                                    && configured.contains(&writing_vendor),
                                egui::Button::new("⚡  立即生成下一章"),
                            )
                            .clicked()
                    {
                        start_now = true;
                    }
                    if !self.auto_gen_log.is_empty() && ui.button("清空日志").clicked() {
                        clear_log = true;
                    }
                });
            });

            ui.add_space(4.0);
            if ui
                .checkbox(
                    &mut self.settings.auto_gen_audit_first,
                    "先审计上一章 → 再写下一章（链式工作流，使用「审计 LLM」）",
                )
                .changed()
            {
                let _ = self.paths.save_settings(&self.settings);
            }

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("写作 LLM：")).color(color::TEXT_DIM).size(11.5));
                if writing_vendor.is_empty() {
                    ui.label(
                        RichText::new("未选择（请到「设置」配置）")
                            .color(color::WARNING)
                            .size(11.5),
                    );
                } else {
                    ui.label(
                        RichText::new(Self::vendor_label(&writing_vendor))
                            .color(color::TEXT)
                            .size(11.5),
                    );
                }
                if self.settings.auto_gen_audit_first {
                    ui.add_space(12.0);
                    ui.label(RichText::new("审计 LLM：").color(color::TEXT_DIM).size(11.5));
                    if self.settings.review_vendor.is_empty() {
                        ui.label(
                            RichText::new("未选择")
                                .color(color::WARNING)
                                .size(11.5),
                        );
                    } else {
                        ui.label(
                            RichText::new(Self::vendor_label(&self.settings.review_vendor))
                                .color(color::TEXT)
                                .size(11.5),
                        );
                    }
                }
                ui.add_space(20.0);
                ui.label(
                    RichText::new(format!(
                        "上次：{}",
                        if last_run.is_empty() { "—" } else { last_run.as_str() }
                    ))
                    .color(color::TEXT_DIM)
                    .size(11.5),
                );
            });

            if let Some(task) = &self.auto_gen_task {
                ui.add_space(6.0);
                ui.label(
                    RichText::new(format!("⏳ {}", task.stats_label()))
                        .color(color::ACCENT_HI)
                        .size(11.5),
                );
            }

            if !self.auto_gen_log.is_empty() {
                ui.add_space(8.0);
                ui.label(RichText::new("最近日志").size(11.5).color(color::TEXT_DIM));
                ui.add_space(2.0);
                for entry in self.auto_gen_log.iter().rev().take(6) {
                    ui.label(RichText::new(format!("· {entry}")).color(color::TEXT).size(11.5));
                }
            }
        });

        if clear_log {
            self.auto_gen_log.clear();
        }
        if save_meta {
            if let Some(p) = self.project.as_mut() {
                p.auto_generate.enabled = new_enabled;
                p.auto_generate.interval_minutes = new_interval.max(1);
            }
            self.save_novel_meta();
            self.auto_gen_last_tick = Instant::now();
        }
        if start_now {
            self.start_auto_gen();
        }
    }

    fn stat_box(ui: &mut egui::Ui, label: &str, value: &str) {
        egui::Frame::default()
            .fill(color::SURFACE_HI)
            .stroke(Stroke::new(1.0, color::BORDER))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(label).color(color::TEXT_DIM).size(11.0));
                    ui.label(RichText::new(value).color(color::TEXT).size(18.0).strong());
                });
            });
    }

    // ---------- 写作页 ----------
    fn ui_writing(&mut self, ui: &mut egui::Ui, time: f64) {
        if self.store.is_none() {
            theme::empty_state(ui, "尚未打开项目", "请先在「项目」中选择小说目录");
            return;
        }
        self.maybe_refresh_pending_hooks();

        // 伏笔提醒卡片
        if !self.pending_hooks_summary.is_empty() {
            self.ui_foreshadow_card(ui);
            ui.add_space(8.0);
        }

        // 工具栏
        theme::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("章节").size(11.5).color(color::TEXT_DIM));
                let nums: Vec<i32> = self
                    .project
                    .as_ref()
                    .map(|p| {
                        let mut v: Vec<_> = p.chapters.iter().map(|c| c.number).collect();
                        v.sort();
                        v
                    })
                    .unwrap_or_default();

                let cur = self.selected_chapter;
                ComboBox::from_id_salt("chap_pick")
                    .width(220.0)
                    .selected_text(
                        cur.map(|n| format!("第 {n} 章"))
                            .unwrap_or_else(|| "（未选择）".into()),
                    )
                    .show_ui(ui, |ui| {
                        for n in nums {
                            let label = if let Some(ref p) = self.project {
                                if let Some(ch) = ProjectStore::get_chapter(p, n) {
                                    format!(
                                        "第 {n} 章 · {} 字 · {}",
                                        ch.word_count,
                                        status_label(&ch.status)
                                    )
                                } else {
                                    format!("第 {n} 章")
                                }
                            } else {
                                format!("第 {n} 章")
                            };
                            if ui.selectable_label(cur == Some(n), label).clicked() {
                                self.select_chapter(n);
                            }
                        }
                    });

                ui.add_space(12.0);
                ui.label(RichText::new("状态").size(11.5).color(color::TEXT_DIM));
                let cur_status = self.chapter_status.clone();
                ComboBox::from_id_salt("ch_status_pick")
                    .width(120.0)
                    .selected_text(status_label(&cur_status))
                    .show_ui(ui, |ui| {
                        for (k, l) in CHAPTER_STATUSES {
                            if ui.selectable_label(cur_status == *k, *l).clicked() {
                                self.chapter_status = (*k).into();
                                self.chapter_dirty = true;
                            }
                        }
                    });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("💾  保存").clicked() {
                        self.save_current_chapter();
                    }
                    // 🔁 保存并同步长期记忆：一键保存当前章节 + 立即触发「AI 刷新全部」
                    // （current_state / pending_hooks / subplot_board / emotional_arcs /
                    // character_matrix / particle_ledger / novel_brief + 本地重建 chapter_summaries）。
                    // 忽略 auto_refresh_state_after_chapter_save 开关，始终尝试刷新。
                    let sync_enabled = self.selected_chapter.is_some()
                        && self.manual_gen_task.is_none()
                        && self.state_refresh_task.is_none()
                        && self.state_refresh_batch.is_none();
                    let sync_btn = ui
                        .add_enabled(sync_enabled, egui::Button::new("🔁  保存并同步记忆"))
                        .on_hover_text(
                            "保存当前章节 → 立即触发「AI 刷新全部长期记忆档案」：\n\
                             · 避免「生成下一章」的[连续性档案]滞后于实际章节推进\n\
                             · 会消耗 7 次写作 LLM 调用（book_rules.md 不动，chapter_summaries 本地重建）\n\
                             · 章节正文仍以本次保存为准，不会自动修改",
                        );
                    if sync_btn.clicked() {
                        self.save_and_sync_memory();
                    }
                    if ui.button("➕  新建下一章").clicked() {
                        self.new_chapter();
                    }

                    let running = self.manual_gen_task.is_some();
                    if running {
                        if ui
                            .button("⏹  取消生成")
                            .on_hover_text("取消当前 AI 生成，已返回的流式内容保留在编辑器")
                            .clicked()
                        {
                            self.cancel_manual_chapter_generation();
                        }
                        let running_n = self.manual_gen_target.unwrap_or(0);
                        theme::pill(
                            ui,
                            &format!("生成中 · 第 {running_n} 章"),
                            Color32::WHITE,
                            color::ACCENT,
                        );
                    } else {
                        // ➡ 生成下一章（对齐 inkoswin：自动保存 → 找/建下一空章 → 触发生成）
                        let next_btn = ui
                            .button("➡️  生成下一章")
                            .on_hover_text(
                                "对齐 inkoswin「生成下一章节」：自动保存当前 → 定位首个正文为空的章节（或新建下一章）→ 基于状态档案 + 前文摘要 + 最近节选触发 AI 生成。",
                            );
                        if next_btn.clicked() {
                            self.start_generate_next_chapter();
                        }

                        // 🪄 生成本章（对齐 inkoswin：若已有正文则弹覆盖确认）
                        if let Some(n) = self.selected_chapter {
                            let btn = ui
                                .button("🪄  生成本章")
                                .on_hover_text(
                                    "对齐 inkoswin「生成当前章节」：当前编辑器里的正文会作为「现有草稿」交给 LLM，可续写或重新生成；生成完后先填入编辑器等你保存。",
                                );
                            if btn.clicked() {
                                let has_body = !self.chapter_body.trim().is_empty();
                                if has_body {
                                    self.pending_gen_confirm = Some(n);
                                } else {
                                    self.start_manual_chapter_generation(n);
                                }
                            }
                            if ui.button("📜  章节历史").clicked() {
                                self.open_chapter_history(n);
                            }
                        } else {
                            let btn = ui
                                .button("🪄  生成本章")
                                .on_hover_text(
                                    "未选中章节时会先新建第 1 章再触发生成。",
                                );
                            if btn.clicked() {
                                self.new_chapter();
                                if let Some(n) = self.selected_chapter {
                                    self.start_manual_chapter_generation(n);
                                }
                            }
                        }
                    }
                    if self.chapter_dirty || self.chapter_summary_dirty {
                        theme::pill(ui, "未保存", Color32::WHITE, color::WARNING);
                    }
                });
            });

            // 字数治理 pill：目标 ± 容差 + 单次纠偏 prompt
            let goal = self
                .project
                .as_ref()
                .map(|p| p.chapter_word_goal.max(0))
                .unwrap_or(0);
            if goal > 0 {
                let tol = self.settings.effective_word_tolerance();
                let report = crate::words::evaluate(
                    &self.chapter_body,
                    crate::words::WordBudget { goal, tolerance: tol },
                );
                let (color, hint) = match report.status {
                    crate::words::WordStatus::OnTarget => (color::SUCCESS, "达标"),
                    crate::words::WordStatus::Under => (color::WARNING, "偏短"),
                    crate::words::WordStatus::Over => (color::WARNING, "偏长"),
                };
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    theme::pill(
                        ui,
                        &format!(
                            "{}/{}（±{}） · {}",
                            report.actual, report.goal, report.tolerance, hint
                        ),
                        Color32::WHITE,
                        color,
                    );
                    if ui
                        .small_button("📐  归一化 prompt")
                        .on_hover_text("生成单次字数纠偏 prompt 并复制；可粘到写作助手或外部 LLM")
                        .clicked()
                    {
                        let body = self.chapter_body.clone();
                        let prompt = crate::words::normalize_prompt(&body, &report);
                        ui.ctx().copy_text(prompt.clone());
                        self.assistant_input = prompt;
                        self.status_message = "归一化 prompt 已复制并送入写作助手输入框".into();
                    }
                });
            }
        });

        ui.add_space(8.0);
        // 标题行
        theme::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("标题").size(11.5).color(color::TEXT_DIM));
                let tr = ui.add(
                    TextEdit::singleline(&mut self.chapter_title)
                        .desired_width(f32::INFINITY)
                        .id_salt("ch_title"),
                );
                if tr.changed() {
                    self.chapter_dirty = true;
                    self.preview_deadline = Some(time + 0.28);
                }
            });
        });

        ui.add_space(8.0);
        // 摘要卡片（标题之下，独立一行）
        theme::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("章节摘要").size(11.5).color(color::TEXT_DIM));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.chapter_summary_dirty {
                        theme::pill(ui, "未保存", Color32::WHITE, color::WARNING);
                    }
                });
            });
            ui.add_space(4.0);
            let r = ui.add(
                TextEdit::multiline(&mut self.chapter_summary)
                    .desired_width(f32::INFINITY)
                    .desired_rows(3)
                    .hint_text("此章关键事件、人物动向、伏笔变化…保存后会写入 project.json")
                    .id_salt("ch_summary"),
            );
            if r.changed() {
                self.chapter_summary_dirty = true;
            }
        });

        ui.add_space(8.0);
        let avail_h = ui.available_height();
        ui.allocate_ui(egui::vec2(ui.available_width(), avail_h), |ui| {
            ui.columns(2, |cols| {
                Self::writing_pane_editor(
                    &mut cols[0],
                    &mut self.chapter_body,
                    &mut self.chapter_dirty,
                    &mut self.preview_deadline,
                    time,
                );
                Self::writing_pane_preview(&mut cols[1], &self.preview_md, &mut self.cm_cache);
            });
        });
    }

    fn ui_foreshadow_card(&mut self, ui: &mut egui::Ui) {
        let count = self.pending_hooks_summary.len();
        let mut goto_state = false;
        egui::Frame::default()
            .fill(Color32::from_rgb(0x2a, 0x24, 0x18))
            .stroke(Stroke::new(1.0, Color32::from_rgb(0x6a, 0x4d, 0x1f)))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("📌  伏笔提醒（{count}）"))
                            .color(Color32::from_rgb(0xf0, 0xc0, 0x70))
                            .size(13.0)
                            .strong(),
                    );
                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if ui.button("打开 pending_hooks.md").clicked() {
                                goto_state = true;
                            }
                            let label = if self.pending_hooks_collapsed { "展开" } else { "收起" };
                            if ui.small_button(label).clicked() {
                                self.pending_hooks_collapsed = !self.pending_hooks_collapsed;
                            }
                        },
                    );
                });
                if !self.pending_hooks_collapsed {
                    ui.add_space(4.0);
                    for item in self.pending_hooks_summary.iter().take(8) {
                        ui.label(
                            RichText::new(format!("· {item}"))
                                .color(color::TEXT)
                                .size(11.5),
                        );
                    }
                    if self.pending_hooks_summary.len() > 8 {
                        ui.label(
                            RichText::new(format!(
                                "…还有 {} 条，到「小说设定 → 状态档案」查看",
                                self.pending_hooks_summary.len() - 8
                            ))
                            .color(color::TEXT_FAINT)
                            .size(11.0),
                        );
                    }
                }
            });
        if goto_state {
            self.section = Section::NovelMeta;
            self.nm_tab = NovelMetaTab::StateDocs;
            self.selected_state_file = "pending_hooks.md".into();
            self.load_state_doc();
        }
    }

    fn open_chapter_history(&mut self, n: i32) {
        self.history_open_for = Some(n);
        if let Some(ref root) = self.novel_path {
            self.history_revisions = history::list_chapter_revisions(root, n);
        } else {
            self.history_revisions.clear();
        }
        self.history_selected_file = self.history_revisions.first().map(|r| r.backup_file.clone());
        self.history_selected_body.clear();
        self.load_selected_history_body();
    }

    fn load_selected_history_body(&mut self) {
        let Some(n) = self.history_open_for else { return };
        let Some(ref file) = self.history_selected_file else { return };
        let Some(ref root) = self.novel_path else { return };
        match history::read_revision(root, n, file) {
            Ok(raw) => self.history_selected_body = history::revision_body_only(&raw),
            Err(e) => self.history_selected_body = format!("读取失败：{e}"),
        }
    }

    /// 覆盖生成确认弹窗（对齐 inkoswin `generate_current_chapter` 中的 `askyesno("覆盖生成", …)`）。
    fn show_gen_confirm_modal(&mut self, ctx: &egui::Context) {
        let Some(n) = self.pending_gen_confirm else { return };
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.pending_gen_confirm = None;
            return;
        }
        let mut decision: Option<bool> = None;
        let mut open = true;
        egui::Window::new("覆盖生成")
            .id(egui::Id::new("gen_confirm_window"))
            .open(&mut open)
            .resizable(false)
            .movable(true)
            .collapsible(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                ui.set_min_width(360.0);
                ui.label(
                    RichText::new(format!("当前第 {n} 章已有正文"))
                        .size(13.5)
                        .strong(),
                );
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "继续生成会把现有正文作为「当前章节现有草稿」交给 AI，\
                         生成结果会覆盖编辑器中的正文（生成完你还需要手动 💾 保存才会落盘）。\n\n\
                         是否继续？",
                    )
                    .color(color::TEXT_DIM)
                    .size(11.5),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("取消").clicked() {
                        decision = Some(false);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(RichText::new("继续生成").strong()).clicked() {
                            decision = Some(true);
                        }
                    });
                });
            });
        if !open {
            self.pending_gen_confirm = None;
            return;
        }
        match decision {
            Some(true) => {
                self.pending_gen_confirm = None;
                self.start_manual_chapter_generation(n);
            }
            Some(false) => {
                self.pending_gen_confirm = None;
            }
            None => {}
        }
    }

    /// `book_rules.md` 覆盖确认弹窗（只在编辑器里已经有内容时弹出）。
    fn show_book_rules_confirm_modal(&mut self, ctx: &egui::Context) {
        if !self.pending_book_rules_confirm {
            return;
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.pending_book_rules_confirm = false;
            return;
        }
        let mut decision: Option<bool> = None;
        let mut open = true;
        egui::Window::new("覆盖 book_rules.md")
            .id(egui::Id::new("book_rules_confirm_window"))
            .open(&mut open)
            .resizable(false)
            .movable(true)
            .collapsible(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                ui.set_min_width(380.0);
                ui.label(
                    RichText::new("当前 book_rules.md 已有内容")
                        .size(13.5)
                        .strong(),
                );
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "AI 会基于当前小说设定重新生成完整的 YAML frontmatter + 叙事指导，\
                         结果会覆盖编辑器中的内容（生成完仍需要手动 💾 保存才会落盘）。\n\n\
                         若现有 frontmatter 设置了 `fanficMode`，会被自动沿用。\n\n\
                         是否继续？",
                    )
                    .color(color::TEXT_DIM)
                    .size(11.5),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button("取消").clicked() {
                        decision = Some(false);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(RichText::new("继续生成").strong()).clicked() {
                            decision = Some(true);
                        }
                    });
                });
            });
        if !open {
            self.pending_book_rules_confirm = false;
            return;
        }
        match decision {
            Some(true) => {
                self.pending_book_rules_confirm = false;
                self.start_book_rules_generation();
            }
            Some(false) => {
                self.pending_book_rules_confirm = false;
            }
            None => {}
        }
    }

    fn show_chapter_history_modal(&mut self, ctx: &egui::Context) {
        let Some(n) = self.history_open_for else { return };
        // ESC 一键关闭
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.history_open_for = None;
            self.history_revisions.clear();
            self.history_selected_file = None;
            self.history_selected_body.clear();
            return;
        }

        let mut open = true;
        let mut want_close = false;
        let mut want_restore = false;
        let mut click_file: Option<String> = None;

        let cur_body = self
            .store
            .as_ref()
            .and_then(|s| s.load_chapter_content(n).ok())
            .map(|(_, b)| b)
            .unwrap_or_default();

        let screen = ctx.content_rect();
        let win_w = (screen.width() - 80.0).clamp(640.0, 1100.0);
        let win_h = (screen.height() - 120.0).clamp(420.0, 720.0);

        egui::Window::new(format!("📜 第 {n} 章 · 版本历史"))
            .id(egui::Id::new("chapter_history_window"))
            .open(&mut open)
            .resizable(true)
            .movable(true)
            .collapsible(false)
            .default_size(egui::vec2(win_w, win_h))
            .max_height(screen.height() - 60.0)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(screen.center())
            .show(ctx, |ui| {
                // 顶部固定操作栏：始终可见的「关闭」按钮，避免内容撑出窗口时找不到关闭入口
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("第 {n} 章 · 共 {} 个版本", self.history_revisions.len()))
                            .color(color::TEXT_DIM)
                            .size(11.5),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(RichText::new("✖  关闭").color(color::TEXT))
                            .on_hover_text("ESC 也可关闭")
                            .clicked()
                        {
                            want_close = true;
                        }
                    });
                });
                ui.separator();
                ui.add_space(6.0);

                if self.history_revisions.is_empty() {
                    theme::empty_state(
                        ui,
                        "暂无历史版本",
                        "保存或 AI 替换原文时会自动备份原版到「章节历史」",
                    );
                } else {
                    ui.horizontal_top(|ui| {
                        // 左侧：版本列表
                        ui.vertical(|ui| {
                            ui.set_width(260.0);
                            ui.label(
                                RichText::new(format!("共 {} 个版本", self.history_revisions.len()))
                                    .color(color::TEXT_DIM)
                                    .size(11.5),
                            );
                            ui.add_space(4.0);
                            egui::ScrollArea::vertical()
                                .id_salt("hist_list")
                                .max_height(420.0)
                                .show(ui, |ui| {
                                    for rev in &self.history_revisions {
                                        let selected = self.history_selected_file.as_deref()
                                            == Some(rev.backup_file.as_str());
                                        let bg = if selected {
                                            color::SURFACE_HI
                                        } else {
                                            color::SURFACE
                                        };
                                        let resp = egui::Frame::default()
                                            .fill(bg)
                                            .stroke(Stroke::new(1.0, color::BORDER))
                                            .corner_radius(CornerRadius::same(6))
                                            .inner_margin(Margin::symmetric(8, 6))
                                            .show(ui, |ui| {
                                                ui.label(
                                                    RichText::new(&rev.timestamp)
                                                        .color(color::TEXT)
                                                        .size(12.0)
                                                        .strong(),
                                                );
                                                ui.label(
                                                    RichText::new(format!(
                                                        "{} · {} → {} 字",
                                                        source_label(&rev.source),
                                                        rev.old_chars,
                                                        rev.new_chars
                                                    ))
                                                    .color(color::TEXT_DIM)
                                                    .size(11.0),
                                                );
                                                if !rev.note.is_empty() {
                                                    ui.label(
                                                        RichText::new(&rev.note)
                                                            .color(color::TEXT_FAINT)
                                                            .size(10.5),
                                                    );
                                                }
                                            })
                                            .response
                                            .interact(egui::Sense::click());
                                        if resp.hovered() {
                                            ui.ctx()
                                                .set_cursor_icon(egui::CursorIcon::PointingHand);
                                        }
                                        if resp.clicked() {
                                            click_file = Some(rev.backup_file.clone());
                                        }
                                        ui.add_space(4.0);
                                    }
                                });
                        });

                        ui.separator();

                        // 右侧：对照差异
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(if let Some(ref f) = self.history_selected_file {
                                        format!("当前选中：{f}")
                                    } else {
                                        "（未选中版本）".into()
                                    })
                                    .color(color::TEXT_DIM)
                                    .size(11.5),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if !self.history_selected_body.is_empty()
                                            && ui
                                                .button(
                                                    RichText::new("⤴  恢复此版本")
                                                        .color(color::WARNING),
                                                )
                                                .clicked()
                                        {
                                            want_restore = true;
                                        }
                                    },
                                );
                            });
                            ui.add_space(4.0);
                            let avail = ui.available_size();
                            ui.allocate_ui(avail, |ui| {
                                ui.columns(2, |cols| {
                                    cols[0].vertical(|ui| {
                                        ui.label(
                                            RichText::new("此版本（备份）")
                                                .size(11.5)
                                                .color(color::TEXT_DIM),
                                        );
                                        ui.add_space(2.0);
                                        egui::Frame::default()
                                            .fill(color::SURFACE)
                                            .stroke(Stroke::new(1.0, color::BORDER))
                                            .corner_radius(CornerRadius::same(8))
                                            .inner_margin(Margin::same(8))
                                            .show(ui, |ui| {
                                                egui::ScrollArea::vertical()
                                                    .id_salt("hist_old")
                                                    .max_height(360.0)
                                                    .show(ui, |ui| {
                                                        ui.add(
                                                            TextEdit::multiline(
                                                                &mut self
                                                                    .history_selected_body
                                                                    .clone(),
                                                            )
                                                            .desired_width(f32::INFINITY)
                                                            .desired_rows(20)
                                                            .interactive(false)
                                                            .id_salt("hist_old_text"),
                                                        );
                                                    });
                                            });
                                    });
                                    cols[1].vertical(|ui| {
                                        ui.label(
                                            RichText::new("当前正文（磁盘）")
                                                .size(11.5)
                                                .color(color::TEXT_DIM),
                                        );
                                        ui.add_space(2.0);
                                        egui::Frame::default()
                                            .fill(color::SURFACE)
                                            .stroke(Stroke::new(1.0, color::BORDER))
                                            .corner_radius(CornerRadius::same(8))
                                            .inner_margin(Margin::same(8))
                                            .show(ui, |ui| {
                                                egui::ScrollArea::vertical()
                                                    .id_salt("hist_cur")
                                                    .max_height(360.0)
                                                    .show(ui, |ui| {
                                                        ui.add(
                                                            TextEdit::multiline(
                                                                &mut cur_body.clone(),
                                                            )
                                                            .desired_width(f32::INFINITY)
                                                            .desired_rows(20)
                                                            .interactive(false)
                                                            .id_salt("hist_cur_text"),
                                                        );
                                                    });
                                            });
                                    });
                                });
                            });
                        });
                    });
                }
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("关闭").clicked() {
                            want_close = true;
                        }
                    });
                });
            });

        if let Some(f) = click_file {
            self.history_selected_file = Some(f);
            self.load_selected_history_body();
        }
        if want_restore {
            self.restore_history_to_chapter();
        }
        if want_close || !open {
            self.history_open_for = None;
            self.history_revisions.clear();
            self.history_selected_file = None;
            self.history_selected_body.clear();
        }
    }

    fn restore_history_to_chapter(&mut self) {
        let Some(n) = self.history_open_for else { return };
        if self.history_selected_body.is_empty() {
            self.status_message = "未选中版本".into();
            return;
        }
        let new_body = self.history_selected_body.clone();
        let novel_root = self.novel_path.clone();
        let (Some(store), Some(project)) = (&self.store, &mut self.project) else { return };
        let cur_title = project
            .chapters
            .iter()
            .find(|c| c.number == n)
            .map(|c| c.title.clone())
            .unwrap_or_default();
        let old_body = store.load_chapter_content(n).map(|(_, b)| b).unwrap_or_default();

        if let Some(ref root) = novel_root {
            let _ = history::snapshot_chapter(
                root,
                n,
                &cur_title,
                &old_body,
                &new_body,
                "restore",
                "恢复历史版本前自动备份当前内容",
            );
        }
        match store.save_chapter(project, n, &cur_title, &new_body, "review", "") {
            Ok(()) => {
                self.status_message = format!("已恢复第 {n} 章历史版本");
                oplog::try_append(
                    novel_root.as_deref(),
                    "恢复历史版本",
                    &format!(
                        "第 {n} 章 · {}",
                        self.history_selected_file.as_deref().unwrap_or("?")
                    ),
                );
                if self.selected_chapter == Some(n) {
                    self.select_chapter(n);
                }
                if let Some(ref root) = novel_root {
                    self.history_revisions = history::list_chapter_revisions(root, n);
                }
            }
            Err(e) => self.status_message = format!("恢复失败：{e}"),
        }
    }

    fn ui_oplog(&mut self, ui: &mut egui::Ui) {
        if self.novel_path.is_none() {
            theme::empty_state(ui, "尚未打开项目", "请先在「项目」中选择小说目录");
            return;
        }
        self.refresh_oplog();

        theme::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("日期").size(11.5).color(color::TEXT_DIM));
                let cur = self.oplog_selected_date;
                let label = cur
                    .map(|d| d.format("%Y-%m-%d").to_string())
                    .unwrap_or_else(|| "（无记录）".into());
                ComboBox::from_id_salt("oplog_date")
                    .width(160.0)
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        for d in &self.oplog_dates {
                            if ui
                                .selectable_label(cur == Some(*d), d.format("%Y-%m-%d").to_string())
                                .clicked()
                            {
                                self.oplog_selected_date = Some(*d);
                                self.oplog_loaded_for = None;
                            }
                        }
                    });
                ui.add_space(8.0);
                ui.label(
                    RichText::new(format!("条目：{}", self.oplog_entries.len()))
                        .color(color::TEXT_DIM)
                        .size(11.5),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⟳  刷新").clicked() {
                        self.oplog_loaded_for = None;
                    }
                    if ui.button("📁  打开日志目录").clicked() {
                        if let Some(ref root) = self.novel_path {
                            let dir = oplog::oplog_root(root);
                            let _ = std::fs::create_dir_all(&dir);
                            let _ = open_in_explorer(&dir);
                        }
                    }
                });
            });
        });

        ui.add_space(8.0);
        theme::card_frame().show(ui, |ui| {
            if self.oplog_entries.is_empty() {
                theme::empty_state(
                    ui,
                    "今日暂无操作记录",
                    "「打开项目 / 保存章节 / AI 审计 / 替换原文 / 定时写作 …」都会自动记录",
                );
                return;
            }
            egui::ScrollArea::vertical()
                .id_salt("oplog_scroll")
                .auto_shrink([false; 2])
                .show(ui, |ui| {
                    egui::Grid::new("oplog_grid")
                        .num_columns(3)
                        .striped(true)
                        .spacing([12.0, 6.0])
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new("时间").color(color::TEXT_DIM).size(11.5).strong(),
                            );
                            ui.label(
                                RichText::new("类型").color(color::TEXT_DIM).size(11.5).strong(),
                            );
                            ui.label(
                                RichText::new("详情").color(color::TEXT_DIM).size(11.5).strong(),
                            );
                            ui.end_row();
                            for entry in self.oplog_entries.iter().rev() {
                                ui.label(
                                    RichText::new(&entry.time)
                                        .color(color::TEXT)
                                        .monospace()
                                        .size(12.0),
                                );
                                ui.label(
                                    RichText::new(&entry.kind)
                                        .color(op_kind_color(&entry.kind))
                                        .size(12.0),
                                );
                                ui.label(
                                    RichText::new(&entry.detail).color(color::TEXT).size(12.0),
                                );
                                ui.end_row();
                            }
                        });
                });
        });
    }

    fn writing_pane_editor(
        ui: &mut egui::Ui,
        body: &mut String,
        dirty: &mut bool,
        deadline: &mut Option<f64>,
        time: f64,
    ) {
        ui.label(RichText::new("正文（Markdown）").size(11.5).color(color::TEXT_DIM));
        ui.add_space(4.0);
        egui::Frame::default()
            .fill(color::SURFACE)
            .stroke(Stroke::new(1.0, color::BORDER))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("editor_scroll")
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        let r = ui.add(
                            TextEdit::multiline(body)
                                .desired_width(f32::INFINITY)
                                .desired_rows(40)
                                .id_salt("ch_body"),
                        );
                        if r.changed() {
                            *dirty = true;
                            *deadline = Some(time + 0.28);
                        }
                    });
            });
    }

    fn writing_pane_preview(ui: &mut egui::Ui, preview: &str, cache: &mut CommonMarkCache) {
        ui.label(RichText::new("预览").size(11.5).color(color::TEXT_DIM));
        ui.add_space(4.0);
        egui::Frame::default()
            .fill(color::SURFACE)
            .stroke(Stroke::new(1.0, color::BORDER))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::same(8))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("preview_scroll")
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        CommonMarkViewer::new().show(ui, cache, preview);
                    });
            });
    }

    // ---------- 审核页 ----------
    fn ui_review(&mut self, ui: &mut egui::Ui) {
        if self.project.is_none() {
            theme::empty_state(ui, "尚未打开项目", "请先在「项目」中选择小说目录");
            return;
        }

        let configured = self.configured_vendor_ids();
        if self.settings.review_vendor.is_empty() {
            self.settings.review_vendor = configured.first().cloned().unwrap_or_default();
        }

        let chapter_nums: Vec<i32> = self
            .project
            .as_ref()
            .map(|p| {
                let mut v: Vec<_> = p.chapters.iter().map(|c| c.number).collect();
                v.sort();
                v
            })
            .unwrap_or_default();
        if self.review_target.is_none() {
            self.review_target = chapter_nums.last().copied();
        }

        let mut start = false;
        let mut clear_result = false;
        let mut replace_now = false;

        theme::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                // 模式切换
                ui.label(RichText::new("模式").size(11.5).color(color::TEXT_DIM));
                let modes = [
                    (ReviewMode::Audit, "审计"),
                    (ReviewMode::Rewrite, "改写"),
                ];
                for (m, l) in modes {
                    let selected = self.review_mode == m;
                    let bg = if selected { color::ACCENT_DIM } else { color::SURFACE_HI };
                    let fg = if selected { color::TEXT } else { color::TEXT_DIM };
                    let r = egui::Frame::default()
                        .fill(bg)
                        .stroke(Stroke::new(1.0, color::BORDER))
                        .corner_radius(CornerRadius::same(6))
                        .inner_margin(Margin::symmetric(10, 4))
                        .show(ui, |ui| ui.label(RichText::new(l).color(fg).size(12.0)))
                        .response
                        .interact(egui::Sense::click());
                    if r.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if r.clicked() && self.review_mode != m {
                        self.swap_review_buffers(m);
                    }
                }

                ui.add_space(12.0);
                ui.label(RichText::new("AI").size(11.5).color(color::TEXT_DIM));
                let sel = if self.settings.review_vendor.is_empty() {
                    "（未选择）".into()
                } else {
                    Self::vendor_label(&self.settings.review_vendor)
                };
                ComboBox::from_id_salt("review_vendor")
                    .width(200.0)
                    .selected_text(sel)
                    .show_ui(ui, |ui| {
                        if configured.is_empty() {
                            ui.label("（请先在「设置」中配置服务商）");
                        }
                        for id in &configured {
                            if ui
                                .selectable_label(
                                    self.settings.review_vendor == *id,
                                    Self::vendor_label(id),
                                )
                                .clicked()
                            {
                                self.settings.review_vendor = id.clone();
                                let _ = self.paths.save_settings(&self.settings);
                            }
                        }
                    });

                ui.add_space(12.0);
                ui.label(RichText::new("章节").size(11.5).color(color::TEXT_DIM));
                let cur = self.review_target;
                ComboBox::from_id_salt("review_chap")
                    .width(140.0)
                    .selected_text(
                        cur.map(|n| format!("第 {n} 章"))
                            .unwrap_or_else(|| "（未选择）".into()),
                    )
                    .show_ui(ui, |ui| {
                        for n in &chapter_nums {
                            if ui.selectable_label(cur == Some(*n), format!("第 {n} 章")).clicked()
                            {
                                self.review_target = Some(*n);
                            }
                        }
                    });

                ui.add_space(12.0);
                if self.review_mode == ReviewMode::Rewrite {
                    ui.checkbox(&mut self.review_compare, "对照原文");
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let action_label = match self.review_mode {
                        ReviewMode::Audit => "🔍  开始审计",
                        ReviewMode::Rewrite => "✨  AI 改写",
                    };
                    let tooltip = match self.review_mode {
                        ReviewMode::Audit => "结合状态档案对当前章节做审计（流式可在「设置」开关）".to_string(),
                        ReviewMode::Rewrite => {
                            let n = self.review_target.unwrap_or(0);
                            let has_audit = self
                                .audit_records
                                .iter()
                                .any(|r| r.mode == "audit" && r.chapter_no == n);
                            if has_audit {
                                format!("将参考第 {n} 章最新审计意见 + 状态档案进行改写")
                            } else {
                                format!("将参考状态档案进行改写（第 {n} 章暂无可引用审计）")
                            }
                        }
                    };
                    if self.review_task.is_some() {
                        ui.add_enabled(false, egui::Button::new("⏳  请求中…"));
                    } else if ui.button(action_label).on_hover_text(tooltip).clicked() {
                        start = true;
                    }
                    if self.review_mode == ReviewMode::Rewrite
                        && self.review_task.is_none()
                        && !self.review_result.trim().is_empty()
                        && self.review_for_chapter.is_some()
                    {
                        if self.state_sync_task.is_some() {
                            ui.add_enabled(
                                false,
                                egui::Button::new(
                                    RichText::new("⏳  同步状态档案中…").color(color::ACCENT_HI),
                                ),
                            );
                        } else if ui
                            .button(RichText::new("⤴  替换原文").color(color::WARNING))
                            .on_hover_text("用 AI 改写覆盖第章正文，并链式触发状态档案同步")
                            .clicked()
                        {
                            replace_now = true;
                        }
                    }
                    if !self.review_result.is_empty() && ui.button("清空").clicked() {
                        clear_result = true;
                    }
                });
            });
        });

        if clear_result {
            self.review_result.clear();
            self.review_for_chapter = None;
            self.selected_audit_ts = None;
        }
        if start {
            match self.review_mode {
                ReviewMode::Audit => self.start_review_audit(),
                ReviewMode::Rewrite => self.start_review_rewrite(),
            }
        }
        if replace_now {
            self.replace_chapter_with_rewrite();
        }

        ui.add_space(8.0);
        // 历史记录条（按当前模式过滤），不抢主区域
        self.ui_audit_records_strip(ui);
        // 状态档案同步进度/日志卡（仅在同步进行中或最近一次执行后有日志时展示）
        if self.state_sync_task.is_some() || !self.state_sync_log.is_empty() {
            ui.add_space(8.0);
            self.ui_state_sync_card(ui);
        }
        ui.add_space(8.0);
        // 主结果区域，独占剩余空间
        match self.review_mode {
            ReviewMode::Audit => self.ui_review_audit_body(ui),
            ReviewMode::Rewrite => self.ui_review_rewrite_body(ui),
        }
    }

    /// 替换原文后链式状态档案同步的进度/日志卡片。
    fn ui_state_sync_card(&mut self, ui: &mut egui::Ui) {
        let chap = match self.state_sync_phase {
            StateSyncPhase::Running { chapter_no } => Some(chapter_no),
            StateSyncPhase::Idle => None,
        };
        theme::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(n) = chap {
                    ui.label(
                        RichText::new(format!("🗂  状态档案同步中（第 {n} 章）"))
                            .size(13.0)
                            .strong()
                            .color(color::ACCENT_HI),
                    );
                    if let Some(task) = &self.state_sync_task {
                        ui.add_space(8.0);
                        ui.label(
                            RichText::new(task.stats_label())
                                .color(color::ACCENT_HI)
                                .size(11.5)
                                .monospace(),
                        );
                    }
                } else {
                    ui.label(
                        RichText::new("🗂  状态档案同步日志")
                            .size(13.0)
                            .strong(),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.state_sync_task.is_none()
                        && !self.state_sync_log.is_empty()
                        && ui.small_button("清空日志").clicked()
                    {
                        self.state_sync_log.clear();
                    }
                });
            });
            if !self.state_sync_log.is_empty() {
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt("state_sync_log_scroll")
                    .max_height(120.0)
                    .auto_shrink([false, true])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in self.state_sync_log.iter().rev().take(50).collect::<Vec<_>>().into_iter().rev() {
                            ui.label(
                                RichText::new(line)
                                    .size(11.0)
                                    .color(color::TEXT_DIM)
                                    .monospace(),
                            );
                        }
                    });
            }
        });
    }

    /// 审计/改写历史条：单行可横向滚动，按当前模式（audit / rewrite）过滤；点击一键载入。
    fn ui_audit_records_strip(&mut self, ui: &mut egui::Ui) {
        let mode_key = match self.review_mode {
            ReviewMode::Audit => "audit",
            ReviewMode::Rewrite => "rewrite",
        };
        let mode_text = match self.review_mode {
            ReviewMode::Audit => "审计",
            ReviewMode::Rewrite => "改写",
        };
        let total = self.audit_records.iter().filter(|r| r.mode == mode_key).count();
        theme::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("📚 {mode_text}记录"))
                        .size(12.5)
                        .strong(),
                );
                ui.label(
                    RichText::new(format!("· 共 {total} 条"))
                        .size(11.5)
                        .color(color::TEXT_DIM),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("打开目录").clicked() {
                        if let Some(ref root) = self.novel_path {
                            let dir = audit_log::audit_root(root);
                            let _ = std::fs::create_dir_all(&dir);
                            let _ = open_in_explorer(&dir);
                        }
                    }
                    if ui
                        .small_button("刷新")
                        .on_hover_text("从磁盘重新读取审计记录")
                        .clicked()
                    {
                        if let Some(ref root) = self.novel_path {
                            self.audit_records = audit_log::list_all(root);
                        }
                    }
                });
            });

            if total == 0 {
                ui.add_space(2.0);
                dim_label(
                    ui,
                    "尚无记录。点击右上「开始审计 / AI 改写」生成第一条。",
                );
                return;
            }

            ui.add_space(4.0);
            let mut load_idx: Option<usize> = None;
            egui::ScrollArea::horizontal()
                .id_salt("audit_records_strip_scroll")
                .auto_shrink([false, true])
                .max_height(64.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (idx, rec) in self.audit_records.iter().enumerate() {
                            if rec.mode != mode_key {
                                continue;
                            }
                            let is_selected = self
                                .selected_audit_ts
                                .as_ref()
                                .map(|t| t == &rec.timestamp)
                                .unwrap_or(false);
                            let (bg, fg) = if is_selected {
                                (color::ACCENT_DIM, color::TEXT)
                            } else {
                                (color::SURFACE_HI, color::TEXT)
                            };
                            let resp = egui::Frame::default()
                                .fill(bg)
                                .stroke(Stroke::new(
                                    1.0,
                                    if is_selected { color::ACCENT_HI } else { color::BORDER },
                                ))
                                .corner_radius(CornerRadius::same(6))
                                .inner_margin(Margin::symmetric(10, 6))
                                .show(ui, |ui| {
                                    ui.vertical(|ui| {
                                        ui.label(
                                            RichText::new(format!(
                                                "第 {} 章 · {}",
                                                rec.chapter_no, rec.time
                                            ))
                                            .color(fg)
                                            .size(11.5)
                                            .strong(),
                                        );
                                        ui.label(
                                            RichText::new(format!(
                                                "{} · {} 字 · {:.1}s",
                                                rec.date, rec.chars, rec.elapsed_secs
                                            ))
                                            .color(color::TEXT_DIM)
                                            .size(10.5),
                                        );
                                    });
                                })
                                .response
                                .interact(egui::Sense::click())
                                .on_hover_text(format!(
                                    "点击载入这条{}（{}）",
                                    mode_text, rec.vendor_label
                                ));
                            if resp.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            if resp.clicked() {
                                load_idx = Some(idx);
                            }
                            ui.add_space(6.0);
                        }
                    });
                });
            if let Some(i) = load_idx {
                self.load_audit_record(i);
            }
        });
    }

    fn ui_review_audit_body(&mut self, ui: &mut egui::Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("审计结果").size(13.5).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if let Some(task) = &self.review_task {
                        ui.label(
                            RichText::new(task.stats_label())
                                .color(color::ACCENT_HI)
                                .size(11.5)
                                .monospace(),
                        );
                    }
                });
            });
            dim_label(ui, "已结合「状态档案」一致性检查（流式可在「设置」中开关）");
            ui.add_space(6.0);
            egui::ScrollArea::vertical()
                .id_salt("review_scroll")
                .auto_shrink([false; 2])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if self.review_result.is_empty() {
                        if self.review_task.is_some() {
                            ui.label(
                                RichText::new("⏳  等待模型首个增量…")
                                    .color(color::TEXT_DIM)
                                    .size(12.5),
                            );
                        } else {
                            theme::empty_state(
                                ui,
                                "暂无审计结果",
                                "选择章节与 AI 后，点击「开始审计」",
                            );
                        }
                    } else {
                        let display = if self.review_task.is_some() {
                            format!("{}{}", self.review_result, typewriter_cursor(ui.ctx()))
                        } else {
                            self.review_result.clone()
                        };
                        CommonMarkViewer::new().show(ui, &mut self.cm_cache, &display);
                    }
                });
        });
    }

    fn ui_review_rewrite_body(&mut self, ui: &mut egui::Ui) {
        if let Some(task) = &self.review_task {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("⏳ {}", task.stats_label()))
                        .color(color::ACCENT_HI)
                        .size(11.5)
                        .monospace(),
                );
            });
            ui.add_space(4.0);
        }
        let avail_h = ui.available_height();
        if self.review_compare {
            ui.allocate_ui(egui::vec2(ui.available_width(), avail_h), |ui| {
                ui.columns(2, |cols| {
                    cols[0].vertical(|ui| {
                        ui.label(RichText::new("原文").size(11.5).color(color::TEXT_DIM));
                        ui.add_space(4.0);
                        egui::Frame::default()
                            .fill(color::SURFACE)
                            .stroke(Stroke::new(1.0, color::BORDER))
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(Margin::same(8))
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical()
                                    .id_salt("orig_scroll")
                                    .auto_shrink([false; 2])
                                    .show(ui, |ui| {
                                        ui.add(
                                            TextEdit::multiline(
                                                &mut self.review_target_original.clone(),
                                            )
                                            .desired_width(f32::INFINITY)
                                            .desired_rows(40)
                                            .interactive(false)
                                            .id_salt("orig_text"),
                                        );
                                    });
                            });
                    });
                    cols[1].vertical(|ui| {
                        ui.label(RichText::new("AI 改写").size(11.5).color(color::TEXT_DIM));
                        ui.add_space(4.0);
                        egui::Frame::default()
                            .fill(color::SURFACE)
                            .stroke(Stroke::new(1.0, color::BORDER))
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(Margin::same(8))
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical()
                                    .id_salt("rewrite_scroll")
                                    .auto_shrink([false; 2])
                                    .stick_to_bottom(true)
                                    .show(ui, |ui| {
                                        if self.review_result.is_empty() {
                                            theme::empty_state(
                                                ui,
                                                "暂无 AI 改写",
                                                "点击「AI 改写」让模型生成新版本",
                                            );
                                        } else {
                                            ui.add(
                                                TextEdit::multiline(&mut self.review_result)
                                                    .desired_width(f32::INFINITY)
                                                    .desired_rows(40)
                                                    .id_salt("rewrite_text"),
                                            );
                                        }
                                    });
                            });
                    });
                });
            });
        } else {
            theme::card_frame().show(ui, |ui| {
                ui.label(RichText::new("AI 改写").size(13.5).strong());
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .id_salt("rewrite_only_scroll")
                    .auto_shrink([false; 2])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if self.review_result.is_empty() {
                            theme::empty_state(
                                ui,
                                "暂无 AI 改写",
                                "点击「AI 改写」让模型生成新版本",
                            );
                        } else {
                            ui.add(
                                TextEdit::multiline(&mut self.review_result)
                                    .desired_width(f32::INFINITY)
                                    .desired_rows(40)
                                    .id_salt("rewrite_text_full"),
                            );
                        }
                    });
            });
        }
    }

    /// 在审计/改写模式间切换，保留各自上次的结果不丢失。
    fn swap_review_buffers(&mut self, target: ReviewMode) {
        if self.review_mode == target {
            return;
        }
        // 把当前结果写回旧模式的 buffer
        match self.review_mode {
            ReviewMode::Audit => {
                self.audit_buf = std::mem::take(&mut self.review_result);
                self.audit_buf_for_chapter = self.review_for_chapter.take();
            }
            ReviewMode::Rewrite => {
                self.rewrite_buf = std::mem::take(&mut self.review_result);
                self.rewrite_buf_for_chapter = self.review_for_chapter.take();
            }
        }
        // 从新模式的 buffer 恢复
        match target {
            ReviewMode::Audit => {
                self.review_result = std::mem::take(&mut self.audit_buf);
                self.review_for_chapter = self.audit_buf_for_chapter.take();
            }
            ReviewMode::Rewrite => {
                self.review_result = std::mem::take(&mut self.rewrite_buf);
                self.review_for_chapter = self.rewrite_buf_for_chapter.take();
            }
        }
        self.review_mode = target;
        self.selected_audit_ts = None;
    }

    /// 把指定历史记录加载到当前结果区（保留在历史里）。
    fn load_audit_record(&mut self, idx: usize) {
        let Some(rec) = self.audit_records.get(idx).cloned() else {
            return;
        };
        let target_mode = match rec.mode.as_str() {
            "rewrite" => ReviewMode::Rewrite,
            _ => ReviewMode::Audit,
        };
        if self.review_mode != target_mode {
            self.swap_review_buffers(target_mode);
        }
        self.review_result = rec.content.clone();
        self.review_for_chapter = Some(rec.chapter_no);
        self.review_target = Some(rec.chapter_no);
        // 顺便把对应章节的原文加载，方便对照/替换
        if let Some(ref store) = self.store {
            if let Ok((_, body)) = store.load_chapter_content(rec.chapter_no) {
                self.review_target_original = body;
            }
        }
        self.selected_audit_ts = Some(rec.timestamp.clone());
        self.status_message = format!(
            "已加载历史{} · 第 {} 章 · {} · {} 字",
            rec.mode_label(),
            rec.chapter_no,
            rec.time,
            rec.chars
        );
    }

    fn start_review_audit(&mut self) {
        let Some(n) = self.review_target else { return };
        let vendor_id = self.settings.review_vendor.clone();
        if vendor_id.is_empty() {
            self.status_message = "请先选择审核 AI".into();
            return;
        }
        let Some(cfg) = self.vendor_config(&vendor_id) else {
            self.status_message = "服务商配置不存在".into();
            return;
        };
        if !cfg.is_configured() {
            self.status_message = "所选服务商未完整配置".into();
            return;
        }
        let Some(ref store) = self.store else { return };
        let (title, body) = match store.load_chapter_content(n) {
            Ok(v) => v,
            Err(e) => {
                self.status_message = format!("读取章节失败：{e}");
                return;
            }
        };
        self.review_target_original = body.clone();

        let project_brief = self
            .project
            .as_ref()
            .map(|p| {
                format!(
                    "小说名：{}\n题材：{}\n核心：{}\n",
                    p.title.trim(),
                    p.genre.trim(),
                    p.premise.trim()
                )
            })
            .unwrap_or_default();
        let state_docs = self.collect_state_docs_for_prompt(4000);

        let user_prompt = format!(
            "## 项目背景\n{project_brief}\n## 状态档案（请据此严格核对一致性）\n{}\n\n## 章节标题\n{title}\n\n## 章节正文\n{body}",
            if state_docs.is_empty() { "（无）".to_string() } else { state_docs }
        );

        let messages = vec![
            ChatMessage::system(
                "你是一名严谨的网文/小说审稿编辑。请用 Markdown 输出审稿意见，包含：\n\
                 1. 总体评分（1-10）；\n\
                 2. 优点（不超过 5 条）；\n\
                 3. 问题与改进建议（按重要性排序）；\n\
                 4. 与「状态档案 / 人物 / 世界 / 已埋伏笔」的一致性检查（指出具体出处）；\n\
                 5. 一段示范修改（任选一处问题）。\n不要复述原文。",
            ),
            ChatMessage::user(user_prompt),
        ];
        let model = cfg.model.clone();
        self.review_result.clear();
        self.review_for_chapter = Some(n);
        let streaming = self.settings.review_streaming;
        self.review_task = Some(spawn_chat(cfg, vendor_id, model, messages, streaming));
        self.status_message = format!("已发起审计：第 {n} 章（流式：{}）", if streaming { "开" } else { "关" });
        oplog::try_append(
            self.novel_path.as_deref(),
            "AI 审计 · 启动",
            &format!("第 {n} 章 · 服务商 {}", Self::vendor_label(&self.settings.review_vendor)),
        );
    }

    fn start_review_rewrite(&mut self) {
        let Some(n) = self.review_target else { return };
        let vendor_id = self.settings.review_vendor.clone();
        if vendor_id.is_empty() {
            self.status_message = "请先选择审核 AI".into();
            return;
        }
        let Some(cfg) = self.vendor_config(&vendor_id) else {
            self.status_message = "服务商配置不存在".into();
            return;
        };
        if !cfg.is_configured() {
            self.status_message = "所选服务商未完整配置".into();
            return;
        }
        let Some(ref store) = self.store else { return };
        let (title, body) = match store.load_chapter_content(n) {
            Ok(v) => v,
            Err(e) => {
                self.status_message = format!("读取章节失败：{e}");
                return;
            }
        };
        self.review_target_original = body.clone();

        let project_brief = self
            .project
            .as_ref()
            .map(|p| {
                format!(
                    "小说名：{}\n题材：{}\n核心：{}\n",
                    p.title.trim(),
                    p.genre.trim(),
                    p.premise.trim()
                )
            })
            .unwrap_or_default();
        let state_docs = self.collect_state_docs_for_prompt(4000);

        // 找到本章最新一条「审计」记录（audit_records 已按 timestamp 倒序）
        let latest_audit = self
            .audit_records
            .iter()
            .find(|r| r.mode == "audit" && r.chapter_no == n)
            .cloned();
        let audit_block = if let Some(ref rec) = latest_audit {
            let snippet: String = rec.content.chars().take(4000).collect();
            let truncated_hint = if rec.content.chars().count() > 4000 {
                "\n…（已截断）"
            } else {
                ""
            };
            format!(
                "时间：{} · 服务商：{} · {} 字\n\n{snippet}{truncated_hint}",
                rec.timestamp, rec.vendor_label, rec.chars
            )
        } else {
            "（无最近审计，按状态档案直接改写）".to_string()
        };

        let user_prompt = format!(
            "## 项目背景\n{project_brief}\n## 状态档案（务必保持一致）\n{}\n\n## 最近一次审计意见\n{audit_block}\n\n## 章节标题\n{title}\n\n## 原文\n{body}",
            if state_docs.is_empty() { "（无）".to_string() } else { state_docs }
        );

        let messages = vec![
            ChatMessage::system(
                "你是一名一线网文/小说编辑。请基于下面的项目背景、状态档案、最近一次审计意见与原文，输出**完整的改写后章节正文**：\n\
                 - 仅输出改写后的正文，不要解释、不要前言、不要代码块包裹；\n\
                 - 优先按「最近一次审计意见」中的「问题与改进建议」逐项修复；\n\
                 - 严格遵守 book_rules.md 中的 personalityLock / behavioralConstraints / prohibitions / forbidden 列表；\n\
                 - 保持人物动机、关键事件、伏笔与原章节一致；\n\
                 - 修复语病、强化节奏、增强画面感；\n\
                 - 保留中文标点习惯。",
            ),
            ChatMessage::user(user_prompt),
        ];
        let model = cfg.model.clone();
        self.review_result.clear();
        self.review_for_chapter = Some(n);
        let streaming = self.settings.review_streaming;
        self.review_task = Some(spawn_chat(cfg, vendor_id, model, messages, streaming));
        let audit_hint = if let Some(ref rec) = latest_audit {
            format!("（已引用审计 {}）", rec.time)
        } else {
            "（无可引用审计）".to_string()
        };
        self.status_message = format!(
            "已发起 AI 改写：第 {n} 章（流式：{}）{audit_hint}",
            if streaming { "开" } else { "关" }
        );
        oplog::try_append(
            self.novel_path.as_deref(),
            "AI 改写 · 启动",
            &format!(
                "第 {n} 章 · 服务商 {} · {audit_hint}",
                Self::vendor_label(&self.settings.review_vendor)
            ),
        );
        if let Some(ref rec) = latest_audit {
            oplog::try_append(
                self.novel_path.as_deref(),
                "AI 改写 · 引用审计",
                &format!("第 {n} 章 · 审计时间 {} · {} 字", rec.timestamp, rec.chars),
            );
        }
    }

    fn replace_chapter_with_rewrite(&mut self) {
        let Some(n) = self.review_for_chapter else { return };
        let new_body = self.review_result.trim().to_string();
        if new_body.is_empty() {
            self.status_message = "改写结果为空，无法替换".into();
            return;
        }
        let novel_root = self.novel_path.clone();
        let (Some(store), Some(project)) = (&self.store, &mut self.project) else { return };
        let cur_title = project
            .chapters
            .iter()
            .find(|c| c.number == n)
            .map(|c| c.title.clone())
            .unwrap_or_default();
        let old_body = if !self.review_target_original.is_empty() {
            self.review_target_original.clone()
        } else {
            store.load_chapter_content(n).map(|(_, b)| b).unwrap_or_default()
        };

        if let Some(ref root) = novel_root {
            match history::snapshot_chapter(
                root,
                n,
                &cur_title,
                &old_body,
                &new_body,
                "ai_rewrite",
                "AI 改写替换原文",
            ) {
                Ok(rev) => {
                    self.auto_gen_log.push(format!(
                        "{}  备份原版至 {}",
                        short_time(),
                        rev.backup_file
                    ));
                    oplog::try_append(
                        Some(root),
                        "章节备份",
                        &format!(
                            "第 {n} 章 · {} 字 → {} 字 · 文件 {}",
                            rev.old_chars, rev.new_chars, rev.backup_file
                        ),
                    );
                }
                Err(e) => {
                    self.status_message = format!("备份原版失败：{e}");
                }
            }
        }

        let save_result = store.save_chapter(project, n, &cur_title, &new_body, "review", "");
        match save_result {
            Ok(()) => {
                self.status_message = format!("已用 AI 改写替换第 {n} 章正文（状态置为待审核）");
                oplog::try_append(
                    novel_root.as_deref(),
                    "替换原文",
                    &format!("第 {n} 章 · 来自 AI 改写"),
                );
                if self.selected_chapter == Some(n) {
                    self.select_chapter(n);
                }
                // 链式触发状态档案同步（第二次 LLM 调用）
                self.start_state_sync(n, &old_body, &new_body);
            }
            Err(e) => self.status_message = format!("替换失败：{e}"),
        }
    }

    /// 替换原文成功后链式触发的「状态档案同步」：
    /// 让审计 LLM 基于（旧正文/新正文/状态档案/最近审计）输出 JSON 差异，再写回 `story_state/*.md`。
    fn start_state_sync(&mut self, n: i32, old_body: &str, new_body: &str) {
        if self.state_sync_task.is_some() {
            self.status_message = "已有状态档案同步任务在进行中".into();
            return;
        }
        let vendor_id = self.settings.review_vendor.clone();
        if vendor_id.is_empty() {
            self.state_sync_log
                .push(format!("{}  跳过同步：未选择审计 LLM", short_time()));
            return;
        }
        let Some(cfg) = self.vendor_config(&vendor_id) else {
            self.state_sync_log
                .push(format!("{}  跳过同步：服务商配置缺失", short_time()));
            return;
        };
        if !cfg.is_configured() {
            self.state_sync_log
                .push(format!("{}  跳过同步：服务商未完整配置", short_time()));
            return;
        }

        // 准备 prompt 上下文（先收集再借用 self）
        let state_docs = self.collect_state_docs_for_prompt(3000);
        let project_brief = self
            .project
            .as_ref()
            .map(|p| {
                format!(
                    "小说名：{}\n题材：{}\n核心：{}\n",
                    p.title.trim(),
                    p.genre.trim(),
                    p.premise.trim()
                )
            })
            .unwrap_or_default();
        let latest_audit_block = self
            .audit_records
            .iter()
            .find(|r| r.mode == "audit" && r.chapter_no == n)
            .map(|r| {
                let snippet: String = r.content.chars().take(2500).collect();
                format!(
                    "时间：{} · {} 字\n\n{snippet}",
                    r.timestamp, r.chars
                )
            })
            .unwrap_or_else(|| "（无）".into());
        let allowed_files: String = STATE_FILES
            .iter()
            .filter(|f| **f != "chapter_summaries.md")
            .map(|f| format!("- {f}"))
            .collect::<Vec<_>>()
            .join("\n");

        let system_prompt = format!(
            "你是一名严谨的小说世界观/状态档案维护编辑。\n\
             基于「旧正文 → 新正文」的改动，更新下列状态档案文件，使其与新正文保持一致：\n{allowed_files}\n\n\
             严格输出**单个 JSON 对象**，禁止任何额外说明或 Markdown 围栏。Schema：\n\
             {{\n  \"summary\": \"本次改动一句话摘要\",\n  \"updates\": [\n    {{ \"file\": \"current_state.md\", \"action\": \"replace\" | \"patch\", \"content\": \"...\" }}\n  ]\n}}\n\n\
             - action=replace：content 必须是该文件的完整新版本（含原有 frontmatter / 标题等结构）。\n\
             - action=patch：content 由若干块组成，每块格式：\n\
               ===REPLACE_BLOCK===\\n旧片段（需精确匹配现文件中的连续子串）\\n===WITH===\\n新片段\\n===END===\n\
             - 没有需要变更的文件就不要列出，updates 可以为空。\n\
             - 严禁更新 chapter_summaries.md 与 book_rules.md（前者由系统生成，后者是硬约束不应被自动改动）。\n\
             - 所有改动必须忠实反映新正文事件，不得引入新设定。"
        );

        let user_prompt = format!(
            "## 项目背景\n{project_brief}\n## 当前状态档案\n{}\n\n## 最近一次审计\n{latest_audit_block}\n\n## 旧正文（替换前）\n{old_body}\n\n## 新正文（替换后）\n{new_body}",
            if state_docs.is_empty() { "（无）".to_string() } else { state_docs }
        );

        let messages = vec![
            ChatMessage::system(system_prompt),
            ChatMessage::user(user_prompt),
        ];
        let model = cfg.model.clone();
        // 状态同步强制非流式：避免半截 JSON 解析失败
        self.state_sync_task = Some(spawn_chat(cfg, vendor_id.clone(), model, messages, false));
        self.state_sync_phase = StateSyncPhase::Running { chapter_no: n };
        self.state_sync_log.push(format!(
            "{}  启动状态档案同步（第 {n} 章 · 服务商 {}）",
            short_time(),
            Self::vendor_label(&vendor_id)
        ));
        oplog::try_append(
            self.novel_path.as_deref(),
            "状态档案同步 · 启动",
            &format!("第 {n} 章 · 服务商 {}", Self::vendor_label(&vendor_id)),
        );
    }

    fn handle_state_sync_done(&mut self) {
        let Some(task) = self.state_sync_task.take() else {
            self.state_sync_phase = StateSyncPhase::Idle;
            return;
        };
        let chapter_no = match self.state_sync_phase {
            StateSyncPhase::Running { chapter_no } => chapter_no,
            StateSyncPhase::Idle => 0,
        };
        self.state_sync_phase = StateSyncPhase::Idle;

        if let Some(e) = task.error.clone() {
            self.state_sync_log
                .push(format!("{}  同步失败：{e}", short_time()));
            self.status_message = format!("状态档案同步失败：{e}");
            oplog::try_append(
                self.novel_path.as_deref(),
                "状态档案同步 · 失败",
                &format!("第 {chapter_no} 章 · {e}"),
            );
            return;
        }

        let raw = task.accumulated.clone();
        let report = match state_sync::parse_state_updates(&raw) {
            Ok(r) => r,
            Err(e) => {
                self.state_sync_log
                    .push(format!("{}  解析失败：{e}", short_time()));
                self.status_message = format!("状态档案同步：解析返回失败：{e}");
                oplog::try_append(
                    self.novel_path.as_deref(),
                    "状态档案同步 · 失败",
                    &format!("第 {chapter_no} 章 · 解析失败：{e}"),
                );
                return;
            }
        };

        let Some(novel_root) = self.novel_path.clone() else {
            return;
        };
        let Some(state_dir) = self.store.as_ref().map(|s| s.state_dir()) else {
            return;
        };
        let changes: Vec<StateFileChange> =
            state_sync::apply_updates(&novel_root, &state_dir, &report, STATE_FILES);

        if changes.is_empty() {
            self.state_sync_log.push(format!(
                "{}  LLM 未提出任何变更（{}）",
                short_time(),
                if report.summary.is_empty() {
                    "无摘要"
                } else {
                    report.summary.as_str()
                }
            ));
            self.status_message = "状态档案同步完成：无变更".into();
            oplog::try_append(
                Some(&novel_root),
                "状态档案同步 · 完成",
                &format!("第 {chapter_no} 章 · 0 个文件 · 摘要：{}", report.summary),
            );
            return;
        }

        let mut applied = 0usize;
        let mut total_delta: i64 = 0;
        let mut applied_files: Vec<String> = Vec::new();
        for c in &changes {
            if c.new_chars != 0 || c.note.starts_with("patch") {
                applied += 1;
                total_delta += c.delta();
                applied_files.push(c.file.clone());
            }
            self.state_sync_log.push(format!(
                "{}  · {} [{}] {} → {} 字（Δ{:+}） {}",
                short_time(),
                c.file,
                c.action,
                c.old_chars,
                c.new_chars,
                c.delta(),
                c.note
            ));
        }
        self.state_sync_log.push(format!(
            "{}  完成：{} 个文件 · 总Δ {:+} 字 · 摘要：{}",
            short_time(),
            applied,
            total_delta,
            if report.summary.is_empty() {
                "(无)"
            } else {
                report.summary.as_str()
            }
        ));
        self.status_message = format!(
            "🗂 已同步 {} 个状态档案 · 总{:+} 字",
            applied, total_delta
        );
        oplog::try_append(
            Some(&novel_root),
            "状态档案同步 · 完成",
            &format!(
                "第 {chapter_no} 章 · {} 个文件 [{}] · 总Δ {:+} 字 · 摘要：{}",
                applied,
                applied_files.join(", "),
                total_delta,
                report.summary
            ),
        );

        // 状态文件已变更：刷新写作页伏笔提醒，并重载当前正在编辑的状态文档（如果命中）
        self.refresh_pending_hooks();
        if self.nm_tab == NovelMetaTab::StateDocs {
            let cur_file = self.selected_state_file.clone();
            if applied_files.iter().any(|f| f == &cur_file) {
                self.load_state_doc();
            }
        }
    }

    // ---------- 写作助手 ----------
    fn ui_assistant(&mut self, ui: &mut egui::Ui) {
        let configured = self.configured_vendor_ids();
        if self.settings.writing_vendor.is_empty() {
            self.settings.writing_vendor = configured.first().cloned().unwrap_or_default();
        }

        theme::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("写作 LLM").size(11.5).color(color::TEXT_DIM));
                let sel = if self.settings.writing_vendor.is_empty() {
                    "（未选择）".into()
                } else {
                    Self::vendor_label(&self.settings.writing_vendor)
                };
                ComboBox::from_id_salt("assist_vendor")
                    .width(220.0)
                    .selected_text(sel)
                    .show_ui(ui, |ui| {
                        if configured.is_empty() {
                            ui.label("（请先在「设置」中配置服务商）");
                        }
                        for id in &configured {
                            if ui
                                .selectable_label(
                                    self.settings.writing_vendor == *id,
                                    Self::vendor_label(id),
                                )
                                .clicked()
                            {
                                self.settings.writing_vendor = id.clone();
                                let _ = self.paths.save_settings(&self.settings);
                            }
                        }
                    });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !self.assistant_log.is_empty() && ui.button("清空对话").clicked() {
                        self.assistant_log.clear();
                    }
                });
            });
        });

        ui.add_space(8.0);
        let avail_h = ui.available_height();
        ui.allocate_ui(egui::vec2(ui.available_width(), avail_h), |ui| {
            theme::card_frame().show(ui, |ui| {
                let log_h = (ui.available_height() - 130.0).max(160.0);
                let (live, stats) = self
                    .assistant_task
                    .as_ref()
                    .map(|t| (t.accumulated.clone(), t.stats_label()))
                    .unwrap_or_default();
                let cursor = typewriter_cursor(ui.ctx());
                egui::ScrollArea::vertical()
                    .id_salt("chat_scroll")
                    .max_height(log_h)
                    .auto_shrink([false; 2])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if self.assistant_log.is_empty() && self.assistant_task.is_none() {
                            theme::empty_state(ui, "开始一次对话", "在下方输入你的问题或写作需求");
                        }
                        for (role, msg) in &self.assistant_log {
                            Self::chat_bubble(ui, role, msg, None);
                        }
                        if self.assistant_task.is_some() {
                            let body = if live.is_empty() {
                                format!("⏳  正在思考…{cursor}")
                            } else {
                                format!("{live}{cursor}")
                            };
                            Self::chat_bubble(ui, "assistant", &body, Some(&stats));
                        }
                    });

                ui.add_space(8.0);
                let r = ui.add(
                    TextEdit::multiline(&mut self.assistant_input)
                        .desired_width(f32::INFINITY)
                        .desired_rows(3)
                        .hint_text("输入对助手的问题或指令…  (Ctrl+Enter 发送)"),
                );
                ui.horizontal(|ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let send_btn = ui
                            .add_enabled(self.assistant_task.is_none(), egui::Button::new("发送  ↵"))
                            .clicked();
                        let send_kbd = r.has_focus()
                            && ui.input(|i| {
                                i.key_pressed(egui::Key::Enter)
                                    && (i.modifiers.ctrl || i.modifiers.command)
                            });
                        if (send_btn || send_kbd)
                            && !self.assistant_input.trim().is_empty()
                            && self.assistant_task.is_none()
                        {
                            self.send_to_assistant();
                        }
                    });
                });
            });
        });
    }

    fn chat_bubble(ui: &mut egui::Ui, role: &str, msg: &str, stats: Option<&str>) {
        let (label, bg) = if role == "user" {
            ("你", color::ACCENT_DIM)
        } else {
            ("助手", color::SURFACE_HI)
        };
        egui::Frame::default()
            .fill(bg)
            .stroke(Stroke::new(1.0, color::BORDER))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(label).color(color::TEXT_DIM).size(11.0).strong());
                    if let Some(s) = stats {
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                ui.label(
                                    RichText::new(s)
                                        .color(color::TEXT_FAINT)
                                        .size(10.5)
                                        .monospace(),
                                );
                            },
                        );
                    }
                });
                ui.label(RichText::new(msg).color(color::TEXT).size(13.0));
            });
        ui.add_space(6.0);
    }

    fn send_to_assistant(&mut self) {
        let vendor_id = self.settings.writing_vendor.clone();
        if vendor_id.is_empty() {
            self.status_message = "请先选择写作 LLM".into();
            return;
        }
        let Some(cfg) = self.vendor_config(&vendor_id) else {
            self.status_message = "服务商配置不存在".into();
            return;
        };
        if !cfg.is_configured() {
            self.status_message = "所选服务商未完整配置".into();
            return;
        }
        let user_msg = self.assistant_input.trim().to_string();
        if user_msg.is_empty() {
            return;
        }
        self.assistant_input.clear();
        self.assistant_log.push(("user".into(), user_msg.clone()));

        // 收集状态档案供 system prompt 注入（先收集再借 self，避免与后续可变借用冲突）
        let state_docs = self.collect_state_docs_for_prompt(2000);
        let state_files_loaded: Vec<&'static str> = if state_docs.is_empty() {
            Vec::new()
        } else {
            STATE_FILES.iter().copied().collect()
        };

        let mut messages: Vec<ChatMessage> = Vec::new();
        let project_brief = if let Some(ref p) = self.project {
            format!(
                "小说：{}（题材：{}）。\n核心：{}\n",
                p.title.trim(),
                p.genre.trim(),
                p.premise.trim()
            )
        } else {
            String::new()
        };
        let sys = if !project_brief.is_empty() || !state_docs.is_empty() {
            let mut s = String::from(
                "你是一位长期跟进当前小说的写作助手。\n\
                 在回答前请通读「项目背景」与「状态档案」，所有建议、续写、人物分析都必须严格遵守 book_rules.md 中的 personalityLock / behavioralConstraints / prohibitions / forbidden 列表，并与 current_state.md / pending_hooks.md / character_matrix.md 等档案保持一致；\n\
                 若用户提出的需求与状态档案冲突，请先指出冲突再给出兼容方案，不要默默改动设定。\n\n",
            );
            if !project_brief.is_empty() {
                s.push_str("## 项目背景\n");
                s.push_str(&project_brief);
                s.push('\n');
            }
            if !state_docs.is_empty() {
                s.push_str("## 状态档案（务必保持一致）\n");
                s.push_str(&state_docs);
            }
            s
        } else {
            "你是一位耐心、专业的中文小说写作助手。".into()
        };
        messages.push(ChatMessage::system(sys));

        for (role, content) in &self.assistant_log {
            match role.as_str() {
                "user" => messages.push(ChatMessage::user(content.clone())),
                "assistant" => messages.push(ChatMessage::assistant(content.clone())),
                _ => {}
            }
        }

        let model = cfg.model.clone();
        let streaming = self.settings.writing_streaming;
        self.assistant_task = Some(spawn_chat(cfg, vendor_id, model, messages, streaming));
        if !state_files_loaded.is_empty() {
            self.status_message = format!(
                "写作助手已注入 {} 个状态档案作为上下文",
                state_files_loaded.len()
            );
            oplog::try_append(
                self.novel_path.as_deref(),
                "写作助手 · 发送",
                &format!("已注入状态档案：{}", state_files_loaded.join(", ")),
            );
        } else {
            oplog::try_append(
                self.novel_path.as_deref(),
                "写作助手 · 发送",
                "未注入状态档案（无项目）",
            );
        }
    }

    // ---------- 定时写作 ----------
    fn tick_auto_gen(&mut self, ctx: &egui::Context) {
        let interval_min = self
            .project
            .as_ref()
            .map(|p| p.auto_generate.interval_minutes.max(1))
            .unwrap_or(30);
        let enabled = self
            .project
            .as_ref()
            .map(|p| p.auto_generate.enabled)
            .unwrap_or(false);

        if !enabled || self.auto_gen_task.is_some() {
            return;
        }
        let elapsed = self.auto_gen_last_tick.elapsed();
        let target = std::time::Duration::from_secs(interval_min as u64 * 60);
        if elapsed >= target {
            self.start_auto_gen();
        } else {
            ctx.request_repaint_after(target - elapsed);
        }
    }

    fn start_auto_gen(&mut self) {
        if self.auto_gen_task.is_some() {
            return;
        }
        let Some(project) = self.project.as_ref() else { return };
        let next_n = project.chapters.iter().map(|c| c.number).max().unwrap_or(0) + 1;
        let prev_n = if next_n > 1 { Some(next_n - 1) } else { None };
        let want_audit = self.settings.auto_gen_audit_first
            && prev_n.is_some()
            && !self.settings.review_vendor.is_empty();

        if want_audit {
            self.start_auto_gen_audit_phase(prev_n.unwrap(), next_n);
        } else {
            self.auto_gen_audit_text.clear();
            self.start_auto_gen_writing_phase(next_n);
        }
        self.auto_gen_last_tick = Instant::now();
    }

    fn start_auto_gen_audit_phase(&mut self, prev_n: i32, next_n: i32) {
        let vendor_id = self.settings.review_vendor.clone();
        let Some(cfg) = self.vendor_config(&vendor_id) else {
            self.auto_gen_log.push(format!(
                "{}  审计 LLM 配置缺失，直接写作",
                short_time()
            ));
            self.start_auto_gen_writing_phase(next_n);
            return;
        };
        if !cfg.is_configured() {
            self.auto_gen_log.push(format!(
                "{}  审计 LLM 未配置完整，直接写作",
                short_time()
            ));
            self.start_auto_gen_writing_phase(next_n);
            return;
        }

        let state_docs = self.collect_state_docs_for_prompt(3000);

        let (title, body, project_brief) = {
            let Some(store) = self.store.as_ref() else { return };
            let Some(project) = self.project.as_ref() else { return };
            let (t, b) = match store.load_chapter_content(prev_n) {
                Ok(v) => v,
                Err(e) => {
                    self.auto_gen_log
                        .push(format!("{}  读取第 {prev_n} 章失败：{e}", short_time()));
                    self.start_auto_gen_writing_phase(next_n);
                    return;
                }
            };
            let pb = format!(
                "小说名：{}\n题材：{}\n核心：{}\n",
                project.title.trim(),
                project.genre.trim(),
                project.premise.trim()
            );
            (t, b, pb)
        };

        let user_prompt = format!(
            "## 项目背景\n{project_brief}\n## 状态档案\n{}\n\n## 上一章标题\n{title}\n\n## 上一章正文\n{body}",
            if state_docs.is_empty() { "（无）".to_string() } else { state_docs }
        );

        let messages = vec![
            ChatMessage::system(
                "你是一名严谨的网文/小说审稿编辑。请用 Markdown 简洁输出对上一章的审计要点，便于作者在写下一章前据此修正：\n\
                 1. 总评（1-10 分，附一句话）；\n\
                 2. 必须修复的问题（不超过 5 条，按重要性排序）；\n\
                 3. 与状态档案/伏笔的不一致点（指出文件名）；\n\
                 4. 写下一章应延续的悬念与情绪；\n\
                 5. 节奏建议。",
            ),
            ChatMessage::user(user_prompt),
        ];

        let model = cfg.model.clone();
        let streaming = self.settings.review_streaming;
        self.auto_gen_phase = AutoGenPhase::AuditingPrev { prev_n, next_n };
        self.auto_gen_audit_text.clear();
        self.auto_gen_task = Some(spawn_chat(cfg, vendor_id, model, messages, streaming));
        self.auto_gen_log.push(format!(
            "{}  链式工作流：先审计第 {prev_n} 章 → 再写第 {next_n} 章",
            short_time()
        ));
        oplog::try_append(
            self.novel_path.as_deref(),
            "定时写作 · 审计上一章",
            &format!("第 {prev_n} 章 → 准备第 {next_n} 章"),
        );
    }

    fn start_auto_gen_writing_phase(&mut self, next_n: i32) {
        let vendor_id = self.settings.writing_vendor.clone();
        if vendor_id.is_empty() {
            self.auto_gen_log
                .push(format!("{}  写作 LLM 未选择，跳过", short_time()));
            self.auto_gen_phase = AutoGenPhase::Idle;
            return;
        }
        let Some(cfg) = self.vendor_config(&vendor_id) else {
            self.auto_gen_log
                .push(format!("{}  服务商配置缺失", short_time()));
            self.auto_gen_phase = AutoGenPhase::Idle;
            return;
        };
        if !cfg.is_configured() {
            self.auto_gen_log
                .push(format!("{}  服务商未完整配置", short_time()));
            self.auto_gen_phase = AutoGenPhase::Idle;
            return;
        }
        let state_docs = self.collect_state_docs_for_prompt(3000);
        let (summaries, novel_brief) = {
            let Some(project) = self.project.as_ref() else { return };
            let Some(store) = self.store.as_ref() else { return };
            let summaries = store.build_chapter_summaries_document(project);
            let novel_brief = format!(
                "书名：{}\n题材：{}\n字数目标：约 {} 字/章\n核心：{}\n主角：{}\n世界：{}\n文风：{}\n大纲：{}\n额外提示：{}",
                project.title.trim(),
                project.genre.trim(),
                project.chapter_word_goal,
                project.premise.trim(),
                project.protagonists.trim(),
                project.world_setting.trim(),
                project.writing_style.trim(),
                project.outline.trim(),
                project.extra_guidance.trim(),
            );
            (summaries, novel_brief)
        };
        let audit_block = if self.auto_gen_audit_text.trim().is_empty() {
            "（无）".to_string()
        } else {
            self.auto_gen_audit_text.trim().to_string()
        };

        let messages = vec![
            ChatMessage::system(
                "你是一位中文长篇小说作者。请按用户提供的设定、状态档案、历史摘要与上一章审计意见，撰写下一章的完整正文：\n\
                 - 输出格式必须是 Markdown，第一行为 `# 章节标题`，后跟正文；\n\
                 - 不要输出任何解释或元信息；\n\
                 - 保持节奏紧凑，画面感强；\n\
                 - 严格遵循审计意见中的「必须修复点」与「应延续的悬念」；\n\
                 - 与状态档案、已埋伏笔严格保持一致。",
            ),
            ChatMessage::user(format!(
                "## 小说设定\n{novel_brief}\n\n## 状态档案\n{}\n\n## 历史章节摘要\n{summaries}\n\n## 上一章审计要点\n{audit_block}\n\n请创作【第 {next_n} 章】。",
                if state_docs.is_empty() { "（无）".to_string() } else { state_docs }
            )),
        ];

        let model = cfg.model.clone();
        let streaming = self.settings.writing_streaming;
        self.auto_gen_phase = AutoGenPhase::Writing { next_n };
        self.auto_gen_task = Some(spawn_chat(cfg, vendor_id, model, messages, streaming));
        self.auto_gen_log
            .push(format!("{}  开始写作第 {next_n} 章", short_time()));
        oplog::try_append(
            self.novel_path.as_deref(),
            "定时写作 · 写下一章",
            &format!("第 {next_n} 章"),
        );
    }

    /// 手动「AI 生成本章」（对齐 inkoswin `generate_current_chapter` + `build_generation_prompts`）。
    ///
    /// - Prompt 主体严格沿用 inkoswin：小说设定 / 前文摘要 / 最近 3 章节选 / 连续性档案 / 当前草稿 / 输出格式；
    /// - 当前章节编辑器里的正文作为「当前章节现有草稿」注入；
    /// - 若本项目启用了 story 控制层（author_intent / current_focus）或存在上一章审计，则在末尾追加「桌面端扩展」段落；
    /// - 不自动落盘，流式结果先填入编辑器，用户确认后再 💾 保存。
    fn start_manual_chapter_generation(&mut self, target_n: i32) {
        self.pending_gen_confirm = None;
        if self.manual_gen_task.is_some() {
            self.status_message = "已有生成任务在进行中".into();
            return;
        }
        let vendor_id = self.settings.writing_vendor.clone();
        if vendor_id.is_empty() {
            self.status_message = "未选择写作 LLM（设置 → 写作）".into();
            return;
        }
        let Some(cfg) = self.vendor_config(&vendor_id) else {
            self.status_message = format!("服务商「{vendor_id}」配置缺失");
            return;
        };
        if !cfg.is_configured() {
            self.status_message = "写作 LLM 服务商未完整配置".into();
            return;
        }

        // --- 构造 inkoswin 风格数据 ---
        let Some(project_ref) = self.project.as_ref() else {
            self.status_message = "未打开项目".into();
            return;
        };
        let Some(store) = self.store.as_ref() else {
            self.status_message = "项目存储未就绪".into();
            return;
        };
        let project_snapshot = project_ref.clone();

        // 1. target chapter：若 project 里还不存在（例如手动点在一个空白编号），构造一个占位记录。
        let target_chapter = project::ProjectStore::get_chapter(&project_snapshot, target_n)
            .cloned()
            .unwrap_or_else(|| crate::project::ChapterRecord {
                number: target_n,
                title: if !self.chapter_title.trim().is_empty() {
                    self.chapter_title.trim().to_string()
                } else {
                    format!("第{target_n}章")
                },
                summary: self.chapter_summary.trim().to_string(),
                status: "draft".into(),
                word_count: 0,
                created_at: crate::project::now_iso(),
                updated_at: crate::project::now_iso(),
            });

        // 2. previous_materials：target 之前、正文非空的章节，按 number 升序。
        let mut previous_materials: Vec<(crate::project::ChapterRecord, String)> = Vec::new();
        let mut sorted: Vec<&crate::project::ChapterRecord> =
            project_snapshot.chapters.iter().collect();
        sorted.sort_by_key(|c| c.number);
        for ch in sorted {
            if ch.number >= target_n {
                break;
            }
            if let Ok((_t, body)) = store.load_chapter_content(ch.number) {
                if !body.trim().is_empty() {
                    previous_materials.push((ch.clone(), body));
                }
            }
        }

        // 3. state_documents：读 story_state/ 9 个档案
        let mut state_docs: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for f in STATE_FILES {
            let path = store.state_dir().join(f);
            if let Ok(text) = fs::read_to_string(&path) {
                if !text.trim().is_empty() {
                    state_docs.insert((*f).to_string(), text);
                }
            }
        }

        // 4. current_draft：编辑器里的当前正文
        let current_draft = self.chapter_body.clone();

        // 5. 构造主体 prompt（inkoswin 原版）
        let (system_prompt, mut user_prompt) = inkoswin_prompt::build_generation_prompts(
            &project_snapshot,
            &target_chapter,
            &previous_materials,
            &current_draft,
            &state_docs,
        );

        // 6. 桌面端扩展（可选增强，不影响核心对齐）：story 控制层 + 上一章审计 + 字数治理提醒
        let story_block = self.collect_story_docs_for_prompt(1200);
        if !story_block.trim().is_empty() {
            user_prompt.push_str("\n\n[story 控制层]\n");
            user_prompt.push_str(story_block.trim());
        }
        let last_audit_block = self.load_last_audit_block(target_n);
        if !last_audit_block.is_empty()
            && !last_audit_block.starts_with('（')
            && !last_audit_block.starts_with('(')
        {
            let clipped_audit = if last_audit_block.chars().count() > 1200 {
                let cut: String = last_audit_block.chars().take(1200).collect();
                format!("{cut}\n…（已截断）")
            } else {
                last_audit_block.clone()
            };
            user_prompt.push_str("\n\n[上一轮审计要点]\n");
            user_prompt.push_str(clipped_audit.trim());
        }
        let tol = self.settings.effective_word_tolerance();
        if project_snapshot.chapter_word_goal > 0 {
            user_prompt.push_str(&format!(
                "\n\n[字数治理]\n目标约 {} 字（容差 ±{tol}），不得硬截断。",
                project_snapshot.chapter_word_goal
            ));
        }

        let messages = vec![
            ChatMessage::system(&system_prompt),
            ChatMessage::user(user_prompt),
        ];

        let model = cfg.model.clone();
        let streaming = self.settings.writing_streaming;
        self.manual_gen_task = Some(spawn_chat(cfg, vendor_id, model, messages, streaming));
        self.manual_gen_target = Some(target_n);
        self.status_message = format!("✨ 正在生成第 {target_n} 章…");
        oplog::try_append(
            self.novel_path.as_deref(),
            "AI 生成本章 · 开始",
            &format!("第 {target_n} 章"),
        );
    }

    /// 对齐 inkoswin `generate_next_chapter`：
    /// 1) 保存当前章节（若 dirty）；
    /// 2) `next_pending_chapter_number`：找第一个正文为空的章节，或 `max+1`（若未超 target_chapters）；
    /// 3) `ensure_chapter` 创建占位章节（若需要）；
    /// 4) 选中并加载到编辑器，然后触发生成。
    fn start_generate_next_chapter(&mut self) {
        self.pending_gen_confirm = None;
        if self.manual_gen_task.is_some() {
            self.status_message = "已有生成任务在进行中".into();
            return;
        }
        if self.store.is_none() || self.project.is_none() {
            self.status_message = "请先打开小说目录".into();
            return;
        }
        // 若当前章节有未保存改动，先保存。保存成功后可能触发「章节保存后自动刷新长期记忆」，
        // 若发生则把实际生成动作挂起到刷新完成后，避免下一章用到过期的 [连续性档案]。
        if self.chapter_dirty || self.chapter_summary_dirty {
            self.save_current_chapter();
            if self.chapter_dirty || self.chapter_summary_dirty {
                return;
            }
        }
        if !self.settings.fast_generate_next_chapter
            && (self.state_refresh_batch.is_some() || self.state_refresh_task.is_some())
        {
            // 刷新在跑：挂起，等待 finish_state_refresh_batch_success 里恢复流程。
            self.pending_next_chapter_after_refresh = true;
            self.status_message =
                "长期记忆档案正在刷新，稍后将自动继续生成下一章…".into();
            oplog::try_append(
                self.novel_path.as_deref(),
                "AI 生成下一章 · 挂起",
                "等待 AI 刷新长期记忆档案完成后继续",
            );
            return;
        }
        self.resume_generate_next_chapter();
    }

    /// `start_generate_next_chapter` 的后半段：定位/新建下一空章 → 选中 → 触发生成。
    /// 拆成独立方法，便于「章节保存 → 长期记忆刷新完成」后恢复流程时复用。
    fn resume_generate_next_chapter(&mut self) {
        let Some(target_n) = self.next_pending_chapter_number() else {
            self.status_message = "已达到目标章节数，暂无下一章可生成".into();
            return;
        };
        if let (Some(store), Some(project)) = (&self.store, &mut self.project) {
            if let Err(e) = store.ensure_chapter(project, target_n, &format!("第{target_n}章")) {
                self.status_message = format!("创建第 {target_n} 章失败：{e}");
                return;
            }
        }
        self.select_chapter(target_n);
        self.start_manual_chapter_generation(target_n);
    }

    /// 对齐 inkoswin `next_pending_chapter_number`：优先取第一个正文为空的章节，
    /// 若所有已登记章节都已写完，则返回 max(number)+1（当且仅当未超 `target_chapters`）。
    fn next_pending_chapter_number(&self) -> Option<i32> {
        let store = self.store.as_ref()?;
        let project = self.project.as_ref()?;
        let mut sorted: Vec<&crate::project::ChapterRecord> = project.chapters.iter().collect();
        sorted.sort_by_key(|c| c.number);
        for ch in &sorted {
            if let Ok((_t, body)) = store.load_chapter_content(ch.number) {
                if body.trim().is_empty() {
                    return Some(ch.number);
                }
            }
        }
        let max_no = sorted.last().map(|c| c.number).unwrap_or(0);
        let target = project.target_chapters.max(1);
        if max_no < target {
            Some(max_no + 1)
        } else {
            None
        }
    }

    fn cancel_manual_chapter_generation(&mut self) {
        if self.manual_gen_task.is_none() {
            return;
        }
        let n = self.manual_gen_target.unwrap_or(0);
        self.manual_gen_task = None;
        self.manual_gen_target = None;
        self.status_message = if n > 0 {
            format!("已取消第 {n} 章生成（已接收的内容未丢弃，可在编辑器继续修改）")
        } else {
            "已取消生成".into()
        };
        oplog::try_append(
            self.novel_path.as_deref(),
            "AI 生成本章 · 取消",
            &format!("第 {n} 章"),
        );
    }

    fn handle_manual_gen_done(&mut self) {
        let Some(task) = self.manual_gen_task.take() else { return };
        let target_n = self.manual_gen_target.take().unwrap_or(0);

        if let Some(err) = task.error.clone() {
            self.status_message = format!("✗ 生成失败：{err}");
            oplog::try_append(self.novel_path.as_deref(), "AI 生成本章 · 失败", &err);
            return;
        }
        let raw = task.accumulated.trim().to_string();
        if raw.is_empty() {
            self.status_message = "生成结果为空".into();
            return;
        }

        // 对齐 inkoswin：优先按「标题：/摘要：/正文：」解析，fallback 到 `#` 首行。
        let fallback_title = if !self.chapter_title.trim().is_empty() {
            self.chapter_title.trim().to_string()
        } else {
            format!("第{target_n}章")
        };
        let result = inkoswin_prompt::parse_generation_output(&raw, &fallback_title);

        self.chapter_title = result.title;
        self.chapter_body = result.content;
        // 摘要：若 LLM 给出，则写入摘要编辑框并标记为 dirty 供用户一并保存；
        // 否则保留原摘要（parse_generation_output 已在 fallback 场景自动生成 make_summary）。
        let summary_trim = result.summary.trim();
        if !summary_trim.is_empty()
            && summary_trim != self.chapter_summary.trim()
        {
            self.chapter_summary = summary_trim.to_string();
            self.chapter_summary_dirty = true;
        }
        self.chapter_dirty = true;
        self.preview_md = self.chapter_body.clone();
        let words = self.chapter_body.chars().filter(|c| !c.is_whitespace()).count();
        self.status_message = format!(
            "✓ 第 {target_n} 章已生成（{words} 字 · {:.1}s），请核对后保存",
            task.elapsed_secs()
        );
        oplog::try_append(
            self.novel_path.as_deref(),
            "AI 生成本章 · 完成",
            &format!("第 {target_n} 章 · {words} 字"),
        );
    }

    /// 「AI 生成 book_rules.md」（对齐 `Narcooo/inkos` `architect.bookRulesPrompt`）。
    ///
    /// - 使用写作 LLM 生成；
    /// - 流式落到 `state_doc_body`，用户随后再 💾 保存（不直接写盘，避免覆盖手工编辑）；
    /// - 若 `state_doc_body` 已被改动，在调用前应让 UI 先弹覆盖确认。
    fn start_book_rules_generation(&mut self) {
        self.pending_book_rules_confirm = false;
        if self.book_rules_gen_task.is_some() {
            self.status_message = "已有 book_rules 生成任务在进行".into();
            return;
        }
        let vendor_id = self.settings.writing_vendor.clone();
        if vendor_id.is_empty() {
            self.status_message = "未选择写作 LLM（设置 → 写作）".into();
            return;
        }
        let Some(cfg) = self.vendor_config(&vendor_id) else {
            self.status_message = format!("服务商「{vendor_id}」配置缺失");
            return;
        };
        if !cfg.is_configured() {
            self.status_message = "写作 LLM 服务商未完整配置".into();
            return;
        }
        let Some(project_ref) = self.project.as_ref() else {
            self.status_message = "未打开项目".into();
            return;
        };
        let project_snapshot = project_ref.clone();

        // 同人模式：若现有 book_rules.md 已包含 fanficMode，则沿用；否则不传。
        let fanfic_mode: Option<String> = {
            let parsed = crate::book_rules::parse_book_rules(&self.state_doc_body);
            parsed.rules.fanfic_mode
        };

        let memory_context = self.collect_book_rules_memory_context(1000);
        let (system_prompt, user_prompt) = crate::book_rules::build_generation_prompts(
            &project_snapshot,
            fanfic_mode.as_deref(),
            None,
            Some(&memory_context),
        );

        let messages = vec![
            ChatMessage::system(&system_prompt),
            ChatMessage::user(user_prompt),
        ];
        let model = cfg.model.clone();
        let streaming = self.settings.writing_streaming;
        self.book_rules_gen_task = Some(spawn_chat(cfg, vendor_id, model, messages, streaming));
        self.status_message = "✨ 正在生成 book_rules.md…".into();
        oplog::try_append(
            self.novel_path.as_deref(),
            "AI 生成 book_rules.md · 开始",
            if memory_context.trim().is_empty() {
                "长期记忆：未命中可用文件"
            } else {
                "长期记忆：story_state/novel_brief.md,current_state.md,pending_hooks.md + story/author_intent.md,current_focus.md"
            },
        );
    }

    fn cancel_book_rules_generation(&mut self) {
        if self.book_rules_gen_task.is_none() {
            return;
        }
        self.book_rules_gen_task = None;
        self.status_message = "已取消 book_rules 生成（已接收的内容保留在编辑器，未保存）".into();
        oplog::try_append(
            self.novel_path.as_deref(),
            "AI 生成 book_rules.md · 取消",
            "",
        );
    }

    fn handle_book_rules_gen_done(&mut self) {
        let Some(task) = self.book_rules_gen_task.take() else { return };
        if let Some(err) = task.error.clone() {
            self.status_message = format!("✗ book_rules 生成失败：{err}");
            oplog::try_append(self.novel_path.as_deref(), "AI 生成 book_rules.md · 失败", &err);
            return;
        }
        let raw = task.accumulated.trim().to_string();
        if raw.is_empty() {
            self.status_message = "book_rules 生成结果为空".into();
            return;
        }
        // 对齐 inkos `parseBookRules`：先剥代码围栏 / 前言，再尝试 YAML 解析；
        // 若 YAML 合法则**用 render_book_rules 重新格式化**一次，保证字段顺序与 schema 对齐。
        let cleaned = crate::book_rules::parse_generation_output(&raw);
        let parsed = crate::book_rules::parse_book_rules(&cleaned);
        let normalized = if parsed.rules.protagonist.is_some()
            || !parsed.rules.prohibitions.is_empty()
            || parsed.rules.genre_lock.is_some()
        {
            crate::book_rules::render_book_rules(&parsed)
        } else {
            // YAML 解析没拿到任何关键字段 —— 大概率 LLM 跑偏，原样保留让用户自己改。
            cleaned
        };
        self.state_doc_body = normalized;
        self.state_doc_dirty = true;
        // 自动切到 book_rules.md（若用户在生成期间切走，回来需要看到结果）
        self.selected_state_file = "book_rules.md".into();
        self.state_doc_loaded_for = Some("book_rules.md".into());
        self.status_message = format!(
            "✓ book_rules.md 已生成（{:.1}s），请核对后 💾 保存",
            task.elapsed_secs()
        );
        oplog::try_append(
            self.novel_path.as_deref(),
            "AI 生成 book_rules.md · 完成",
            "",
        );
    }

    /// 读取 story/ 控制层（author_intent / current_focus / book_rules 之外的硬约束提示）。
    fn collect_story_docs_for_prompt(&self, max_chars_each: usize) -> String {
        let Some(root) = self.novel_path.as_deref() else { return String::new() };
        let mut out = String::new();
        for f in ["story/author_intent.md", "story/current_focus.md"] {
            let path = root.join(f);
            let Ok(text) = fs::read_to_string(&path) else { continue };
            let trimmed = text.trim();
            if trimmed.is_empty() {
                continue;
            }
            let truncated = if trimmed.chars().count() > max_chars_each {
                let cut: String = trimmed.chars().take(max_chars_each).collect();
                format!("{cut}\n…（已截断）")
            } else {
                trimmed.to_string()
            };
            out.push_str(&format!("\n### {f}\n{truncated}\n"));
        }
        out
    }

    fn collect_book_rules_memory_context(&self, max_chars_each: usize) -> String {
        let Some(store) = self.store.as_ref() else {
            return String::new();
        };
        let Some(root) = self.novel_path.as_deref() else {
            return String::new();
        };
        let mut blocks: Vec<String> = Vec::new();
        for rel in [
            "story_state/novel_brief.md",
            "story_state/current_state.md",
            "story_state/pending_hooks.md",
            "story/author_intent.md",
            "story/current_focus.md",
        ] {
            let path = if let Some(name) = rel.strip_prefix("story_state/") {
                store.state_dir().join(name)
            } else {
                root.join(rel)
            };
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let trimmed = text.trim();
            if trimmed.is_empty() {
                continue;
            }
            let clipped = if trimmed.chars().count() > max_chars_each {
                let cut: String = trimmed.chars().take(max_chars_each).collect();
                format!("{cut}\n…（已截断）")
            } else {
                trimmed.to_string()
            };
            blocks.push(format!("[{rel}]\n{clipped}"));
        }
        blocks.join("\n\n")
    }

    /// 读取 target_n 之前最近一次审计结果（审计记录/log.jsonl 或 history 审计产物），作为参考。
    fn load_last_audit_block(&self, target_n: i32) -> String {
        if target_n <= 1 {
            return "（首章，无上一章审计）".to_string();
        }
        let prev = target_n - 1;
        let Some(root) = self.novel_path.as_deref() else { return "（无）".into() };
        if let Some(text) = history::latest_audit_text(root, prev) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                let limit = 2000;
                if trimmed.chars().count() > limit {
                    let cut: String = trimmed.chars().take(limit).collect();
                    return format!("{cut}\n…（已截断）");
                }
                return trimmed.to_string();
            }
        }
        "（暂无上一章审计记录）".into()
    }

    fn handle_auto_gen_done(&mut self) {
        let Some(task) = self.auto_gen_task.take() else { return };
        let phase = std::mem::take(&mut self.auto_gen_phase);

        if let Some(err) = task.error.clone() {
            self.auto_gen_log
                .push(format!("{}  请求失败：{err}", short_time()));
            oplog::try_append(self.novel_path.as_deref(), "定时写作 · 失败", &err);
            return;
        }
        let raw = task.accumulated.trim().to_string();

        match phase {
            AutoGenPhase::Idle => {}
            AutoGenPhase::AuditingPrev { prev_n, next_n } => {
                self.auto_gen_audit_text = raw.clone();
                if let Some(ref root) = self.novel_path {
                    if let Ok(p) = history::save_audit_artifact(root, prev_n, &raw) {
                        self.auto_gen_log.push(format!(
                            "{}  审计完成 → {}",
                            short_time(),
                            p.file_name().and_then(|s| s.to_str()).unwrap_or("audit.md")
                        ));
                    }
                }
                oplog::try_append(
                    self.novel_path.as_deref(),
                    "定时写作 · 审计完成",
                    &format!("第 {prev_n} 章 · {} 字 · {:.1}s", raw.chars().count(), task.elapsed_secs()),
                );
                self.start_auto_gen_writing_phase(next_n);
            }
            AutoGenPhase::Writing { next_n: n } => {
                if raw.is_empty() {
                    self.auto_gen_log
                        .push(format!("{}  第 {n} 章空响应", short_time()));
                    return;
                }
                let (title, body) = parse_chapter_markdown(&raw);
                let title = if title.is_empty() { format!("第{n}章") } else { title };
                let novel_root = self.novel_path.clone();
                let (Some(store), Some(project)) = (&self.store, &mut self.project) else { return };
                match store.save_chapter(project, n, &title, &body, "generated", "") {
                    Ok(()) => {
                        project.auto_generate.last_run_at = crate::project::now_iso();
                        let _ = store.save_project(project);
                        let words = body.chars().filter(|c| !c.is_whitespace()).count();
                        self.auto_gen_log.push(format!(
                            "{}  第 {n} 章已生成（{words} 字 · {:.1}s）",
                            short_time(),
                            task.elapsed_secs()
                        ));
                        oplog::try_append(
                            novel_root.as_deref(),
                            "定时写作 · 完成",
                            &format!("第 {n} 章 · {words} 字"),
                        );
                        let novel_title = project.title.clone();
                        let evt = crate::notify::NotifyEvent {
                            kind: "chapter_done".into(),
                            title: format!("第 {n} 章已生成"),
                            body: format!(
                                "{} · {} 字 · 耗时 {:.1}s",
                                title,
                                words,
                                task.elapsed_secs()
                            ),
                            novel: novel_title,
                            chapter_no: Some(n),
                            timestamp: crate::project::now_iso(),
                        };
                        let _ = crate::notify::notify_all(&self.settings.notify, &evt);
                    }
                    Err(e) => {
                        self.auto_gen_log
                            .push(format!("{}  保存第 {n} 章失败：{e}", short_time()));
                        oplog::try_append(
                            novel_root.as_deref(),
                            "定时写作 · 保存失败",
                            &format!("第 {n} 章 · {e}"),
                        );
                        let evt = crate::notify::NotifyEvent {
                            kind: "error".into(),
                            title: "定时写作保存失败".into(),
                            body: format!("第 {n} 章 · {e}"),
                            novel: project.title.clone(),
                            chapter_no: Some(n),
                            timestamp: crate::project::now_iso(),
                        };
                        let _ = crate::notify::notify_all(&self.settings.notify, &evt);
                    }
                }
                self.auto_gen_audit_text.clear();
            }
        }
    }

    // ---------- 小说设定 ----------
    fn ui_novel_meta(&mut self, ui: &mut egui::Ui) {
        if self.project.is_none() {
            theme::empty_state(ui, "尚未打开项目", "请先在「项目」中选择小说目录");
            return;
        }

        ui.horizontal(|ui| {
            for (tab, label) in [
                (NovelMetaTab::Basics, "基础信息"),
                (NovelMetaTab::StateDocs, "状态档案"),
            ] {
                let selected = self.nm_tab == tab;
                let bg = if selected { color::ACCENT_DIM } else { color::SURFACE };
                let fg = if selected { color::TEXT } else { color::TEXT_DIM };
                let resp = egui::Frame::default()
                    .fill(bg)
                    .stroke(Stroke::new(1.0, color::BORDER))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(14, 7))
                    .show(ui, |ui| {
                        ui.label(RichText::new(label).color(fg).size(13.0));
                    })
                    .response
                    .interact(egui::Sense::click());
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if resp.clicked() {
                    self.nm_tab = tab;
                }
            }
        });

        ui.add_space(10.0);
        match self.nm_tab {
            NovelMetaTab::Basics => self.ui_meta_basics(ui),
            NovelMetaTab::StateDocs => self.ui_meta_state_docs(ui),
        }
    }

    fn ui_meta_basics(&mut self, ui: &mut egui::Ui) {
        let mut save_clicked = false;
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            let project = self.project.as_mut().unwrap();
            theme::card_frame().show(ui, |ui| {
                ui.label(RichText::new("书名").size(11.5).color(color::TEXT_DIM));
                ui.add(TextEdit::singleline(&mut project.title).desired_width(f32::INFINITY));
                ui.add_space(6.0);
                ui.label(RichText::new("题材").size(11.5).color(color::TEXT_DIM));
                let cur_genre = project.genre.clone();
                ComboBox::from_id_salt("genre_pick")
                    .width(240.0)
                    .selected_text(if cur_genre.is_empty() {
                        "（未选择）".to_string()
                    } else {
                        cur_genre.clone()
                    })
                    .show_ui(ui, |ui| {
                        for g in GENRES {
                            if ui.selectable_label(cur_genre == *g, *g).clicked() {
                                project.genre = (*g).into();
                            }
                        }
                    });
                if project.genre == "其他" {
                    ui.add_space(4.0);
                    ui.label(RichText::new("自定义题材").size(11.5).color(color::TEXT_DIM));
                    ui.add(
                        TextEdit::singleline(&mut project.genre)
                            .hint_text("填写自定义题材后会替换上方选择")
                            .desired_width(f32::INFINITY),
                    );
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("目标章节数").size(11.5).color(color::TEXT_DIM));
                    ui.add(
                        egui::DragValue::new(&mut project.target_chapters)
                            .speed(1.0)
                            .range(1..=50_000),
                    );
                    ui.add_space(20.0);
                    ui.label(RichText::new("每章目标字数").size(11.5).color(color::TEXT_DIM));
                    ui.add(
                        egui::DragValue::new(&mut project.chapter_word_goal)
                            .speed(50.0)
                            .range(100..=200_000),
                    );
                });
            });

            ui.add_space(10.0);
            theme::card_frame().show(ui, |ui| {
                ui.label(
                    RichText::new("提示").size(13.0).color(color::TEXT_DIM).strong(),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "「故事核心 / 人物 / 世界 / 文风 / 大纲」等长期设定，请到\
                         「状态档案」标签页中编辑 novel_brief.md。",
                    )
                    .color(color::TEXT_DIM)
                    .size(12.5),
                );
            });

            ui.add_space(12.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("💾  保存小说资料").clicked() {
                    save_clicked = true;
                }
            });
        });
        if save_clicked {
            self.save_novel_meta();
        }
    }

    fn ui_meta_state_docs(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("story_state/ · 长期记忆 9 件套（兼容 inkoswin）")
                .color(color::TEXT_FAINT)
                .size(11.5),
        );
        ui.horizontal_wrapped(|ui| {
            for f in STATE_FILES {
                let selected = self.selected_state_file == *f;
                let bg = if selected { color::SURFACE_HI } else { color::SURFACE };
                let fg = if selected { color::TEXT } else { color::TEXT_DIM };
                let resp = egui::Frame::default()
                    .fill(bg)
                    .stroke(Stroke::new(1.0, color::BORDER))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(10, 5))
                    .show(ui, |ui| {
                        ui.label(RichText::new(*f).color(fg).size(12.0));
                    })
                    .response
                    .interact(egui::Sense::click());
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if resp.clicked() && !selected {
                    self.selected_state_file = (*f).into();
                    self.load_state_doc();
                }
            }
        });
        ui.add_space(6.0);
        ui.label(
            RichText::new("story/ · 致敬 Narcooo/inkos 的 Input Governance / Style / Fanfic")
                .color(color::TEXT_FAINT)
                .size(11.5),
        );
        ui.horizontal_wrapped(|ui| {
            for (path, label) in STORY_FILES {
                let selected = self.selected_state_file == *path;
                let bg = if selected { color::SURFACE_HI } else { color::SURFACE };
                let fg = if selected { color::TEXT } else { color::TEXT_DIM };
                let resp = egui::Frame::default()
                    .fill(bg)
                    .stroke(Stroke::new(1.0, color::BORDER))
                    .corner_radius(CornerRadius::same(6))
                    .inner_margin(Margin::symmetric(10, 5))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(format!("{label}  ·  {path}"))
                                .color(fg)
                                .size(12.0),
                        );
                    })
                    .response
                    .interact(egui::Sense::click());
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if resp.clicked() && !selected {
                    self.selected_state_file = (*path).into();
                    self.load_state_doc();
                }
            }
        });

        if self.state_doc_loaded_for.as_deref() != Some(self.selected_state_file.as_str()) {
            self.load_state_doc();
        }

        ui.add_space(8.0);
        theme::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("当前文件：{}", self.selected_state_file))
                        .color(color::TEXT_DIM)
                        .size(12.0),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("💾  保存").clicked() {
                        self.save_state_doc();
                    }
                    if ui.button("⟳  重新载入").clicked() {
                        self.load_state_doc();
                    }
                    // 对齐 inkoswin：AI 刷新当前 / 全部 story_state 档案（结果直接落盘；章节仍手动保存）。
                    let state_refresh_busy =
                        self.state_refresh_task.is_some() || self.state_refresh_batch.is_some();
                    if state_refresh_busy {
                        if ui
                            .button("⏹  取消刷新")
                            .on_hover_text("中断当前批次，已落盘的文件不会回滚")
                            .clicked()
                        {
                            self.cancel_state_refresh();
                        }
                        theme::pill(ui, "刷新档案中", Color32::WHITE, color::ACCENT);
                    } else {
                        let can_current = crate::state_refresh::is_ai_refreshable(
                            &self.selected_state_file,
                        ) && !self.selected_state_file.starts_with("story/");
                        if ui
                            .add_enabled(
                                can_current,
                                egui::Button::new("🔄  AI 刷新当前"),
                            )
                            .on_hover_text(
                                "对齐 inkoswin「AI 刷新当前文件」：\
                                 用写作 LLM 根据章节 digest + 其他档案摘要重写当前文件并保存。\n\
                                 chapter_summaries.md 为本地重建不调 LLM；book_rules / story/ 下文件请手动维护。",
                            )
                            .clicked()
                        {
                            self.start_state_refresh_current();
                        }
                        if ui
                            .button("🔄  AI 刷新全部")
                            .on_hover_text(
                                "对齐 inkoswin「AI 刷新全部文件」：\
                                 按固定顺序刷新 8 个档案（不含 book_rules）。\
                                 chapter_summaries 仅本地汇总；其余逐个调用 LLM。\
                                 请先保存当前章节与未保存的状态档案编辑。",
                            )
                            .clicked()
                        {
                            self.start_state_refresh_all();
                        }
                    }
                    // book_rules.md 专属：AI 生成 / 取消 按钮，对齐 Narcooo/inkos
                    // architect.bookRulesPrompt 的硬约束生成逻辑。
                    if self.selected_state_file == "book_rules.md" {
                        if self.book_rules_gen_task.is_some() {
                            if ui
                                .button("⏹  取消生成")
                                .on_hover_text("取消当前 AI 生成，已写入编辑器的内容保留")
                                .clicked()
                            {
                                self.cancel_book_rules_generation();
                            }
                            theme::pill(ui, "生成中", Color32::WHITE, color::ACCENT);
                        } else {
                            let btn = ui
                                .button("🪄  AI 生成")
                                .on_hover_text(
                                    "对齐 Narcooo/inkos `architect.bookRulesPrompt`：\n\
                                     - 输入：当前小说设定（题材/主角/世界观/大纲/文风/额外指引）\n\
                                     - 输出：YAML frontmatter（version/protagonist/genreLock/\n\
                                       prohibitions/chapterTypesOverride/fatigueWordsOverride/\n\
                                       additionalAuditDimensions/enableFullCastTracking）+ 叙事指导\n\
                                     - 修真/玄幻等题材会自动追加 numericalSystemOverrides\n\
                                     - 如果当前 frontmatter 含 fanficMode 会被沿用\n\
                                     - 结果只填入编辑器，需手动 💾 保存",
                                );
                            if btn.clicked() {
                                let body_trim = self.state_doc_body.trim();
                                let parsed = crate::book_rules::parse_book_rules(body_trim);
                                let has_real_content = parsed.rules.protagonist.is_some()
                                    || parsed.rules.genre_lock.is_some()
                                    || !parsed.rules.prohibitions.is_empty()
                                    || parsed.body.trim_start_matches('#').trim().len() > 40;
                                if has_real_content || self.state_doc_dirty {
                                    self.pending_book_rules_confirm = true;
                                } else {
                                    self.start_book_rules_generation();
                                }
                            }
                        }
                    }
                    if self.state_doc_dirty {
                        theme::pill(ui, "未保存", Color32::WHITE, color::WARNING);
                    }
                });
            });
        });

        ui.add_space(8.0);
        let avail_h = ui.available_height();
        ui.allocate_ui(egui::vec2(ui.available_width(), avail_h), |ui| {
            ui.columns(2, |cols| {
                cols[0].vertical(|ui| {
                    ui.label(RichText::new("内容（Markdown）").size(11.5).color(color::TEXT_DIM));
                    ui.add_space(4.0);
                    egui::Frame::default()
                        .fill(color::SURFACE)
                        .stroke(Stroke::new(1.0, color::BORDER))
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::same(8))
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("state_body_scroll")
                                .auto_shrink([false; 2])
                                .show(ui, |ui| {
                                    let r = ui.add(
                                        TextEdit::multiline(&mut self.state_doc_body)
                                            .desired_width(f32::INFINITY)
                                            .desired_rows(40)
                                            .id_salt("state_body"),
                                    );
                                    if r.changed() {
                                        self.state_doc_dirty = true;
                                    }
                                });
                        });
                });
                cols[1].vertical(|ui| {
                    ui.label(RichText::new("预览").size(11.5).color(color::TEXT_DIM));
                    ui.add_space(4.0);
                    egui::Frame::default()
                        .fill(color::SURFACE)
                        .stroke(Stroke::new(1.0, color::BORDER))
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::same(8))
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical()
                                .id_salt("state_preview_scroll")
                                .auto_shrink([false; 2])
                                .show(ui, |ui| {
                                    CommonMarkViewer::new()
                                        .show(ui, &mut self.cm_cache, &self.state_doc_body);
                                });
                        });
                });
            });
        });
    }

    // ---------- 工具箱 ----------
    fn ui_tools(&mut self, ui: &mut egui::Ui) {
        if self.project.is_none() {
            theme::empty_state(ui, "尚未打开项目", "请先在「项目」中选择小说目录");
            return;
        }
        ui.horizontal_wrapped(|ui| {
            for tab in [
                ToolsTab::Search,
                ToolsTab::Import,
                ToolsTab::Export,
                ToolsTab::Rename,
                ToolsTab::Pipeline,
                ToolsTab::Style,
                ToolsTab::Aigc,
                ToolsTab::Analytics,
                ToolsTab::Fanfic,
            ] {
                let selected = self.tools_tab == tab;
                let bg = if selected { color::ACCENT_DIM } else { color::SURFACE };
                let fg = if selected { color::TEXT } else { color::TEXT_DIM };
                let resp = egui::Frame::default()
                    .fill(bg)
                    .stroke(Stroke::new(1.0, color::BORDER))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(12, 6))
                    .show(ui, |ui| {
                        ui.label(RichText::new(tab.label()).color(fg).size(12.5));
                    })
                    .response
                    .interact(egui::Sense::click());
                if resp.hovered() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                if resp.clicked() {
                    self.tools_tab = tab;
                }
            }
        });
        ui.add_space(10.0);
        egui::ScrollArea::vertical()
            .id_salt("tools_scroll")
            .auto_shrink([false; 2])
            .show(ui, |ui| match self.tools_tab {
                ToolsTab::Search => self.ui_tools_search(ui),
                ToolsTab::Import => self.ui_tools_import(ui),
                ToolsTab::Export => self.ui_tools_export(ui),
                ToolsTab::Rename => self.ui_tools_rename(ui),
                ToolsTab::Pipeline => self.ui_tools_pipeline(ui),
                ToolsTab::Style => self.ui_tools_style(ui),
                ToolsTab::Aigc => self.ui_tools_aigc(ui),
                ToolsTab::Analytics => self.ui_tools_analytics(ui),
                ToolsTab::Fanfic => self.ui_tools_fanfic(ui),
            });
    }

    fn ui_tools_search(&mut self, ui: &mut egui::Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.label(RichText::new("全书搜索").size(14.0).strong());
            dim_label(ui, "覆盖 chapters/ + story_state/ + story/。");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add(
                    TextEdit::singleline(&mut self.search_query)
                        .hint_text("输入要搜索的关键词")
                        .desired_width(360.0),
                );
                ui.checkbox(&mut self.search_case_insensitive, "忽略大小写");
                if ui.button("🔍  搜索").clicked() {
                    if let Some(ref store) = self.store {
                        let opts = crate::search::SearchOptions {
                            query: self.search_query.clone(),
                            case_insensitive: self.search_case_insensitive,
                            max_hits: 800,
                        };
                        self.search_results = crate::search::search(store, &opts);
                        self.search_msg = format!("命中 {} 条", self.search_results.len());
                    }
                }
                if ui.button("清空").clicked() {
                    self.search_results.clear();
                    self.search_msg.clear();
                }
            });
            if !self.search_msg.is_empty() {
                dim_label(ui, &self.search_msg);
            }
        });

        ui.add_space(8.0);
        if self.search_results.is_empty() {
            theme::empty_state(ui, "暂无结果", "尝试换关键词或勾选「忽略大小写」。");
            return;
        }
        let results: Vec<_> = self.search_results.clone();
        let mut want_open: Option<i32> = None;
        for hit in &results {
            theme::card_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    theme::pill(ui, hit.source.label(), Color32::WHITE, color::ACCENT_DIM);
                    ui.label(RichText::new(&hit.label).color(color::TEXT).size(12.5).strong());
                    ui.label(
                        RichText::new(format!("行 {}", hit.line_no))
                            .color(color::TEXT_FAINT)
                            .size(11.0),
                    );
                    if let Some(n) = hit.chapter_no {
                        if ui.button("打开章节").clicked() {
                            want_open = Some(n);
                        }
                    }
                });
                if let Some(ref b) = hit.context_before {
                    ui.label(
                        RichText::new(b)
                            .color(color::TEXT_FAINT)
                            .size(11.5)
                            .monospace(),
                    );
                }
                ui.label(RichText::new(&hit.line).color(color::ACCENT_HI).size(12.5).monospace());
                if let Some(ref a) = hit.context_after {
                    ui.label(
                        RichText::new(a)
                            .color(color::TEXT_FAINT)
                            .size(11.5)
                            .monospace(),
                    );
                }
            });
            ui.add_space(4.0);
        }
        if let Some(n) = want_open {
            self.section = Section::Writing;
            self.select_chapter(n);
        }
    }

    fn ui_tools_import(&mut self, ui: &mut egui::Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.label(RichText::new("从单个文本导入章节").size(14.0).strong());
            dim_label(
                ui,
                "默认按「第X章」拆分；已存在文件默认跳过；可勾选「覆盖」强制写入。",
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("📄  选择 .txt / .md 文件").clicked() {
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("Text", &["txt", "md", "markdown"])
                        .pick_file()
                    {
                        match std::fs::read_to_string(&p) {
                            Ok(t) => {
                                self.import_text = t;
                                self.import_source = p.display().to_string();
                                self.import_msg = format!("已读取 {}", self.import_source);
                            }
                            Err(e) => self.import_msg = format!("读取失败：{e}"),
                        }
                    }
                }
                if !self.import_source.is_empty() {
                    ui.label(
                        RichText::new(&self.import_source)
                            .color(color::TEXT_DIM)
                            .size(11.5),
                    );
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("起始章节号");
                ui.add(egui::DragValue::new(&mut self.import_start_no).range(1..=50_000));
                ui.checkbox(&mut self.import_overwrite, "覆盖已存在");
            });
            ui.label(RichText::new("分章正则").color(color::TEXT_DIM).size(11.5));
            ui.add(
                TextEdit::singleline(&mut self.import_regex)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace),
            );
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("🚀  开始导入").clicked() {
                    let plan = crate::import_chapters::ImportPlan {
                        source_label: self.import_source.clone(),
                        split_regex: self.import_regex.clone(),
                        starting_number: self.import_start_no,
                        overwrite: self.import_overwrite,
                    };
                    let raw = self.import_text.clone();
                    if let (Some(store), Some(project)) = (&self.store, self.project.as_mut()) {
                        match crate::import_chapters::import_text(store, project, &raw, &plan) {
                            Ok(report) => {
                                self.import_msg = format!(
                                    "完成：新建 {} · 跳过 {} · 覆盖 {} · 空 {}",
                                    report.created,
                                    report.skipped,
                                    report.overwritten,
                                    report.empty
                                );
                                if let Ok(p) = store.load_project() {
                                    self.project = Some(p);
                                }
                                oplog::try_append(
                                    self.novel_path.as_deref(),
                                    "导入章节",
                                    &self.import_msg,
                                );
                                self.import_report = Some(report);
                            }
                            Err(e) => self.import_msg = format!("导入失败：{e}"),
                        }
                    }
                }
                if ui.button("仅预览拆分").clicked() {
                    match crate::import_chapters::split_text(&self.import_text, &self.import_regex)
                    {
                        Ok(segs) => self.import_msg = format!("预览：拆出 {} 段", segs.len()),
                        Err(e) => self.import_msg = format!("正则错误：{e}"),
                    }
                }
            });
            if !self.import_msg.is_empty() {
                ui.add_space(4.0);
                dim_label(ui, &self.import_msg);
            }
            if let Some(ref r) = self.import_report {
                ui.add_space(8.0);
                ui.label(RichText::new("最近导入").color(color::TEXT_DIM).size(11.5));
                egui::ScrollArea::vertical()
                    .id_salt("import_report_scroll")
                    .max_height(280.0)
                    .show(ui, |ui| {
                        for it in &r.items {
                            ui.label(
                                RichText::new(format!(
                                    "第 {} 章 · {} · {}",
                                    it.number,
                                    it.status.label(),
                                    it.title
                                ))
                                .color(color::TEXT)
                                .size(12.0),
                            );
                        }
                    });
            }
        });
    }

    fn ui_tools_export(&mut self, ui: &mut egui::Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.label(RichText::new("全书导出").size(14.0).strong());
            dim_label(ui, "导出到 <小说根目录>/导出/<书名>-<时间>.<ext>。");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                for fmt in [
                    crate::export::ExportFormat::Txt,
                    crate::export::ExportFormat::Markdown,
                    crate::export::ExportFormat::Epub,
                ] {
                    let on = self.export_format == fmt;
                    if ui.selectable_label(on, fmt.label()).clicked() {
                        self.export_format = fmt;
                    }
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("📦  开始导出").clicked() {
                    if let (Some(store), Some(project)) = (&self.store, &self.project) {
                        match crate::export::export_book(store, project, self.export_format) {
                            Ok(p) => {
                                self.export_msg = format!("已导出：{}", p.display());
                                self.export_last_path = Some(p.clone());
                                oplog::try_append(
                                    self.novel_path.as_deref(),
                                    "导出全书",
                                    &p.display().to_string(),
                                );
                            }
                            Err(e) => self.export_msg = format!("导出失败：{e}"),
                        }
                    }
                }
                if let Some(ref p) = self.export_last_path {
                    if ui.button("📂  打开导出目录").clicked() {
                        if let Some(parent) = p.parent() {
                            let _ = open_in_explorer(parent);
                        }
                    }
                }
            });
            if !self.export_msg.is_empty() {
                ui.add_space(4.0);
                dim_label(ui, &self.export_msg);
            }
        });
    }

    fn ui_tools_rename(&mut self, ui: &mut egui::Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.label(RichText::new("全书实体改名").size(14.0).strong());
            dim_label(
                ui,
                "扫描 chapters/ + story_state/ + story/，逐字面量替换；改名前自动备份。",
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("旧名");
                ui.add(TextEdit::singleline(&mut self.rename_from).desired_width(180.0));
                ui.label("新名");
                ui.add(TextEdit::singleline(&mut self.rename_to).desired_width(180.0));
                if ui.button("✏️  执行改名").clicked() {
                    if let (Some(store), Some(project)) = (&self.store, self.project.as_mut()) {
                        match crate::rename::rename_entity(
                            store,
                            project,
                            &self.rename_from,
                            &self.rename_to,
                        ) {
                            Ok(report) => {
                                let total: usize = report.items.iter().map(|i| i.occurrences).sum();
                                self.rename_msg = format!(
                                    "完成：影响 {} 个文件 · {} 处替换",
                                    report.items.len(),
                                    total
                                );
                                oplog::try_append(
                                    self.novel_path.as_deref(),
                                    "全书改名",
                                    &format!(
                                        "`{}` → `{}` · {} 处",
                                        report.from, report.to, total
                                    ),
                                );
                                self.rename_report = Some(report);
                            }
                            Err(e) => self.rename_msg = format!("失败：{e}"),
                        }
                    }
                }
            });
            if !self.rename_msg.is_empty() {
                ui.add_space(4.0);
                dim_label(ui, &self.rename_msg);
            }
            if let Some(ref r) = self.rename_report {
                ui.add_space(8.0);
                if let Some(ref bk) = r.backup_dir {
                    ui.label(
                        RichText::new(format!("备份位置：{}", bk.display()))
                            .color(color::TEXT_DIM)
                            .size(11.5),
                    );
                }
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt("rename_report_scroll")
                    .max_height(280.0)
                    .show(ui, |ui| {
                        for it in &r.items {
                            ui.label(
                                RichText::new(format!(
                                    "[{}] {} · {} 处",
                                    it.kind.label(),
                                    it.label,
                                    it.occurrences
                                ))
                                .color(color::TEXT)
                                .size(12.0),
                            );
                        }
                    });
            }
        });
    }

    fn ui_tools_pipeline(&mut self, ui: &mut egui::Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.label(RichText::new("写作管线 · 五段式 prompt 生成").size(14.0).strong());
            dim_label(
                ui,
                "Plan → Compose → Draft → Audit → Revise；prompt 含 governance / 33 维 / 字数治理。",
            );
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                for stage in crate::pipeline::PipelineStage::all() {
                    let on = self.pipeline_stage == *stage;
                    if ui.selectable_label(on, stage.label()).clicked() {
                        self.pipeline_stage = *stage;
                    }
                }
            });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("章节号");
                ui.add(egui::DragValue::new(&mut self.pipeline_chapter_no).range(1..=50_000));
                if ui.button("⚙  生成 prompt").clicked() {
                    self.regen_pipeline_prompt();
                }
                if ui.button("📋  复制").clicked() {
                    ui.ctx().copy_text(self.pipeline_prompt.clone());
                    self.pipeline_msg = "已复制到剪贴板".into();
                }
                if ui.button("→  发送到写作助手").clicked() {
                    self.assistant_input = self.pipeline_prompt.clone();
                    self.section = Section::Assistant;
                    self.pipeline_msg = "已转送到写作助手输入框".into();
                }
            });
            if !self.pipeline_msg.is_empty() {
                dim_label(ui, &self.pipeline_msg);
            }
        });
        ui.add_space(8.0);
        theme::card_frame().show(ui, |ui| {
            ui.label(RichText::new("当前阶段 prompt").size(13.0).strong());
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .id_salt("pipeline_prompt_scroll")
                .max_height(380.0)
                .show(ui, |ui| {
                    ui.add(
                        TextEdit::multiline(&mut self.pipeline_prompt)
                            .desired_width(f32::INFINITY)
                            .desired_rows(20)
                            .font(egui::TextStyle::Monospace),
                    );
                });
        });
        ui.add_space(8.0);
        theme::card_frame().show(ui, |ui| {
            let n = self.pipeline_chapter_no;
            ui.label(RichText::new(format!("第 {n} 章 runtime 工件")).size(13.0).strong());
            if let Some(ref root) = self.novel_path {
                let intent = crate::runtime::intent_path(root, n);
                let ctx = crate::runtime::context_path(root, n);
                let rs = crate::runtime::rule_stack_path(root, n);
                let trace = crate::runtime::trace_path(root, n);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{}",
                            intent.strip_prefix(root).unwrap_or(&intent).display()
                        ))
                        .color(if intent.exists() {
                            color::TEXT
                        } else {
                            color::TEXT_FAINT
                        })
                        .size(11.5),
                    );
                });
                ui.label(
                    RichText::new(format!(
                        "{}",
                        ctx.strip_prefix(root).unwrap_or(&ctx).display()
                    ))
                    .color(if ctx.exists() { color::TEXT } else { color::TEXT_FAINT })
                    .size(11.5),
                );
                ui.label(
                    RichText::new(format!(
                        "{}",
                        rs.strip_prefix(root).unwrap_or(&rs).display()
                    ))
                    .color(if rs.exists() { color::TEXT } else { color::TEXT_FAINT })
                    .size(11.5),
                );
                ui.label(
                    RichText::new(format!(
                        "{}",
                        trace.strip_prefix(root).unwrap_or(&trace).display()
                    ))
                    .color(if trace.exists() { color::TEXT } else { color::TEXT_FAINT })
                    .size(11.5),
                );
                ui.add_space(4.0);
                if ui.button("📂  打开 runtime 目录").clicked() {
                    let _ = open_in_explorer(&crate::runtime::runtime_dir(root));
                }
            }
        });
    }

    fn regen_pipeline_prompt(&mut self) {
        let (Some(store), Some(project)) = (&self.store, &self.project) else { return };
        let n = self.pipeline_chapter_no;
        let title = project
            .chapters
            .iter()
            .find(|c| c.number == n)
            .map(|c| c.title.clone())
            .unwrap_or_else(|| format!("第{n}章"));
        let goal = project.chapter_word_goal.max(0);
        let tol = self.settings.effective_word_tolerance();
        let intent_md = self
            .novel_path
            .as_ref()
            .map(|r| crate::runtime::read_intent(r, n))
            .unwrap_or_default();
        let body = store
            .load_chapter_content(n)
            .ok()
            .map(|(_, b)| b)
            .unwrap_or_default();
        self.pipeline_prompt = match self.pipeline_stage {
            crate::pipeline::PipelineStage::Plan => {
                crate::pipeline::build_plan_prompt(store, project, n, &title)
            }
            crate::pipeline::PipelineStage::Compose => {
                crate::pipeline::build_compose_prompt(store, project, n, &intent_md)
            }
            crate::pipeline::PipelineStage::Draft => crate::pipeline::build_draft_prompt(
                store, project, n, &title, &intent_md, "（请粘贴上一步 Compose 输出）", goal, tol,
            ),
            crate::pipeline::PipelineStage::Audit => {
                crate::pipeline::build_audit_prompt(store, project, n, &body)
            }
            crate::pipeline::PipelineStage::Revise => crate::pipeline::build_revise_prompt(
                store,
                project,
                n,
                &body,
                "（请粘贴上一步 Audit 报告）",
                goal,
                tol,
            ),
        };
        // Persist context + rule stack snapshot
        if let Some(ref root) = self.novel_path {
            let route = self
                .settings
                .agent_routing
                .route_for(self.pipeline_stage.slug())
                .cloned()
                .unwrap_or_default();
            let ctx = crate::pipeline::make_context(
                project,
                if route.vendor.is_empty() {
                    &self.settings.writing_vendor
                } else {
                    &route.vendor
                },
                if route.model_override.is_empty() {
                    &self.novel_llm.model
                } else {
                    &route.model_override
                },
                &self.novel_llm.temperature,
                &self.novel_llm.max_tokens,
                goal,
                tol,
                STATE_FILES.iter().map(|s| s.to_string()).collect(),
                crate::intent::read_author_intent(root).chars().count(),
                crate::intent::read_current_focus(root).chars().count(),
                crate::style::load(root).is_some(),
            );
            let _ = crate::runtime::write_context(root, n, &ctx);
            let rs = crate::pipeline::collect_rule_stack(root, project, goal, tol);
            let _ = crate::runtime::write_rule_stack(root, n, &rs);
            if matches!(self.pipeline_stage, crate::pipeline::PipelineStage::Plan) {
                let _ = crate::runtime::write_intent(root, n, &self.pipeline_prompt);
            }
        }
        self.pipeline_msg = "已生成 prompt（已写入 story/runtime/）".into();
    }

    fn ui_tools_style(&mut self, ui: &mut egui::Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.label(RichText::new("文风指纹").size(14.0).strong());
            dim_label(
                ui,
                "粘贴一段或一章原作，自动分析平均句长、对白比例、高频二元短语等，落地到 story/style_fingerprint.md。",
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("样本来源标签");
                ui.add(TextEdit::singleline(&mut self.style_source_label).desired_width(240.0));
                if ui.button("📄  从文件读入").clicked() {
                    if let Some(p) = rfd::FileDialog::new().pick_file() {
                        match std::fs::read_to_string(&p) {
                            Ok(t) => {
                                self.style_input = t;
                                if self.style_source_label.is_empty() {
                                    self.style_source_label = p.display().to_string();
                                }
                                self.style_msg = "已读入样本".into();
                            }
                            Err(e) => self.style_msg = format!("读取失败：{e}"),
                        }
                    }
                }
                if ui.button("从当前章节抓取").clicked() {
                    self.style_input = self.chapter_body.clone();
                    if self.style_source_label.is_empty() {
                        self.style_source_label = format!(
                            "本书第 {} 章",
                            self.selected_chapter.unwrap_or(0)
                        );
                    }
                }
            });
            ui.add_space(4.0);
            egui::ScrollArea::vertical()
                .id_salt("style_input_scroll")
                .max_height(220.0)
                .show(ui, |ui| {
                    ui.add(
                        TextEdit::multiline(&mut self.style_input)
                            .desired_width(f32::INFINITY)
                            .desired_rows(10)
                            .hint_text("在此粘贴待分析的中文样本……"),
                    );
                });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("🧪  分析并保存").clicked() {
                    let label = if self.style_source_label.trim().is_empty() {
                        "未命名样本".to_string()
                    } else {
                        self.style_source_label.trim().to_string()
                    };
                    let fp = crate::style::analyze(&self.style_input, &label);
                    if let Some(ref root) = self.novel_path {
                        match crate::style::save(root, &fp) {
                            Ok(()) => {
                                self.style_msg = format!(
                                    "已保存到 {}",
                                    crate::style::fingerprint_path(root).display()
                                );
                            }
                            Err(e) => self.style_msg = format!("保存失败：{e}"),
                        }
                    }
                    self.style_fp = Some(fp);
                }
                if ui.button("仅分析（不保存）").clicked() {
                    let label = if self.style_source_label.trim().is_empty() {
                        "未命名样本".to_string()
                    } else {
                        self.style_source_label.trim().to_string()
                    };
                    self.style_fp = Some(crate::style::analyze(&self.style_input, &label));
                    self.style_msg = "已分析（未保存）".into();
                }
            });
            if !self.style_msg.is_empty() {
                dim_label(ui, &self.style_msg);
            }
        });

        if let Some(fp) = self.style_fp.clone() {
            ui.add_space(8.0);
            theme::card_frame().show(ui, |ui| {
                ui.label(RichText::new("分析结果").size(13.0).strong());
                ui.label(
                    RichText::new(format!(
                        "样本：{} · 总字数 {} · 句子 {} · 平均句长 {:.1}",
                        fp.source_label, fp.total_chars, fp.total_sentences, fp.avg_sentence_chars
                    ))
                    .color(color::TEXT_DIM)
                    .size(11.5),
                );
                ui.label(
                    RichText::new(format!(
                        "长句 {:.1}% · 短句 {:.1}% · 对白行 {:.1}%",
                        fp.long_sentence_ratio * 100.0,
                        fp.short_sentence_ratio * 100.0,
                        fp.dialogue_ratio * 100.0
                    ))
                    .color(color::TEXT_DIM)
                    .size(11.5),
                );
                ui.label(
                    RichText::new(format!(
                        "标点：逗号 {} · 句号 {} · 破折号 {} · 省略号 {}",
                        fp.punctuation.commas,
                        fp.punctuation.periods,
                        fp.punctuation.dashes,
                        fp.punctuation.ellipses
                    ))
                    .color(color::TEXT_DIM)
                    .size(11.5),
                );
                ui.add_space(4.0);
                ui.label(RichText::new("高频二元短语").color(color::TEXT_DIM).size(11.5));
                ui.horizontal_wrapped(|ui| {
                    for (t, n) in fp.top_terms.iter().take(20) {
                        theme::pill(
                            ui,
                            &format!("{t} × {n}"),
                            color::TEXT,
                            color::SURFACE_HI,
                        );
                    }
                });
            });
        }
    }

    fn ui_tools_aigc(&mut self, ui: &mut egui::Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.label(RichText::new("AIGC 检测（启发式 + LLM 评分 prompt）").size(14.0).strong());
            dim_label(
                ui,
                "本地启发式仅作快筛；LLM 评分 prompt 可一键复制到 ChatGPT / Claude / 写作助手。",
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.aigc_use_current_chapter, "使用当前章节正文");
                if ui.button("🧪  本地启发式分析").clicked() {
                    let body = if self.aigc_use_current_chapter {
                        self.chapter_body.clone()
                    } else {
                        self.aigc_input.clone()
                    };
                    self.aigc_report = Some(crate::aigc::quick_score(&body));
                }
                if ui.button("🧠  生成 LLM 评分 prompt").clicked() {
                    let body = if self.aigc_use_current_chapter {
                        self.chapter_body.clone()
                    } else {
                        self.aigc_input.clone()
                    };
                    self.aigc_llm_prompt = crate::aigc::llm_judge_prompt(&body);
                }
            });
            if !self.aigc_use_current_chapter {
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt("aigc_input_scroll")
                    .max_height(180.0)
                    .show(ui, |ui| {
                        ui.add(
                            TextEdit::multiline(&mut self.aigc_input)
                                .desired_width(f32::INFINITY)
                                .desired_rows(8)
                                .hint_text("在此粘贴待检测的文本"),
                        );
                    });
            }
        });
        if let Some(rep) = self.aigc_report.clone() {
            ui.add_space(8.0);
            theme::card_frame().show(ui, |ui| {
                let color_for = |level: &str| match level {
                    "偏人类" => color::SUCCESS,
                    "可疑" => color::WARNING,
                    "偏 AI" => color::DANGER,
                    _ => color::TEXT_DIM,
                };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("综合分数：{:.2}", rep.overall_score)).size(15.0).strong());
                    theme::pill(ui, &rep.level, Color32::WHITE, color_for(&rep.level));
                });
                ui.add_space(6.0);
                for s in &rep.signals {
                    ui.label(
                        RichText::new(format!(
                            "{:<14}  命中 {:>3}  Δ {:.2}  · {}",
                            s.label, s.hits, s.score_delta, s.note
                        ))
                        .color(color::TEXT)
                        .size(12.0)
                        .monospace(),
                    );
                }
                if !rep.samples.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!("证据示例：{}", rep.samples.join("、")))
                            .color(color::TEXT_DIM)
                            .size(11.5),
                    );
                }
            });
        }
        if !self.aigc_llm_prompt.is_empty() {
            ui.add_space(8.0);
            theme::card_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("LLM 评分 prompt").size(13.0).strong());
                    if ui.button("📋  复制").clicked() {
                        ui.ctx().copy_text(self.aigc_llm_prompt.clone());
                    }
                    if ui.button("→  发送到写作助手").clicked() {
                        self.assistant_input = self.aigc_llm_prompt.clone();
                        self.section = Section::Assistant;
                    }
                });
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt("aigc_prompt_scroll")
                    .max_height(280.0)
                    .show(ui, |ui| {
                        ui.add(
                            TextEdit::multiline(&mut self.aigc_llm_prompt)
                                .desired_width(f32::INFINITY)
                                .desired_rows(10)
                                .font(egui::TextStyle::Monospace),
                        );
                    });
            });
        }
    }

    fn ui_tools_analytics(&mut self, ui: &mut egui::Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.label(RichText::new("数据分析").size(14.0).strong());
            dim_label(ui, "字数趋势、状态分布、达标率、近 30 天写作速度。");
            ui.add_space(6.0);
            if ui.button("⟳  重新计算").clicked() {
                if let Some(ref project) = self.project {
                    let goal = project.chapter_word_goal.max(0);
                    let tol = self.settings.effective_word_tolerance();
                    self.analytics_report = Some(crate::analytics::analyze(project, goal, tol));
                }
            }
        });
        if self.analytics_report.is_none() {
            if let Some(ref project) = self.project {
                let goal = project.chapter_word_goal.max(0);
                let tol = self.settings.effective_word_tolerance();
                self.analytics_report = Some(crate::analytics::analyze(project, goal, tol));
            }
        }
        if let Some(rep) = self.analytics_report.clone() {
            ui.add_space(8.0);
            theme::card_frame().show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    Self::stat_box(ui, "章节数", &rep.total_chapters.to_string());
                    Self::stat_box(ui, "总字数", &rep.total_words.to_string());
                    Self::stat_box(ui, "平均字/章", &format!("{:.0}", rep.avg_words));
                    Self::stat_box(ui, "字数目标", &rep.goal.to_string());
                    Self::stat_box(ui, "达标章节", &rep.on_target.to_string());
                    Self::stat_box(ui, "偏短章节", &rep.under_target.to_string());
                    Self::stat_box(ui, "偏长章节", &rep.over_target.to_string());
                });
            });
            ui.add_space(8.0);
            theme::card_frame().show(ui, |ui| {
                ui.label(RichText::new("状态分布").size(13.0).strong());
                ui.add_space(4.0);
                for (st, n) in &rep.status_distribution {
                    ui.horizontal(|ui| {
                        theme::pill(ui, status_label(st), Color32::WHITE, status_color(st));
                        ui.label(
                            RichText::new(format!("{n} 章")).color(color::TEXT).size(12.0),
                        );
                    });
                }
            });
            ui.add_space(8.0);
            theme::card_frame().show(ui, |ui| {
                ui.label(RichText::new("近 30 天写作速度").size(13.0).strong());
                ui.add_space(4.0);
                if rep.recent_velocity.is_empty() {
                    dim_label(ui, "暂无数据");
                } else {
                    egui::ScrollArea::horizontal()
                        .id_salt("velocity_scroll")
                        .max_width(f32::INFINITY)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                for v in &rep.recent_velocity {
                                    ui.vertical(|ui| {
                                        ui.label(
                                            RichText::new(&v.date)
                                                .color(color::TEXT_DIM)
                                                .size(10.5),
                                        );
                                        ui.label(
                                            RichText::new(format!("{} 章", v.chapters))
                                                .color(color::TEXT)
                                                .size(12.0)
                                                .strong(),
                                        );
                                        ui.label(
                                            RichText::new(format!("{} 字", v.words))
                                                .color(color::ACCENT_HI)
                                                .size(11.0),
                                        );
                                    });
                                    ui.add_space(8.0);
                                }
                            });
                        });
                }
            });
        }
    }

    fn ui_tools_fanfic(&mut self, ui: &mut egui::Ui) {
        theme::card_frame().show(ui, |ui| {
            ui.label(RichText::new("同人书向导").size(14.0).strong());
            dim_label(
                ui,
                "粘贴原作样本，启发式抽取角色 / 口头禅，落到 story/fanfic_brief.md。",
            );
            ui.add_space(6.0);
            egui::ScrollArea::vertical()
                .id_salt("fanfic_sample_scroll")
                .max_height(200.0)
                .show(ui, |ui| {
                    ui.add(
                        TextEdit::multiline(&mut self.fanfic_sample)
                            .desired_width(f32::INFINITY)
                            .desired_rows(8)
                            .hint_text("在此粘贴原作 1-3 章，作为提取依据"),
                    );
                });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button("🧪  从样本抽取").clicked() {
                    let mut b = crate::fanfic::quick_extract(&self.fanfic_sample);
                    b.origin_title = self.fanfic_brief.origin_title.clone();
                    b.origin_author = self.fanfic_brief.origin_author.clone();
                    b.fanfic_premise = self.fanfic_brief.fanfic_premise.clone();
                    self.fanfic_brief = b;
                    self.fanfic_msg = "已抽取（角色/口头禅由启发式给出，请人工核对）".into();
                }
                if ui.button("💾  保存到 story/fanfic_brief.md").clicked() {
                    if let Some(ref root) = self.novel_path {
                        match crate::fanfic::save(root, &self.fanfic_brief) {
                            Ok(()) => self.fanfic_msg = "已保存".into(),
                            Err(e) => self.fanfic_msg = format!("保存失败：{e}"),
                        }
                    }
                }
            });
        });
        ui.add_space(8.0);
        theme::card_frame().show(ui, |ui| {
            ui.label(RichText::new("同人 Brief 表单").size(13.0).strong());
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("原作名称");
                ui.add(
                    TextEdit::singleline(&mut self.fanfic_brief.origin_title)
                        .desired_width(240.0),
                );
                ui.label("原作作者");
                ui.add(
                    TextEdit::singleline(&mut self.fanfic_brief.origin_author)
                        .desired_width(180.0),
                );
            });
            ui.label(RichText::new("同人主线设定").color(color::TEXT_DIM).size(11.5));
            ui.add(
                TextEdit::multiline(&mut self.fanfic_brief.fanfic_premise)
                    .desired_width(f32::INFINITY)
                    .desired_rows(3),
            );
            ui.label(
                RichText::new(format!(
                    "已抽取候选角色 {} 个 · 口头禅 {} 条",
                    self.fanfic_brief.characters.len(),
                    self.fanfic_brief.catchphrases.len()
                ))
                .color(color::TEXT_DIM)
                .size(11.5),
            );
            if !self.fanfic_msg.is_empty() {
                dim_label(ui, &self.fanfic_msg);
            }
        });
        ui.add_space(8.0);
        theme::card_frame().show(ui, |ui| {
            ui.label(RichText::new("批量写 N 章规划").size(13.0).strong());
            dim_label(ui, "估算批次拆分；与定时写作组合使用。");
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("起始章节号");
                let mut start = self
                    .project
                    .as_ref()
                    .and_then(|p| p.chapters.iter().map(|c| c.number).max())
                    .map(|m| m + 1)
                    .unwrap_or(1);
                ui.add(egui::DragValue::new(&mut start).range(1..=50_000));
                ui.label("待写章节数");
                ui.add(egui::DragValue::new(&mut self.fanfic_total).range(1..=10_000));
                ui.label("批次大小");
                ui.add(egui::DragValue::new(&mut self.fanfic_batch_size).range(1..=200));
                ui.label("分钟/章");
                ui.add(egui::DragValue::new(&mut self.fanfic_minutes).range(1..=240));
                if ui.button("📐  生成批次").clicked() {
                    self.fanfic_plan = Some(crate::fanfic::plan_batch(
                        start,
                        self.fanfic_total,
                        self.fanfic_batch_size,
                        self.fanfic_minutes,
                    ));
                }
            });
            if let Some(ref plan) = self.fanfic_plan {
                ui.add_space(6.0);
                let total_minutes: i32 = plan.batches.iter().map(|b| b.estimated_minutes).sum();
                ui.label(
                    RichText::new(format!(
                        "共 {} 批 · 总耗时 ≈ {} 分钟（{:.1} 小时）",
                        plan.batches.len(),
                        total_minutes,
                        total_minutes as f32 / 60.0
                    ))
                    .color(color::TEXT)
                    .size(12.0),
                );
                for b in &plan.batches {
                    ui.label(
                        RichText::new(format!(
                            "  第 {}-{} 章 · ≈ {} 分钟",
                            b.from, b.to, b.estimated_minutes
                        ))
                        .color(color::TEXT_DIM)
                        .size(11.5),
                    );
                }
            }
        });
    }

    // ---------- 设置 ----------
    fn ui_settings(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().auto_shrink([false; 2]).show(ui, |ui| {
            ui.label(RichText::new("服务商管理").size(20.0).strong());
            ui.add_space(4.0);
            dim_label(
                ui,
                "Studio 为全局配置（跨小说共享）；当前小说目录下的 .env 仅覆盖本项目的活跃服务商。",
            );
            ui.add_space(8.0);

            self.ui_detected_card(ui);

            ui.add_space(16.0);
            self.ui_vendor_grid(ui);

            ui.add_space(16.0);
            self.ui_writing_llm_card(ui);

            ui.add_space(12.0);
            self.ui_review_llm_card(ui);

            ui.add_space(16.0);
            self.ui_word_governance_card(ui);

            ui.add_space(12.0);
            self.ui_agent_routing_card(ui);

            ui.add_space(12.0);
            self.ui_notify_card(ui);

            if self.novel_path.is_some() {
                ui.add_space(20.0);
                ui.collapsing(
                    RichText::new("高级：编辑当前小说 .env（项目级覆盖）")
                        .color(color::TEXT_DIM)
                        .size(12.5),
                    |ui| {
                        theme::card_frame().show(ui, |ui| {
                            ui.label(
                                RichText::new("小说级 .env（覆盖当前活跃服务商）")
                                    .size(13.0)
                                    .strong(),
                            );
                            ui.add_space(2.0);
                            dim_label(
                                ui,
                                "仅当前小说目录生效；留空表示沿用 Studio 中活跃服务商的配置。",
                            );
                            ui.add_space(6.0);
                            Self::ui_llm_form(ui, &mut self.novel_llm, "n");
                            ui.add_space(6.0);
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("💾  保存").clicked() {
                                        self.save_novel_config_ui();
                                    }
                                },
                            );
                        });
                    },
                );
            }
        });
    }

    fn ui_writing_llm_card(&mut self, ui: &mut egui::Ui) {
        let configured = self.configured_vendor_ids();
        let mut save_settings = false;
        theme::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("✍  写作 LLM 配置").size(14.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    theme::pill(ui, "助手 / 定时写作", color::TEXT, color::SURFACE_HI);
                });
            });
            ui.add_space(4.0);
            dim_label(ui, "用于「写作助手」与「定时写作」的模型。");
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.label(RichText::new("活跃服务商").size(11.5).color(color::TEXT_DIM));
                let cur = self.settings.writing_vendor.clone();
                let sel = if cur.is_empty() {
                    "（未选择）".into()
                } else {
                    Self::vendor_label(&cur)
                };
                ComboBox::from_id_salt("set_writing_vendor")
                    .width(240.0)
                    .selected_text(sel)
                    .show_ui(ui, |ui| {
                        if configured.is_empty() {
                            ui.label("（请先在上方配置服务商）");
                        }
                        for id in &configured {
                            if ui
                                .selectable_label(cur == *id, Self::vendor_label(id))
                                .clicked()
                            {
                                self.settings.writing_vendor = id.clone();
                                save_settings = true;
                            }
                        }
                    });
            });
            ui.add_space(6.0);
            if ui
                .checkbox(&mut self.settings.writing_streaming, "流式输出（边生成边显示）")
                .changed()
            {
                save_settings = true;
            }
        });
        if save_settings {
            let _ = self.paths.save_settings(&self.settings);
        }
    }

    fn ui_review_llm_card(&mut self, ui: &mut egui::Ui) {
        let configured = self.configured_vendor_ids();
        let mut save_settings = false;
        theme::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("🔍  审计 LLM 配置").size(14.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    theme::pill(ui, "审核 / 改写", color::TEXT, color::SURFACE_HI);
                });
            });
            ui.add_space(4.0);
            dim_label(ui, "用于「审核」页的审计与 AI 改写。");
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                ui.label(RichText::new("活跃服务商").size(11.5).color(color::TEXT_DIM));
                let cur = self.settings.review_vendor.clone();
                let sel = if cur.is_empty() {
                    "（未选择）".into()
                } else {
                    Self::vendor_label(&cur)
                };
                ComboBox::from_id_salt("set_review_vendor")
                    .width(240.0)
                    .selected_text(sel)
                    .show_ui(ui, |ui| {
                        if configured.is_empty() {
                            ui.label("（请先在上方配置服务商）");
                        }
                        for id in &configured {
                            if ui
                                .selectable_label(cur == *id, Self::vendor_label(id))
                                .clicked()
                            {
                                self.settings.review_vendor = id.clone();
                                save_settings = true;
                            }
                        }
                    });
            });
            ui.add_space(6.0);
            if ui
                .checkbox(&mut self.settings.review_streaming, "流式输出（边生成边显示）")
                .changed()
            {
                save_settings = true;
            }
        });
        if save_settings {
            let _ = self.paths.save_settings(&self.settings);
        }
    }

    fn ui_word_governance_card(&mut self, ui: &mut egui::Ui) {
        let mut save_settings = false;
        theme::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("📏  字数治理").size(14.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    theme::pill(ui, "目标 ± 容差 / 单次纠偏", color::TEXT, color::SURFACE_HI);
                });
            });
            ui.add_space(4.0);
            dim_label(
                ui,
                "目标字数取「小说设定」中的「目标字/章」；容差用于判断章节是否「达标」与生成归一化 prompt。",
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("容差（绝对字数）").size(11.5).color(color::TEXT_DIM));
                if ui
                    .add(
                        egui::DragValue::new(&mut self.settings.word_tolerance)
                            .range(0..=100_000)
                            .speed(10.0),
                    )
                    .changed()
                {
                    save_settings = true;
                }
                if ui
                    .checkbox(
                        &mut self.settings.word_normalize_on_save,
                        "保存章节时弹出「归一化 prompt」",
                    )
                    .changed()
                {
                    save_settings = true;
                }
            });
            ui.add_space(4.0);
            if ui
                .checkbox(
                    &mut self.settings.auto_refresh_state_after_chapter_save,
                    "章节保存后自动 AI 刷新长期记忆档案（避免「生成下一章」与前文错位）",
                )
                .on_hover_text(
                    "对齐 inkoswin 的「连续性档案」前提：\n\
                     · 保存章节成功后，自动串联 `AI 刷新全部`（排除 book_rules.md）\n\
                     · 刷新范围：novel_brief / current_state / pending_hooks /\n\
                       subplot_board / emotional_arcs / character_matrix /\n\
                       particle_ledger；chapter_summaries 本地重建\n\
                     · 写作 LLM 未配置 / 有其它任务在跑时会静默跳过（OpLog 记录）\n\
                     · 每次保存会消耗 7 次写作 LLM 调用；如担心成本可关闭，\n\
                       改用写作区「🔁 保存并同步记忆」按需触发",
                )
                .changed()
            {
                save_settings = true;
            }
            if ui
                .checkbox(
                    &mut self.settings.fast_generate_next_chapter,
                    "生成下一章使用快速模式（不等待长期记忆刷新）",
                )
                .on_hover_text(
                    "开启后点击「生成下一章」会跳过“等待状态档案刷新完成”的门控，\n\
                     可更快发起生成，但上下文可能短时间落后于最新章节。",
                )
                .changed()
            {
                save_settings = true;
            }
        });
        if save_settings {
            let _ = self.paths.save_settings(&self.settings);
        }
    }

    fn ui_agent_routing_card(&mut self, ui: &mut egui::Ui) {
        let configured = self.configured_vendor_ids();
        let mut save_settings = false;
        self.settings.agent_routing.ensure_keys();
        theme::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("🧭  Agent 路由").size(14.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    theme::pill(
                        ui,
                        "致敬 Narcooo/inkos · per-agent vendor",
                        color::TEXT,
                        color::SURFACE_HI,
                    );
                });
            });
            dim_label(
                ui,
                "针对管线中的不同 agent（plan / draft / audit / aigc 等）单独覆盖 vendor 与 model。留空表示沿用「写作 LLM」/「审计 LLM」。",
            );
            ui.add_space(6.0);

            let keys: Vec<(&'static str, &'static str)> =
                crate::agent_routing::AGENT_KEYS.to_vec();
            for (key, label) in keys {
                let entry = self
                    .settings
                    .agent_routing
                    .map
                    .entry(key.to_string())
                    .or_default();
                let mut local = entry.clone();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(label).size(12.5).color(color::TEXT));
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("vendor").size(11.0).color(color::TEXT_DIM));
                    let cur = local.vendor.clone();
                    let sel = if cur.is_empty() {
                        "（沿用默认）".into()
                    } else {
                        Self::vendor_label(&cur)
                    };
                    ComboBox::from_id_salt(format!("route_vendor_{key}"))
                        .width(220.0)
                        .selected_text(sel)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(cur.is_empty(), "（沿用默认）")
                                .clicked()
                            {
                                local.vendor = String::new();
                            }
                            for id in &configured {
                                if ui
                                    .selectable_label(cur == *id, Self::vendor_label(id))
                                    .clicked()
                                {
                                    local.vendor = id.clone();
                                }
                            }
                        });
                    ui.label(RichText::new("model 覆盖").size(11.0).color(color::TEXT_DIM));
                    ui.add(
                        TextEdit::singleline(&mut local.model_override)
                            .hint_text("如 gpt-5.4 / claude-opus-4 / glm-4.5-air …")
                            .desired_width(260.0),
                    );
                });
                if local != *entry {
                    *entry = local;
                    save_settings = true;
                }
                ui.add_space(2.0);
            }
        });
        if save_settings {
            let _ = self.paths.save_settings(&self.settings);
        }
    }

    fn ui_notify_card(&mut self, ui: &mut egui::Ui) {
        let mut save_settings = false;
        theme::card_frame().show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("🔔  通知 · Telegram / 飞书 / 企业微信 / Webhook").size(14.0).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("🚀  发送测试通知").clicked() {
                        let evt = crate::notify::NotifyEvent {
                            kind: "chapter_done".into(),
                            title: "InkOS 测试通知".into(),
                            body: "如果你看到这条消息，通道工作正常。".into(),
                            novel: self
                                .project
                                .as_ref()
                                .map(|p| p.title.clone())
                                .unwrap_or_default(),
                            chapter_no: None,
                            timestamp: crate::project::now_iso(),
                        };
                        let results = crate::notify::notify_all(&self.settings.notify, &evt);
                        let summary: Vec<String> = results
                            .iter()
                            .map(|r| {
                                format!(
                                    "{}：{}",
                                    r.channel,
                                    if r.ok { "OK" } else { "失败" }
                                )
                            })
                            .collect();
                        self.status_message = format!("通知测试：{}", summary.join(" · "));
                        oplog::try_append(
                            self.novel_path.as_deref(),
                            "通知测试",
                            &self.status_message,
                        );
                    }
                });
            });
            ui.add_space(4.0);
            dim_label(ui, "未配置的通道会自动跳过；HMAC 签名（飞书）已内置。");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui
                    .checkbox(&mut self.settings.notify.on_chapter_done, "章节完成时通知")
                    .changed()
                {
                    save_settings = true;
                }
                if ui
                    .checkbox(&mut self.settings.notify.on_audit_done, "审计完成时通知")
                    .changed()
                {
                    save_settings = true;
                }
                if ui
                    .checkbox(&mut self.settings.notify.on_error, "出错时通知")
                    .changed()
                {
                    save_settings = true;
                }
            });

            ui.add_space(6.0);
            egui::CollapsingHeader::new(RichText::new("Telegram").color(color::TEXT_DIM))
                .id_salt("notify_telegram")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Bot Token");
                        if ui
                            .add(
                                TextEdit::singleline(&mut self.settings.notify.telegram.bot_token)
                                    .password(true)
                                    .desired_width(360.0),
                            )
                            .changed()
                        {
                            save_settings = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Chat ID");
                        if ui
                            .add(
                                TextEdit::singleline(&mut self.settings.notify.telegram.chat_id)
                                    .desired_width(240.0),
                            )
                            .changed()
                        {
                            save_settings = true;
                        }
                    });
                });
            egui::CollapsingHeader::new(RichText::new("飞书 Lark").color(color::TEXT_DIM))
                .id_salt("notify_feishu")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Webhook URL");
                        if ui
                            .add(
                                TextEdit::singleline(&mut self.settings.notify.feishu.webhook_url)
                                    .desired_width(420.0),
                            )
                            .changed()
                        {
                            save_settings = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Secret（可选）");
                        if ui
                            .add(
                                TextEdit::singleline(&mut self.settings.notify.feishu.secret)
                                    .password(true)
                                    .desired_width(320.0),
                            )
                            .changed()
                        {
                            save_settings = true;
                        }
                    });
                });
            egui::CollapsingHeader::new(RichText::new("企业微信").color(color::TEXT_DIM))
                .id_salt("notify_wecom")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Webhook URL");
                        if ui
                            .add(
                                TextEdit::singleline(&mut self.settings.notify.wecom.webhook_url)
                                    .desired_width(420.0),
                            )
                            .changed()
                        {
                            save_settings = true;
                        }
                    });
                });
            egui::CollapsingHeader::new(RichText::new("Webhook（自定义）").color(color::TEXT_DIM))
                .id_salt("notify_webhook")
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("URL");
                        if ui
                            .add(
                                TextEdit::singleline(&mut self.settings.notify.webhook.url)
                                    .desired_width(420.0),
                            )
                            .changed()
                        {
                            save_settings = true;
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label("Method");
                        if ui
                            .add(
                                TextEdit::singleline(&mut self.settings.notify.webhook.method)
                                    .hint_text("POST / GET")
                                    .desired_width(100.0),
                            )
                            .changed()
                        {
                            save_settings = true;
                        }
                    });
                });
        });
        if save_settings {
            let _ = self.paths.save_settings(&self.settings);
        }
    }

    /// 若当前打开的小说目录下存在 `.env` 且含有 LLM 字段，提示用户「项目级覆盖生效」。
    /// 不再扫描 / 展示 ~/.inkos/.env（该路径已被首启迁移归档）。
    fn ui_detected_card(&self, ui: &mut egui::Ui) {
        let Some(root) = self.novel_path.as_ref() else {
            return;
        };
        let env_path = self.paths.novel_env_path(root);
        if !env_path.exists() {
            return;
        }
        let n = &self.novel_llm;
        let has_any = !n.base_url.trim().is_empty()
            || !n.api_key.trim().is_empty()
            || !n.model.trim().is_empty()
            || !n.temperature.trim().is_empty()
            || !n.max_tokens.trim().is_empty()
            || !n.thinking_budget.trim().is_empty();
        if !has_any {
            return;
        }
        egui::Frame::default()
            .fill(Color32::from_rgb(0x2a, 0x24, 0x18))
            .stroke(Stroke::new(1.0, Color32::from_rgb(0x6a, 0x4d, 0x1f)))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(16, 12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("本小说 .env 覆盖生效：")
                            .color(Color32::from_rgb(0xf0, 0xc0, 0x70))
                            .strong()
                            .size(12.5),
                    );
                    ui.label(
                        RichText::new(env_path.display().to_string())
                            .color(color::TEXT_DIM)
                            .size(11.5),
                    );
                });
                ui.add_space(4.0);
                let mono = |ui: &mut egui::Ui, label: &str, value: &str| {
                    if value.trim().is_empty() {
                        return;
                    }
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!("{label}:")).color(color::TEXT_DIM).size(12.0),
                        );
                        ui.label(RichText::new(value).color(color::TEXT).monospace().size(12.0));
                    });
                };
                mono(ui, "Base URL", &n.base_url);
                mono(ui, "Model", &n.model);
                mono(ui, "Provider", &n.provider);
                if !n.api_key.trim().is_empty() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("API Key:").color(color::TEXT_DIM).size(12.0));
                        ui.label(RichText::new("已设置").color(color::SUCCESS).size(12.0));
                    });
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "本小说发起的 LLM 请求会把这些字段覆盖到当前活跃服务商之上。",
                    )
                    .color(color::TEXT_DIM)
                    .size(11.5),
                );
            });
    }

    fn ui_vendor_grid(&mut self, ui: &mut egui::Ui) {
        let mut to_open: Option<String> = None;
        ui.columns(2, |cols| {
            for (i, v) in VENDORS.iter().enumerate() {
                let col = &mut cols[i % 2];
                let configured = self
                    .settings
                    .vendors
                    .get(v.id)
                    .map(|c| c.is_configured())
                    .unwrap_or(false);
                if Self::vendor_card(col, v, configured) {
                    to_open = Some(v.id.to_string());
                }
                col.add_space(8.0);
            }
        });
        if let Some(id) = to_open {
            self.open_vendor_editor(&id);
        }
    }

    fn vendor_card(ui: &mut egui::Ui, v: &VendorPreset, configured: bool) -> bool {
        let resp = egui::Frame::default()
            .fill(color::SURFACE)
            .stroke(Stroke::new(1.0, color::BORDER))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(16, 14))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(v.name).color(color::TEXT).size(14.0).strong(),
                        );
                        ui.add_space(2.0);
                        let (lbl, c) = if configured {
                            ("已配置", color::SUCCESS)
                        } else {
                            ("未配置", color::TEXT_FAINT)
                        };
                        ui.label(RichText::new(lbl).color(c).size(12.0));
                        if !v.note.is_empty() {
                            ui.label(RichText::new(v.note).color(color::TEXT_FAINT).size(10.5));
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let dot = if configured { color::SUCCESS } else { color::TEXT_FAINT };
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 4.0, dot);
                    });
                });
            })
            .response
            .interact(egui::Sense::click());
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        resp.clicked()
    }

    fn open_vendor_editor(&mut self, id: &str) {
        let preset = find_vendor(id);
        let existing = self.settings.vendors.get(id).cloned();
        let buf = match (existing, preset) {
            (Some(c), _) => c,
            (None, Some(p)) => VendorConfig {
                base_url: p.base_url.into(),
                model: p.default_model.into(),
                ..Default::default()
            },
            _ => VendorConfig::default(),
        };
        self.edit_vendor_buf = buf;
        self.edit_vendor_id = Some(id.to_string());
        self.vendor_test_msg.clear();
        self.vendor_test_task = None;
    }

    fn show_vendor_editor(&mut self, ctx: &egui::Context) {
        let Some(id) = self.edit_vendor_id.clone() else { return };
        let preset = find_vendor(&id);
        let title = preset.map(|p| p.name.to_string()).unwrap_or_else(|| id.clone());

        let mut open = true;
        let mut want_save = false;
        let mut want_delete = false;
        let mut want_close = false;
        let mut want_test = false;

        egui::Window::new(format!("配置：{title}"))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .default_width(540.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Base URL").size(11.5).color(color::TEXT_DIM));
                    ui.add(
                        TextEdit::singleline(&mut self.edit_vendor_buf.base_url)
                            .desired_width(f32::INFINITY)
                            .id_salt("ed_base"),
                    );
                    ui.add_space(6.0);
                    ui.label(RichText::new("API Key").size(11.5).color(color::TEXT_DIM));
                    ui.add(
                        TextEdit::singleline(&mut self.edit_vendor_buf.api_key)
                            .password(true)
                            .desired_width(f32::INFINITY)
                            .id_salt("ed_key"),
                    );
                    ui.add_space(6.0);
                    ui.label(RichText::new("Model").size(11.5).color(color::TEXT_DIM));
                    ui.add(
                        TextEdit::singleline(&mut self.edit_vendor_buf.model)
                            .desired_width(f32::INFINITY)
                            .id_salt("ed_model"),
                    );
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new("Temperature").size(11.5).color(color::TEXT_DIM));
                            ui.add(
                                TextEdit::singleline(&mut self.edit_vendor_buf.temperature)
                                    .desired_width(140.0)
                                    .id_salt("ed_t"),
                            );
                        });
                        ui.vertical(|ui| {
                            ui.label(RichText::new("Max Tokens").size(11.5).color(color::TEXT_DIM));
                            ui.add(
                                TextEdit::singleline(&mut self.edit_vendor_buf.max_tokens)
                                    .desired_width(140.0)
                                    .id_salt("ed_x"),
                            );
                        });
                        ui.vertical(|ui| {
                            ui.label(
                                RichText::new("Thinking Budget").size(11.5).color(color::TEXT_DIM),
                            );
                            ui.add(
                                TextEdit::singleline(&mut self.edit_vendor_buf.thinking_budget)
                                    .desired_width(140.0)
                                    .id_salt("ed_h"),
                            );
                        });
                    });
                    if let Some(p) = preset {
                        if !p.note.is_empty() {
                            ui.add_space(6.0);
                            dim_label(ui, p.note);
                        }
                    }

                    ui.add_space(10.0);
                    egui::Frame::default()
                        .fill(color::SURFACE_HI)
                        .stroke(Stroke::new(1.0, color::BORDER))
                        .corner_radius(CornerRadius::same(8))
                        .inner_margin(Margin::symmetric(12, 8))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("连接测试").color(color::TEXT_DIM).size(11.5),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if self.vendor_test_task.is_some() {
                                            ui.add_enabled(false, egui::Button::new("⏳  测试中…"));
                                        } else if ui.button("🔌  测试连接").clicked() {
                                            want_test = true;
                                        }
                                    },
                                );
                            });
                            if !self.vendor_test_msg.is_empty() {
                                let color = if self.vendor_test_msg.starts_with('✓') {
                                    color::SUCCESS
                                } else {
                                    color::DANGER
                                };
                                ui.label(
                                    RichText::new(&self.vendor_test_msg).color(color).size(12.0),
                                );
                            } else if self.vendor_test_task.is_none() {
                                dim_label(ui, "点击「测试连接」会向所选模型发起一次最小请求。");
                            }
                        });

                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("💾  保存").clicked() {
                            want_save = true;
                        }
                        if ui.button("取消").clicked() {
                            want_close = true;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("移除此服务商配置").color(color::DANGER),
                                    )
                                    .fill(color::SURFACE_HI),
                                )
                                .clicked()
                            {
                                want_delete = true;
                            }
                        });
                    });
                });
            });

        if want_test {
            let cfg = self.edit_vendor_buf.clone();
            if cfg.base_url.trim().is_empty() || cfg.model.trim().is_empty() {
                self.vendor_test_msg = "✗  请先填写 Base URL 与 Model".into();
            } else {
                self.vendor_test_msg.clear();
                let model = cfg.model.clone();
                self.vendor_test_task = Some(spawn_ping(cfg, model));
            }
        }

        if want_save {
            let buf = self.edit_vendor_buf.clone();
            self.settings.vendors.insert(id.clone(), buf.clone());
            if buf.is_configured() {
                if self.settings.active_vendor.is_empty() {
                    self.settings.active_vendor = id.clone();
                }
                if self.settings.writing_vendor.is_empty() {
                    self.settings.writing_vendor = id.clone();
                }
                if self.settings.review_vendor.is_empty() {
                    self.settings.review_vendor = id.clone();
                }
            }
            if let Err(e) = self.paths.save_settings(&self.settings) {
                self.status_message = format!("保存设置失败：{e}");
            } else {
                self.status_message = format!("已保存：{title}");
            }
            self.edit_vendor_id = None;
        } else if want_delete {
            self.settings.vendors.remove(&id);
            if self.settings.active_vendor == id {
                self.settings.active_vendor.clear();
            }
            if self.settings.review_vendor == id {
                self.settings.review_vendor.clear();
            }
            if self.settings.writing_vendor == id {
                self.settings.writing_vendor.clear();
            }
            let _ = self.paths.save_settings(&self.settings);
            self.status_message = format!("已移除：{title}");
            self.edit_vendor_id = None;
        } else if want_close || !open {
            self.edit_vendor_id = None;
        }
    }

    fn ui_llm_form(ui: &mut egui::Ui, cfg: &mut LlmConfig, id: &str) {
        let row = |ui: &mut egui::Ui, label: &str, value: &mut String, password: bool, salt: &str| {
            ui.label(RichText::new(label).size(11.5).color(color::TEXT_DIM));
            let mut e = TextEdit::singleline(value).desired_width(f32::INFINITY).id_salt(salt);
            if password {
                e = e.password(true);
            }
            ui.add(e);
            ui.add_space(2.0);
        };
        row(ui, "Provider", &mut cfg.provider, false, &(id.to_string() + "p"));
        row(ui, "Base URL", &mut cfg.base_url, false, &(id.to_string() + "b"));
        row(ui, "API Key", &mut cfg.api_key, true, &(id.to_string() + "k"));
        row(ui, "Model", &mut cfg.model, false, &(id.to_string() + "m"));
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("Temperature").size(11.5).color(color::TEXT_DIM));
                ui.add(
                    TextEdit::singleline(&mut cfg.temperature)
                        .desired_width(140.0)
                        .id_salt(id.to_string() + "t"),
                );
            });
            ui.vertical(|ui| {
                ui.label(RichText::new("Max Tokens").size(11.5).color(color::TEXT_DIM));
                ui.add(
                    TextEdit::singleline(&mut cfg.max_tokens)
                        .desired_width(140.0)
                        .id_salt(id.to_string() + "x"),
                );
            });
            ui.vertical(|ui| {
                ui.label(RichText::new("Thinking Budget").size(11.5).color(color::TEXT_DIM));
                ui.add(
                    TextEdit::singleline(&mut cfg.thinking_budget)
                        .desired_width(140.0)
                        .id_salt(id.to_string() + "h"),
                );
            });
        });
    }
}

fn first_line(s: &str) -> String {
    let s = s.trim();
    let line = s.lines().next().unwrap_or("").trim();
    if line.chars().count() > 80 {
        let cut: String = line.chars().take(80).collect();
        format!("{cut}…")
    } else {
        line.to_string()
    }
}

fn short_time() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

/// 0.7s 周期闪烁的「▌」光标，随每帧 ctx.input(i.time) 切换。
fn typewriter_cursor(ctx: &egui::Context) -> &'static str {
    let t = ctx.input(|i| i.time);
    ctx.request_repaint_after(std::time::Duration::from_millis(450));
    if (t * 1.4).fract() < 0.5 { "▌" } else { " " }
}

fn source_label(s: &str) -> &'static str {
    match s {
        "manual" => "手动保存",
        "ai_rewrite" => "AI 改写替换",
        "auto_gen" => "定时写作",
        "restore" => "恢复历史",
        _ => "其它",
    }
}

fn op_kind_color(k: &str) -> Color32 {
    if k.contains("失败") {
        color::DANGER
    } else if k.contains("AI") || k.contains("定时写作") {
        color::ACCENT_HI
    } else if k.contains("替换") || k.contains("恢复") || k.contains("备份") {
        color::WARNING
    } else if k.contains("保存") {
        color::SUCCESS
    } else {
        color::TEXT
    }
}

fn open_in_explorer(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer").arg(path).spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(path).spawn()?;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(path).spawn()?;
    }
    Ok(())
}

#[allow(dead_code)]
fn _unused_keep(_: &HashMap<String, VendorConfig>) {}
