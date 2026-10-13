//! SQLite 存储层：材料、卡片、复习记录与统计。

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS materials (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    title       TEXT NOT NULL,
    source      TEXT NOT NULL,
    source_kind TEXT NOT NULL,
    chars       INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS cards (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    material_id  INTEGER NOT NULL REFERENCES materials(id) ON DELETE CASCADE,
    seq          INTEGER NOT NULL DEFAULT 0,
    section      TEXT NOT NULL DEFAULT '',
    title        TEXT NOT NULL,
    content      TEXT NOT NULL DEFAULT '',
    summary      TEXT NOT NULL DEFAULT '',
    key_points   TEXT NOT NULL DEFAULT '[]',
    question     TEXT NOT NULL DEFAULT '',
    answer       TEXT NOT NULL DEFAULT '',
    tags         TEXT NOT NULL DEFAULT '[]',
    state        INTEGER NOT NULL DEFAULT 0,
    stability    REAL NOT NULL DEFAULT 0,
    difficulty   REAL NOT NULL DEFAULT 0,
    reps         INTEGER NOT NULL DEFAULT 0,
    lapses       INTEGER NOT NULL DEFAULT 0,
    last_review  TEXT,
    due          TEXT NOT NULL DEFAULT '',
    learned      INTEGER NOT NULL DEFAULT 0,
    exam_notes   TEXT NOT NULL DEFAULT '[]',
    created_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS exams (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    title       TEXT NOT NULL,
    material_id INTEGER,
    scope       TEXT NOT NULL DEFAULT '[]',
    slot_kinds  TEXT NOT NULL DEFAULT '[]',
    total       INTEGER NOT NULL DEFAULT 0,
    full_score  REAL NOT NULL DEFAULT 0,
    status      TEXT NOT NULL DEFAULT 'draft',
    score       REAL,
    correct     INTEGER NOT NULL DEFAULT 0,
    report      TEXT NOT NULL DEFAULT '',
    created_at  TEXT NOT NULL,
    finished_at TEXT
);

CREATE TABLE IF NOT EXISTS exam_items (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    exam_id     INTEGER NOT NULL REFERENCES exams(id) ON DELETE CASCADE,
    seq         INTEGER NOT NULL,
    kind        TEXT NOT NULL,
    card_id     INTEGER,
    card_title  TEXT NOT NULL DEFAULT '',
    stem        TEXT NOT NULL DEFAULT '',
    options     TEXT NOT NULL DEFAULT '[]',
    answer      TEXT NOT NULL DEFAULT '',
    explain     TEXT NOT NULL DEFAULT '',
    points      REAL NOT NULL DEFAULT 1,
    user_answer TEXT NOT NULL DEFAULT '',
    score       REAL,
    verdict     TEXT NOT NULL DEFAULT '',
    feedback    TEXT NOT NULL DEFAULT '',
    missed      TEXT NOT NULL DEFAULT '[]',
    suggestion  TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_exam_items_exam ON exam_items(exam_id);
CREATE INDEX IF NOT EXISTS idx_cards_material ON cards(material_id);
CREATE INDEX IF NOT EXISTS idx_cards_due ON cards(due);
CREATE INDEX IF NOT EXISTS idx_cards_learned ON cards(learned);

CREATE TABLE IF NOT EXISTS reviews (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    card_id        INTEGER NOT NULL REFERENCES cards(id) ON DELETE CASCADE,
    rated_at       TEXT NOT NULL,
    rating         INTEGER NOT NULL,
    elapsed_days   INTEGER NOT NULL DEFAULT 0,
    scheduled_days INTEGER NOT NULL DEFAULT 0,
    score          REAL,
    answer_text    TEXT NOT NULL DEFAULT '',
    mode           TEXT NOT NULL DEFAULT 'review'
);

CREATE INDEX IF NOT EXISTS idx_reviews_card ON reviews(card_id);
CREATE INDEX IF NOT EXISTS idx_reviews_date ON reviews(rated_at);

CREATE TABLE IF NOT EXISTS material_text (
    material_id INTEGER PRIMARY KEY REFERENCES materials(id) ON DELETE CASCADE,
    body        TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS plans (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    material_id INTEGER NOT NULL REFERENCES materials(id) ON DELETE CASCADE,
    days        INTEGER NOT NULL,
    per_day     INTEGER NOT NULL,
    start_date  TEXT NOT NULL,
    created_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS snapshots (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at TEXT NOT NULL,
    kind       TEXT NOT NULL,
    payload    TEXT NOT NULL,
    svg        TEXT NOT NULL
);
";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Material {
    pub id: i64,
    pub title: String,
    pub source: String,
    pub source_kind: String,
    pub chars: i64,
    pub created_at: String,
    pub card_count: i64,
    /// 还没有考点的单元数（模型不可用或调用失败时会攒下来）
    pub pending_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub id: i64,
    pub material_id: i64,
    pub material_title: String,
    pub seq: i64,
    /// 所属章节（材料目录 / 文档里的「分类」）
    pub section: String,
    pub title: String,
    /// 完整讲解正文（Markdown 原文，学习页直接渲染）
    pub content: String,
    pub summary: String,
    pub key_points: Vec<String>,
    pub question: String,
    pub answer: String,
    pub tags: Vec<String>,
    pub state: i64,
    pub stability: f64,
    pub difficulty: f64,
    pub reps: i64,
    pub lapses: i64,
    pub last_review: Option<String>,
    pub due: String,
    pub learned: bool,
    /// 测验里答错/答漏的要点，按时间追加，学习时能看到
    pub exam_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewRecord {
    pub id: i64,
    pub card_id: i64,
    pub card_title: String,
    pub rated_at: String,
    pub rating: i64,
    pub score: Option<f64>,
    pub mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub materials: i64,
    pub cards: i64,
    pub learned: i64,
    pub due_today: i64,
    pub reviews_total: i64,
    pub reviews_today: i64,
    pub avg_score: f64,
    pub streak_days: i64,
    pub active_days: i64,
    pub stage: String,
    pub stage_progress: f64,
    pub next_stage: String,
}

pub struct Store {
    pub conn: Connection,
}

fn parse_list(raw: &str) -> Vec<String> {
    serde_json::from_str::<Vec<String>>(raw).unwrap_or_default()
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("创建数据目录失败")?;
        }
        let conn = Connection::open(path).context("打开数据库失败")?;
        conn.execute_batch(SCHEMA).context("初始化数据库失败")?;
        // 老库补列（已存在时会报错，忽略即可）
        let _ = conn.execute("ALTER TABLE cards ADD COLUMN section TEXT NOT NULL DEFAULT ''", []);
        let _ = conn.execute("ALTER TABLE cards ADD COLUMN content TEXT NOT NULL DEFAULT ''", []);
        let _ = conn.execute("ALTER TABLE cards ADD COLUMN exam_notes TEXT NOT NULL DEFAULT '[]'", []);
        Ok(Self { conn })
    }

    pub fn insert_material(&self, title: &str, source: &str, kind: &str, chars: i64,
                           text: &str, created_at: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO materials (title, source, source_kind, chars, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![title, source, kind, chars, created_at],
        )?;
        let id = self.conn.last_insert_rowid();
        // 全文单独存放，避免列表查询时读取大字段
        self.conn.execute(
            "INSERT OR REPLACE INTO material_text (material_id, body) VALUES (?1, ?2)",
            params![id, text],
        )?;
        Ok(id)
    }

    pub fn material_text(&self, id: i64) -> Result<String> {
        let body: Option<String> = self
            .conn
            .query_row("SELECT body FROM material_text WHERE material_id = ?1", params![id], |r| r.get(0))
            .optional()?;
        Ok(body.unwrap_or_default())
    }

    pub fn list_materials(&self) -> Result<Vec<Material>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.title, m.source, m.source_kind, m.chars, m.created_at,
                    (SELECT COUNT(*) FROM cards c WHERE c.material_id = m.id),
                    (SELECT COUNT(*) FROM cards c
                      WHERE c.material_id = m.id AND TRIM(c.question) = '')
             FROM materials m ORDER BY m.id DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Material {
                id: r.get(0)?, title: r.get(1)?, source: r.get(2)?, source_kind: r.get(3)?,
                chars: r.get(4)?, created_at: r.get(5)?, card_count: r.get(6)?,
                pending_count: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn rename_material(&self, id: i64, title: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE materials SET title = ?2 WHERE id = ?1",
            params![id, title],
        )?;
        Ok(())
    }

    pub fn delete_material(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM materials WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn insert_cards(&self, material_id: i64, cards: &[NewCard], created_at: &str) -> Result<i64> {
        let mut n = 0;
        for (i, c) in cards.iter().enumerate() {
            self.conn.execute(
                "INSERT INTO cards (material_id, seq, section, title, content, summary, key_points, question, answer, tags, due, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    material_id,
                    i as i64 + 1,
                    c.section,
                    c.title,
                    c.content,
                    c.summary,
                    serde_json::to_string(&c.key_points)?,
                    c.question,
                    c.answer,
                    serde_json::to_string(&c.tags)?,
                    created_at, // due：新卡当天进入复习队列
                    created_at, // created_at
                ],
            )?;
            n += 1;
        }
        Ok(n)
    }

    fn map_card(r: &rusqlite::Row) -> rusqlite::Result<Card> {
        let key_points: String = r.get("key_points")?;
        let tags: String = r.get("tags")?;
        Ok(Card {
            id: r.get("id")?,
            material_id: r.get("material_id")?,
            material_title: r.get("material_title").unwrap_or_default(),
            seq: r.get("seq")?,
            section: r.get("section").unwrap_or_default(),
            title: r.get("title")?,
            content: r.get("content").unwrap_or_default(),
            summary: r.get("summary")?,
            key_points: parse_list(&key_points),
            question: r.get("question")?,
            answer: r.get("answer")?,
            tags: parse_list(&tags),
            state: r.get("state")?,
            stability: r.get("stability")?,
            difficulty: r.get("difficulty")?,
            reps: r.get("reps")?,
            lapses: r.get("lapses")?,
            last_review: r.get("last_review")?,
            due: r.get("due")?,
            learned: r.get::<_, i64>("learned")? != 0,
            exam_notes: parse_list(&r.get::<_, String>("exam_notes").unwrap_or_default()),
        })
    }

    const CARD_SELECT: &'static str =
        "SELECT c.*, m.title AS material_title FROM cards c JOIN materials m ON m.id = c.material_id";

    pub fn list_cards(&self, material_id: Option<i64>) -> Result<Vec<Card>> {
        let sql = format!("{} WHERE (?1 IS NULL OR c.material_id = ?1) ORDER BY c.material_id DESC, c.seq ASC", Self::CARD_SELECT);
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![material_id], |r| Self::map_card(r))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_card(&self, id: i64) -> Result<Option<Card>> {
        let sql = format!("{} WHERE c.id = ?1", Self::CARD_SELECT);
        Ok(self.conn.query_row(&sql, params![id], |r| Self::map_card(r)).optional()?)
    }

    /// 到期该复习、且今天还没复习过的卡：今天复习过的当天不再出现。
    pub fn due_cards(&self, today: &str, limit: i64) -> Result<Vec<Card>> {
        let sql = format!(
            "{} WHERE c.learned = 1 AND c.due <= ?1              AND (c.last_review IS NULL OR SUBSTR(c.last_review, 1, 10) <> ?1)              ORDER BY c.due ASC, c.id ASC LIMIT ?2",
            Self::CARD_SELECT
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![today, limit], |r| Self::map_card(r))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 今天的学习计划排到、但还没标记已学的单元（首页待办的前半部分）。
    pub fn scheduled_cards(&self, today: &str, limit: i64) -> Result<Vec<Card>> {
        let mut out: Vec<Card> = Vec::new();
        for plan in self.list_plans()? {
            let Some(today_plan) = self.plan_today(plan.id, today)? else { continue };
            out.extend(today_plan.day_units.into_iter().filter(|card| !card.learned));
            if out.len() as i64 >= limit {
                break;
            }
        }
        out.truncate(limit.max(1) as usize);
        Ok(out)
    }

    pub fn learning_cards(&self, limit: i64) -> Result<Vec<Card>> {
        let sql = format!(
            "{} WHERE c.learned = 0 ORDER BY c.material_id ASC, c.seq ASC LIMIT ?1",
            Self::CARD_SELECT
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit], |r| Self::map_card(r))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn mark_learned(&self, id: i64, learned: bool) -> Result<()> {
        self.conn.execute("UPDATE cards SET learned = ?2 WHERE id = ?1", params![id, learned as i64])?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn apply_schedule(&self, id: i64, state: i64, stability: f64, difficulty: f64,
                          reps: i64, lapses: i64, last_review: &str, due: &str, learned: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE cards SET state = ?2, stability = ?3, difficulty = ?4, reps = ?5, lapses = ?6,
                    last_review = ?7, due = ?8, learned = ?9 WHERE id = ?1",
            params![id, state, stability, difficulty, reps, lapses, last_review, due, learned as i64],
        )?;
        Ok(())
    }

    /// 写入某个学习单元的考点。
    pub fn update_digest(&self, id: i64, digest: &CardDigest) -> Result<()> {
        self.conn.execute(
            "UPDATE cards SET summary = ?2, key_points = ?3, question = ?4, answer = ?5 WHERE id = ?1",
            params![
                id,
                digest.summary,
                serde_json::to_string(&digest.key_points)?,
                digest.question,
                digest.answer
            ],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_review(&self, card_id: i64, rated_at: &str, rating: i64, elapsed: i64,
                         scheduled: i64, score: Option<f64>, answer: &str, mode: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO reviews (card_id, rated_at, rating, elapsed_days, scheduled_days, score, answer_text, mode)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![card_id, rated_at, rating, elapsed, scheduled, score, answer, mode],
        )?;
        Ok(())
    }

    pub fn recent_reviews(&self, limit: i64) -> Result<Vec<ReviewRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT r.id, r.card_id, c.title, r.rated_at, r.rating, r.score, r.mode
             FROM reviews r JOIN cards c ON c.id = r.card_id
             ORDER BY r.id DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |r| {
            Ok(ReviewRecord {
                id: r.get(0)?, card_id: r.get(1)?, card_title: r.get(2)?, rated_at: r.get(3)?,
                rating: r.get(4)?, score: r.get(5)?, mode: r.get(6)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// 未来若干天、每天的到期卡数量（用于复习计划图）。
    pub fn due_forecast(&self, from: &str, days: i64) -> Result<Vec<(String, i64)>> {
        let mut out = Vec::new();
        let mut stmt = self.conn.prepare(
            "SELECT due, COUNT(*) FROM cards WHERE learned = 1 AND due >= ?1 AND due <= ?2 GROUP BY due",
        )?;
        let rows = stmt.query_map(params![from, add_days(from, days)], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })?;
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn stats(&self, today: &str) -> Result<Stats> {
        let materials: i64 = self.conn.query_row("SELECT COUNT(*) FROM materials", [], |r| r.get(0))?;
        let cards: i64 = self.conn.query_row("SELECT COUNT(*) FROM cards", [], |r| r.get(0))?;
        let learned: i64 = self.conn.query_row("SELECT COUNT(*) FROM cards WHERE learned = 1", [], |r| r.get(0))?;
        // 与首页「今天要做」同一口径：今天已经复习过的不再算欠账
        let due_today: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM cards WHERE learned = 1 AND due <= ?1 \
             AND (last_review IS NULL OR substr(last_review, 1, 10) <> ?1)",
            params![today], |r| r.get(0))?;
        let reviews_total: i64 = self.conn.query_row("SELECT COUNT(*) FROM reviews", [], |r| r.get(0))?;
        let reviews_today: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM reviews WHERE substr(rated_at, 1, 10) = ?1", params![today], |r| r.get(0))?;
        let avg_score: f64 = self.conn.query_row(
            "SELECT COALESCE(AVG(score), 0) FROM reviews WHERE score IS NOT NULL", [], |r| r.get(0))?;
        let active_days: i64 = self.conn.query_row(
            "SELECT COUNT(DISTINCT substr(rated_at, 1, 10)) FROM reviews", [], |r| r.get(0))?;

        let mut days: Vec<String> = Vec::new();
        let mut stmt = self.conn.prepare("SELECT DISTINCT substr(rated_at, 1, 10) AS d FROM reviews ORDER BY d DESC")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for row in rows {
            days.push(row?);
        }
        let streak_days = streak(&days, today);

        let (stage, stage_progress, next_stage) = stage_of(learned, cards, avg_score);

        Ok(Stats {
            materials, cards, learned, due_today, reviews_total, reviews_today,
            avg_score: (avg_score * 10.0).round() / 10.0, streak_days, active_days,
            stage, stage_progress, next_stage,
        })
    }

    pub fn save_snapshot(&self, created_at: &str, kind: &str, payload: &str, svg: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO snapshots (created_at, kind, payload, svg) VALUES (?1, ?2, ?3, ?4)",
            params![created_at, kind, payload, svg],
        )?;
        Ok(self.conn.last_insert_rowid())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub id: i64,
    pub material_id: i64,
    pub material_title: String,
    pub days: i64,
    pub per_day: i64,
    pub start_date: String,
    pub created_at: String,
    pub units_total: i64,
    pub learned: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanToday {
    pub plan: Plan,
    /// 今天是计划的第几天（从 1 开始；超出计划天数时停在最后一天）
    pub day_index: i64,
    pub day_units: Vec<Card>,
    pub remaining_units: i64,
}

impl Store {
    pub fn insert_plan(&self, material_id: i64, days: i64, per_day: i64,
                       start_date: &str, created_at: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO plans (material_id, days, per_day, start_date, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![material_id, days, per_day, start_date, created_at],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    fn map_plan(r: &rusqlite::Row) -> rusqlite::Result<Plan> {
        Ok(Plan {
            id: r.get("id")?,
            material_id: r.get("material_id")?,
            material_title: r.get("material_title").unwrap_or_default(),
            days: r.get("days")?,
            per_day: r.get("per_day")?,
            start_date: r.get("start_date")?,
            created_at: r.get("created_at")?,
            units_total: r.get("units_total").unwrap_or(0),
            learned: r.get("learned").unwrap_or(0),
        })
    }

    const PLAN_SELECT: &'static str =
        "SELECT p.*, m.title AS material_title,
                (SELECT COUNT(*) FROM cards c WHERE c.material_id = p.material_id) AS units_total,
                (SELECT COUNT(*) FROM cards c WHERE c.material_id = p.material_id AND c.learned = 1) AS learned
         FROM plans p JOIN materials m ON m.id = p.material_id";

    pub fn list_plans(&self) -> Result<Vec<Plan>> {
        let sql = format!("{} ORDER BY p.id DESC", Self::PLAN_SELECT);
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |r| Self::map_plan(r))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_plan(&self, id: i64) -> Result<Option<Plan>> {
        let sql = format!("{} WHERE p.id = ?1", Self::PLAN_SELECT);
        Ok(self
            .conn
            .query_row(&sql, params![id], |r| Self::map_plan(r))
            .optional()?)
    }

    pub fn delete_plan(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM plans WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// 今天该学哪些单元：按计划天数把材料顺序切开，取第 n 天那一段。
    pub fn plan_today(&self, id: i64, today: &str) -> Result<Option<PlanToday>> {
        let Some(plan) = self.get_plan(id)? else {
            return Ok(None);
        };
        let elapsed = match (
            chrono::NaiveDate::parse_from_str(&plan.start_date, "%Y-%m-%d"),
            chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d"),
        ) {
            (Ok(start), Ok(now)) => (now - start).num_days() + 1,
            _ => 1,
        };
        let day_index = elapsed.clamp(1, plan.days.max(1));
        let cards = self.list_cards(Some(plan.material_id))?;
        let start = ((day_index - 1) * plan.per_day).max(0) as usize;
        let day_units: Vec<Card> = cards
            .iter()
            .skip(start)
            .take(plan.per_day.max(1) as usize)
            .cloned()
            .collect();
        let remaining_units = cards.iter().filter(|card| !card.learned).count() as i64;
        Ok(Some(PlanToday {
            plan,
            day_index,
            day_units,
            remaining_units,
        }))
    }

    // ── 测验 ───────────────────────────────────────────────

    /// 建一份卷子：题目位置先占好（只有题型和分值），题干由模型分批填。
    pub fn create_exam(&self, title: &str, material_id: Option<i64>, scope: &[i64],
                       kinds: &[String], points: &[f64], created_at: &str,
                       cards: &[(i64, String)]) -> Result<i64> {
        let full: f64 = points.iter().sum();
        self.conn.execute(
            "INSERT INTO exams (title, material_id, scope, slot_kinds, total, full_score, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'draft', ?7)",
            params![title, material_id, serde_json::to_string(scope)?,
                    serde_json::to_string(kinds)?, kinds.len() as i64, full, created_at],
        )?;
        let exam_id = self.conn.last_insert_rowid();
        for (i, kind) in kinds.iter().enumerate() {
            let (card_id, card_title) = cards
                .get(i % cards.len().max(1))
                .cloned()
                .unwrap_or((0, String::new()));
            self.conn.execute(
                "INSERT INTO exam_items (exam_id, seq, kind, card_id, card_title, points)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![exam_id, i as i64 + 1, kind,
                        if card_id == 0 { None } else { Some(card_id) },
                        card_title, points[i]],
            )?;
        }
        Ok(exam_id)
    }

    pub fn pending_exam_items(&self, exam_id: i64, limit: i64) -> Result<Vec<ExamItem>> {
        let sql = format!(
            "{} WHERE exam_id = ?1 AND stem = '' ORDER BY seq ASC LIMIT ?2",
            Self::EXAM_ITEM_SELECT
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![exam_id, limit], Self::map_exam_item)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn fill_exam_item(&self, id: i64, stem: &str, options: &[String], answer: &str, explain: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE exam_items SET stem = ?2, options = ?3, answer = ?4, explain = ?5 WHERE id = ?1",
            params![id, stem, serde_json::to_string(options)?, answer, explain],
        )?;
        Ok(())
    }

    pub fn exam_items(&self, exam_id: i64) -> Result<Vec<ExamItem>> {
        let sql = format!("{} WHERE exam_id = ?1 ORDER BY seq ASC", Self::EXAM_ITEM_SELECT);
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![exam_id], Self::map_exam_item)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn get_exam(&self, id: i64) -> Result<Option<Exam>> {
        let sql = format!("{} WHERE id = ?1", Self::EXAM_SELECT);
        Ok(self.conn.query_row(&sql, params![id], Self::map_exam).optional()?)
    }

    pub fn list_exams(&self, limit: i64) -> Result<Vec<Exam>> {
        let sql = format!("{} ORDER BY id DESC LIMIT ?1", Self::EXAM_SELECT);
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit], Self::map_exam)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn save_exam_answer(&self, item_id: i64, answer: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE exam_items SET user_answer = ?2 WHERE id = ?1",
            params![item_id, answer],
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn grade_exam_item(&self, item_id: i64, score: f64, verdict: &str, feedback: &str,
                           missed: &[String], suggestion: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE exam_items SET score = ?2, verdict = ?3, feedback = ?4, missed = ?5, suggestion = ?6
             WHERE id = ?1",
            params![item_id, score, verdict, feedback,
                    serde_json::to_string(missed)?, suggestion],
        )?;
        Ok(())
    }

    pub fn finish_exam(&self, id: i64, score: f64, correct: i64, report: &str, finished_at: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE exams SET status = 'done', score = ?2, correct = ?3, report = ?4, finished_at = ?5
             WHERE id = ?1",
            params![id, score, correct, report, finished_at],
        )?;
        Ok(())
    }

    pub fn delete_exam(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM exam_items WHERE exam_id = ?1", params![id])?;
        self.conn.execute("DELETE FROM exams WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// 把测验要点写回卡片（去重、最多留 8 条），学习时能看到。
    pub fn append_card_note(&self, card_id: i64, note: &str) -> Result<()> {
        let note = note.trim();
        if note.is_empty() {
            return Ok(());
        }
        let current: String = self.conn
            .query_row("SELECT exam_notes FROM cards WHERE id = ?1", params![card_id], |r| r.get(0))
            .unwrap_or_else(|_| "[]".to_string());
        let mut notes = parse_list(&current);
        if notes.iter().any(|item| item == note) {
            return Ok(());
        }
        notes.push(note.to_string());
        if notes.len() > 8 {
            let drop = notes.len() - 8;
            notes.drain(0..drop);
        }
        self.conn.execute(
            "UPDATE cards SET exam_notes = ?2 WHERE id = ?1",
            params![card_id, serde_json::to_string(&notes)?],
        )?;
        Ok(())
    }

    const EXAM_SELECT: &'static str =
        "SELECT e.*, (SELECT COUNT(*) FROM exam_items i WHERE i.exam_id = e.id AND i.stem <> '') AS made
         FROM exams e";

    const EXAM_ITEM_SELECT: &'static str = "SELECT * FROM exam_items";

    fn map_exam(r: &rusqlite::Row<'_>) -> rusqlite::Result<Exam> {
        Ok(Exam {
            id: r.get("id")?,
            title: r.get("title")?,
            material_id: r.get("material_id")?,
            total: r.get("total")?,
            full_score: r.get("full_score")?,
            status: r.get("status")?,
            score: r.get("score")?,
            correct: r.get("correct")?,
            report: r.get("report")?,
            created_at: r.get("created_at")?,
            finished_at: r.get("finished_at")?,
            made: r.get("made").unwrap_or(0),
        })
    }

    fn map_exam_item(r: &rusqlite::Row<'_>) -> rusqlite::Result<ExamItem> {
        let options: String = r.get("options").unwrap_or_default();
        let missed: String = r.get("missed").unwrap_or_default();
        Ok(ExamItem {
            id: r.get("id")?,
            exam_id: r.get("exam_id")?,
            seq: r.get("seq")?,
            kind: r.get("kind")?,
            card_id: r.get("card_id")?,
            card_title: r.get("card_title").unwrap_or_default(),
            stem: r.get("stem").unwrap_or_default(),
            options: parse_list(&options),
            answer: r.get("answer").unwrap_or_default(),
            explain: r.get("explain").unwrap_or_default(),
            points: r.get("points").unwrap_or(1.0),
            user_answer: r.get("user_answer").unwrap_or_default(),
            score: r.get("score")?,
            verdict: r.get("verdict").unwrap_or_default(),
            feedback: r.get("feedback").unwrap_or_default(),
            missed: parse_list(&missed),
            suggestion: r.get("suggestion").unwrap_or_default(),
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Exam {
    pub id: i64,
    pub title: String,
    pub material_id: Option<i64>,
    pub total: i64,
    pub full_score: f64,
    pub status: String,
    pub score: Option<f64>,
    pub correct: i64,
    pub report: String,
    pub created_at: String,
    pub finished_at: Option<String>,
    /// 已经出好的题数（stem 非空）
    pub made: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExamItem {
    pub id: i64,
    pub exam_id: i64,
    pub seq: i64,
    pub kind: String,
    pub card_id: Option<i64>,
    pub card_title: String,
    pub stem: String,
    pub options: Vec<String>,
    pub answer: String,
    pub explain: String,
    pub points: f64,
    pub user_answer: String,
    pub score: Option<f64>,
    pub verdict: String,
    pub feedback: String,
    pub missed: Vec<String>,
    pub suggestion: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewCard {
    #[serde(default)]
    pub section: String,
    pub title: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default, alias = "key_points", alias = "points", deserialize_with = "string_or_vec")]
    pub key_points: Vec<String>,
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub answer: String,
    #[serde(default, alias = "tag", deserialize_with = "string_or_vec")]
    pub tags: Vec<String>,
}

/// 模型为学习单元出的考点（正文原文不入这里，正文导入时就已有）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CardDigest {
    #[serde(default)]
    pub summary: String,
    #[serde(default, alias = "key_points", alias = "points", deserialize_with = "string_or_vec")]
    pub key_points: Vec<String>,
    #[serde(default)]
    pub question: String,
    #[serde(default)]
    pub answer: String,
}

/// 模型有时把数组写成单个字符串，两种都收下。
fn string_or_vec<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        List(Vec<String>),
        One(String),
        Numbers(Vec<f64>),
    }
    Ok(match Raw::deserialize(deserializer)? {
        Raw::List(list) => list,
        Raw::One(text) => {
            if text.trim().is_empty() {
                Vec::new()
            } else {
                vec![text]
            }
        }
        Raw::Numbers(nums) => nums.into_iter().map(|n| n.to_string()).collect(),
    })
}

/// 学习阶段划分：入门 → 铺开 → 巩固 → 精通。
fn stage_of(learned: i64, cards: i64, avg_score: f64) -> (String, f64, String) {
    let coverage = if cards > 0 { learned as f64 / cards as f64 } else { 0.0 };
    if cards == 0 {
        return ("未开始".into(), 0.0, "入门".into());
    }
    if coverage < 0.3 {
        ("入门".into(), coverage / 0.3, "铺开".into())
    } else if coverage < 0.7 {
        ("铺开".into(), (coverage - 0.3) / 0.4, "巩固".into())
    } else if coverage < 1.0 || avg_score < 8.0 {
        ("巩固".into(), ((coverage - 0.7) / 0.3).clamp(0.0, 1.0), "精通".into())
    } else {
        ("精通".into(), 1.0, "保持".into())
    }
}

fn add_days(date: &str, days: i64) -> String {
    match chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        Ok(d) => (d + chrono::Duration::days(days)).format("%Y-%m-%d").to_string(),
        Err(_) => date.to_string(),
    }
}

/// 连续学习天数（今天或昨天有记录即算延续）。
fn streak(days_desc: &[String], today: &str) -> i64 {
    if days_desc.is_empty() {
        return 0;
    }
    let set: std::collections::HashSet<&str> = days_desc.iter().map(|s| s.as_str()).collect();
    let start = if set.contains(today) {
        today.to_string()
    } else {
        let y = add_days(today, -1);
        if set.contains(y.as_str()) { y } else { return 0; }
    };
    let mut count = 0i64;
    let mut cursor = start;
    while set.contains(cursor.as_str()) {
        count += 1;
        cursor = add_days(&cursor, -1);
    }
    count
}