//! 致知（Zhizhi）命令层：前端通过 invoke 调这些命令。

pub mod autostart;
pub mod graph;
pub mod ingest;
pub mod llm;
pub mod notify;
pub mod settings;
pub mod srs;
pub mod store;

use std::path::PathBuf;
use std::sync::{Mutex, RwLock};

use serde::{Deserialize, Serialize};
use tauri::{Manager, State};

use futures::StreamExt;

use settings::{ProxyInfo, Settings};
use store::{Card, NewCard, Stats, Store};

pub struct AppState {
    store: Mutex<Store>,
    /// 运行期设置（接口、代理），保存在数据目录的 config.json。
    settings: RwLock<Settings>,
    settings_path: PathBuf,
    llm: RwLock<llm::LlmClient>,
    http: RwLock<reqwest::Client>,
    data_dir: PathBuf,
}

fn err_str<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn now_iso() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    data_dir: String,
    llm_ready: bool,
    model: String,
    version: String,
}

fn app_info_of(state: &AppState) -> AppInfo {
    let llm = state.llm.read().unwrap();
    AppInfo {
        data_dir: state.data_dir.display().to_string(),
        llm_ready: llm.ready(),
        model: llm.model_name().to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

#[tauri::command]
fn app_info(state: State<'_, AppState>) -> AppInfo {
    app_info_of(&state)
}

// ── 设置：模型接口与代理 ─────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TestResult {
    ok: bool,
    message: String,
    detail: String,
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.read().unwrap().clone()
}

/// 保存设置并立刻重建 HTTP 与模型客户端（不用重启应用）。
#[tauri::command]
fn save_settings(state: State<'_, AppState>, settings: Settings) -> Result<AppInfo, String> {
    let llm = llm::LlmClient::from_settings(&settings).map_err(err_str)?;
    let http = settings.build_http(60).map_err(err_str)?;
    settings.save(&state.settings_path).map_err(err_str)?;
    *state.llm.write().unwrap() = llm;
    *state.http.write().unwrap() = http;
    *state.settings.write().unwrap() = settings;
    Ok(app_info_of(&state))
}

/// 检测代理：环境变量、Windows 系统代理、本地常见端口。
#[tauri::command]
async fn detect_proxy(state: State<'_, AppState>) -> Result<ProxyInfo, String> {
    let settings = state.settings.read().unwrap().clone();
    tauri::async_runtime::spawn_blocking(move || settings::detect(&settings))
        .await
        .map_err(err_str)
}

/// 用当前设置发一次最小请求，验证 Key / 地址 / 代理是否通。
#[tauri::command]
async fn test_llm(state: State<'_, AppState>) -> Result<TestResult, String> {
    let llm = state.llm.read().unwrap().clone();
    let started = std::time::Instant::now();
    Ok(match llm.ping().await {
        Ok(reply) => TestResult {
            ok: true,
            message: format!("接口正常 · {:.1} 秒", started.elapsed().as_secs_f32()),
            detail: format!("{} 回复：{reply}", llm.model_name()),
        },
        Err(error) => TestResult {
            ok: false,
            message: "接口调用失败".to_string(),
            detail: error.to_string(),
        },
    })
}

/// 用当前（带代理的）网络设置抓一个网址，验证代理是否真的生效。
#[tauri::command]
async fn test_fetch(state: State<'_, AppState>, url: String) -> Result<TestResult, String> {
    let http = state.http.read().unwrap().clone();
    let target = url.trim().to_string();
    if target.is_empty() {
        return Err("先填一个要测试的网址".into());
    }
    let started = std::time::Instant::now();
    let outcome = ingest::fetch_page(&http, &target).await;
    let ms = started.elapsed().as_millis();
    Ok(match outcome {
        Ok((final_url, title, text)) => TestResult {
            ok: true,
            message: format!("{ms} 毫秒抓到 {} 字", text.chars().count()),
            detail: format!("{final_url}｜{title}"),
        },
        Err(error) => TestResult {
            ok: false,
            message: format!("{ms} 毫秒后失败"),
            detail: error.to_string(),
        },
    })
}

// ── 摄取 ─────────────────────────────────────────────────

/// 智能导入：给了链接就抓，给了主题就搜；两者都先交给模型整理，脚本兜底。
#[tauri::command]
async fn ingest_query(state: State<'_, AppState>, text: String) -> Result<ingest::Ingested, String> {
    let query = text.trim().to_string();
    if query.is_empty() {
        return Err("先粘贴链接，或者写一个想学的主题".into());
    }
    let http = state.http.read().unwrap().clone();
    let llm = state.llm.read().unwrap().clone();
    if query.starts_with("http://") || query.starts_with("https://") {
        ingest::smart_from_url(&http, &llm, &query).await.map_err(err_str)
    } else {
        if query.chars().count() < 4 {
            return Err("主题至少 4 个字，或者直接给一个链接".into());
        }
        ingest::smart_from_topic(&http, &llm, &query).await.map_err(err_str)
    }
}

#[tauri::command]
async fn ingest_bytes(
    state: State<'_, AppState>,
    name: String,
    bytes: Vec<u8>,
) -> Result<ingest::Ingested, String> {
    if bytes.is_empty() {
        return Err("文件是空的".into());
    }
    let llm = state.llm.read().unwrap().clone();
    let item = ingest::from_bytes(&name, &bytes).map_err(err_str)?;
    Ok(ingest::polish_ingested(&llm, item).await)
}

#[tauri::command]
fn ingest_text(title: String, text: String) -> Result<ingest::Ingested, String> {
    ingest::Ingested::from_text(title, text).map_err(err_str)
}

// ── 材料与卡片 ───────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MaterialOut {
    material_id: i64,
    units: i64,
}

/// 建档后立刻切分学习单元：正文原文入库，不用等模型，导入完就能开始学。
#[tauri::command]
fn create_material(state: State<'_, AppState>, item: ingest::Ingested) -> Result<MaterialOut, String> {
    let store = state.store.lock().unwrap();
    let material_id = store
        .insert_material(&item.title, &item.source, &item.kind, item.chars, &item.text, &now_iso())
        .map_err(err_str)?;

    let units = ingest::split_units(&item.text, &item.kind);
    let cards: Vec<NewCard> = units
        .iter()
        .map(|unit| NewCard {
            section: unit.section.clone(),
            title: unit.title.clone(),
            content: unit.content.clone(),
            ..Default::default()
        })
        .collect();
    let count = store.insert_cards(material_id, &cards, &today()).map_err(err_str)?;
    Ok(MaterialOut { material_id, units: count })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PolishOut {
    polished: i64,
    remaining: i64,
    done: bool,
    /// 本轮第一处失败原因；成功时为 None。前端据此把「0 份考点」讲清楚。
    error: Option<String>,
}

/// 给还没有考点的学习单元补考点（要点 / 考题 / 答案）。
/// 分步执行：整仓材料有一百多个单元，一次跑完既看不到进度也容易超时。
#[tauri::command]
async fn polish_units_step(
    state: State<'_, AppState>,
    material_id: i64,
    limit: usize,
) -> Result<PolishOut, String> {
    let pending: Vec<(i64, String, String)> = {
        let store = state.store.lock().unwrap();
        store
            .list_cards(Some(material_id))
            .map_err(err_str)?
            .into_iter()
            .filter(|card| card.question.trim().is_empty())
            .take(limit.max(1))
            .map(|card| (card.id, card.title, card.content))
            .collect()
    };

    if pending.is_empty() {
        return Ok(PolishOut {
            polished: 0,
            remaining: 0,
            done: true,
            error: None,
        });
    }

    let llm = state.llm.read().unwrap().clone();
    let results: Vec<(i64, anyhow::Result<store::CardDigest>)> =
        futures::stream::iter(pending.into_iter())
            .map(|(id, title, content)| {
                let llm = llm.clone();
                async move { (id, llm.digest_unit(&title, &content).await) }
            })
            .buffer_unordered(4)
            .collect()
            .await;

    let store = state.store.lock().unwrap();
    let mut polished = 0i64;
    let mut first_error: Option<String> = None;
    for (id, result) in results {
        match result {
            Ok(digest) => {
                store.update_digest(id, &digest).map_err(err_str)?;
                polished += 1;
            }
            Err(e) => {
                eprintln!("[考点] 生成失败：{e}");
                if first_error.is_none() {
                    first_error = Some(e.to_string());
                }
            }
        }
    }
    let remaining = store
        .list_cards(Some(material_id))
        .map_err(err_str)?
        .into_iter()
        .filter(|card| card.question.trim().is_empty())
        .count() as i64;
    Ok(PolishOut {
        polished,
        remaining,
        done: remaining == 0,
        error: first_error,
    })
}

#[tauri::command]
fn list_materials(state: State<'_, AppState>) -> Result<Vec<store::Material>, String> {
    state.store.lock().unwrap().list_materials().map_err(err_str)
}

// ── 开机自启 ─────────────────────────────────────────────

/// 直接读注册表当前状态，不缓存：用户也可以在任务管理器里自己关掉。
#[tauri::command]
fn autostart_on() -> Result<bool, String> {
    autostart::enabled().map_err(err_str)
}

#[tauri::command]
fn set_autostart(enabled: bool) -> Result<bool, String> {
    autostart::set(enabled).map_err(err_str)?;
    autostart::enabled().map_err(err_str)
}

// ── 学习提醒 ─────────────────────────────────────────────

#[tauri::command]
fn set_notify(state: State<'_, AppState>, enabled: bool) -> Result<bool, String> {
    let mut settings = state.settings.write().unwrap();
    settings.notify = enabled;
    settings.save(&state.settings_path).map_err(err_str)?;
    Ok(settings.notify)
}

// ── 学习计划 ─────────────────────────────────────────────

/// 给某份材料排一个计划：把单元按天数均分，每天学一段。
#[tauri::command]
fn create_plan(state: State<'_, AppState>, material_id: i64, days: i64) -> Result<store::Plan, String> {
    let store = state.store.lock().unwrap();
    let units = store.list_cards(Some(material_id)).map_err(err_str)?.len() as i64;
    if units == 0 {
        return Err("这份材料还没有学习单元".into());
    }
    let days = days.clamp(1, 365);
    let per_day = ((units as f64 / days as f64).ceil() as i64).max(1);
    let id = store
        .insert_plan(material_id, days, per_day, &today(), &now_iso())
        .map_err(err_str)?;
    store
        .get_plan(id)
        .map_err(err_str)?
        .ok_or_else(|| "计划创建失败".to_string())
}

#[tauri::command]
fn list_plans(state: State<'_, AppState>) -> Result<Vec<store::Plan>, String> {
    state.store.lock().unwrap().list_plans().map_err(err_str)
}

#[tauri::command]
fn delete_plan(state: State<'_, AppState>, plan_id: i64) -> Result<(), String> {
    state.store.lock().unwrap().delete_plan(plan_id).map_err(err_str)
}

/// 今天的计划进度：按实际进度算到第几段、这一段学哪些单元、还剩多少没学。
#[tauri::command]
fn plan_today(state: State<'_, AppState>, plan_id: i64) -> Result<Option<store::PlanToday>, String> {
    state.store.lock().unwrap().plan_today(plan_id).map_err(err_str)
}

#[tauri::command]
fn rename_material(state: State<'_, AppState>, material_id: i64, title: String) -> Result<(), String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("标题不能为空".into());
    }
    state
        .store
        .lock()
        .unwrap()
        .rename_material(material_id, title)
        .map_err(err_str)
}

/// 把材料正文导出成 Markdown 文件，返回落盘路径。
#[tauri::command]
fn export_material(state: State<'_, AppState>, material_id: i64) -> Result<String, String> {
    let (title, source, body) = {
        let store = state.store.lock().unwrap();
        let material = store
            .list_materials()
            .map_err(err_str)?
            .into_iter()
            .find(|m| m.id == material_id)
            .ok_or("材料不存在")?;
        (
            material.title,
            material.source,
            store.material_text(material_id).map_err(err_str)?,
        )
    };
    let dir = state.data_dir.join("exports");
    std::fs::create_dir_all(&dir).map_err(err_str)?;
    let safe: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    let path = dir.join(format!("{safe}.md"));
    let content = format!("# {title}\n\n> 来源：{source}\n\n---\n\n{body}\n");
    std::fs::write(&path, content).map_err(err_str)?;
    Ok(path.display().to_string())
}

#[tauri::command]
fn delete_material(state: State<'_, AppState>, material_id: i64) -> Result<(), String> {
    state.store.lock().unwrap().delete_material(material_id).map_err(err_str)
}

#[tauri::command]
fn list_cards(state: State<'_, AppState>, material_id: Option<i64>) -> Result<Vec<Card>, String> {
    state.store.lock().unwrap().list_cards(material_id).map_err(err_str)
}

#[tauri::command]
fn mark_learned(state: State<'_, AppState>, card_id: i64, learned: bool) -> Result<(), String> {
    state.store.lock().unwrap().mark_learned(card_id, learned).map_err(err_str)
}

// ── 学习与复习 ───────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Queue {
    due: Vec<Card>,
    /// 今天计划要学、还没学的单元
    scheduled: Vec<Card>,
    fresh: Vec<Card>,
    /// 未来 7 天每天的到期量，首页画复习节奏
    rhythm: Vec<(String, i64)>,
    stats: Stats,
}

#[tauri::command]
fn today_queue(state: State<'_, AppState>) -> Result<Queue, String> {
    let store = state.store.lock().unwrap();
    let day = today();
    Ok(Queue {
        due: store.due_cards(&day, 200).map_err(err_str)?,
        scheduled: store.scheduled_cards(100).map_err(err_str)?,
        fresh: store.learning_cards(60).map_err(err_str)?,
        rhythm: store.due_forecast(&day, 6).map_err(err_str)?,
        stats: store.stats(&day).map_err(err_str)?,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GradeOut {
    score: f64,
    rating: i64,
    feedback: String,
    missed: Vec<String>,
    suggestion: String,
}

#[tauri::command]
async fn grade_answer(
    state: State<'_, AppState>,
    card_id: i64,
    answer: String,
) -> Result<GradeOut, String> {
    let card = {
        let store = state.store.lock().unwrap();
        store.get_card(card_id).map_err(err_str)?.ok_or("卡片不存在")?
    };
    if answer.trim().is_empty() {
        return Err("先写下你的回答再提交".into());
    }
    let llm = state.llm.read().unwrap().clone();
    let result = llm
        .grade(&card.question, &card.answer, &answer)
        .await
        .map_err(err_str)?;
    Ok(GradeOut {
        score: result.score,
        rating: srs::rating_from_score(result.score) as i64,
        feedback: result.feedback,
        missed: result.missed,
        suggestion: result.suggestion,
    })
}

#[tauri::command]
fn submit_review(
    state: State<'_, AppState>,
    card_id: i64,
    rating: i64,
    score: Option<f64>,
    answer: Option<String>,
    mode: Option<String>,
) -> Result<Card, String> {
    let store = state.store.lock().unwrap();
    let card = store.get_card(card_id).map_err(err_str)?.ok_or("卡片不存在")?;
    let day = today();
    let memory = if card.stability > 0.0 {
        Some((card.stability as f32, card.difficulty as f32))
    } else {
        None
    };
    let scheduled = srs::schedule(memory, rating.clamp(1, 4) as u32, &day).map_err(err_str)?;
    let lapses = card.lapses + i64::from(rating <= 1);
    let now = now_iso();
    store
        .apply_schedule(
            card.id,
            scheduled.state,
            scheduled.stability as f64,
            scheduled.difficulty as f64,
            card.reps + 1,
            lapses,
            &now,
            &scheduled.due,
            true,
        )
        .map_err(err_str)?;
    store
        .insert_review(
            card.id,
            &now,
            rating,
            0,
            scheduled.interval_days,
            score,
            &answer.unwrap_or_default(),
            &mode.unwrap_or_else(|| "review".to_string()),
        )
        .map_err(err_str)?;
    store
        .get_card(card_id)
        .map_err(err_str)?
        .ok_or_else(|| "卡片不存在".to_string())
}

#[tauri::command]
fn stats(state: State<'_, AppState>) -> Result<Stats, String> {
    state.store.lock().unwrap().stats(&today()).map_err(err_str)
}

#[tauri::command]
fn recent_reviews(
    state: State<'_, AppState>,
    limit: Option<i64>,
) -> Result<Vec<store::ReviewRecord>, String> {
    state
        .store
        .lock()
        .unwrap()
        .recent_reviews(limit.unwrap_or(20))
        .map_err(err_str)
}

// ── 看板 ─────────────────────────────────────────────────

#[tauri::command]
fn render_knowledge_map(state: State<'_, AppState>, material_id: Option<i64>) -> Result<String, String> {
    let store = state.store.lock().unwrap();
    let day = today();
    let cards = store.list_cards(material_id).map_err(err_str)?;
    let title = match material_id {
        Some(id) => store
            .list_materials()
            .map_err(err_str)?
            .into_iter()
            .find(|m| m.id == id)
            .map(|m| m.title)
            .unwrap_or_else(|| format!("材料 #{id}")),
        None => "全部材料".to_string(),
    };
    Ok(graph::knowledge_map(&title, &cards, &day))
}

#[tauri::command]
fn render_review_plan(state: State<'_, AppState>) -> Result<String, String> {
    let store = state.store.lock().unwrap();
    let day = today();
    let due = store.due_cards(&day, 200).map_err(err_str)?;
    let forecast = store.due_forecast(&day, 30).map_err(err_str)?;
    Ok(graph::review_plan(&forecast, &day, &due))
}

#[tauri::command]
fn render_stage_guide(state: State<'_, AppState>) -> Result<String, String> {
    let store = state.store.lock().unwrap();
    let day = today();
    let cards = store.list_cards(None).map_err(err_str)?;
    let info = store.stats(&day).map_err(err_str)?;
    Ok(graph::stage_guide(&info, &cards, &day))
}

#[tauri::command]
fn export_svg(state: State<'_, AppState>, name: String, svg: String) -> Result<String, String> {
    let dir = state.data_dir.join("exports");
    std::fs::create_dir_all(&dir).map_err(err_str)?;
    let safe: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    let path = dir.join(format!(
        "{}-{}.svg",
        safe,
        chrono::Local::now().format("%Y%m%d-%H%M%S")
    ));
    std::fs::write(&path, svg).map_err(err_str)?;
    Ok(path.display().to_string())
}

#[tauri::command]
fn reveal_path(path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(format!("/select,{path}"))
            .spawn()
            .map_err(err_str)?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
    }
    Ok(())
}

#[tauri::command]
fn save_snapshot(state: State<'_, AppState>, kind: String, svg: String) -> Result<i64, String> {
    state
        .store
        .lock()
        .unwrap()
        .save_snapshot(&now_iso(), &kind, "{}", &svg)
        .map_err(err_str)
}

// ── 托盘 ─────────────────────────────────────────────────

/// 托盘菜单只有两项：把窗口叫回来，或真正退出。
/// 左键点图标直接显示窗口，右键出菜单。
fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    let show = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出 致知", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("致知 · 学以致知，孜孜不倦")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => reveal_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                reveal_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

fn reveal_main(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}


// ── 测验（出卷 / 判分 / 复习要点）─────────────────────────

#[cfg(test)]
mod exam_tests {
    use super::*;

    #[test]
    fn objective_grading_covers_every_kind() {
        // 单选
        let (score, verdict, ..) = grade_objective("single", "B", "b", 2.0);
        assert_eq!((score, verdict), (2.0, "correct"));
        let (score, verdict, ..) = grade_objective("single", "B", "C", 2.0);
        assert_eq!((score, verdict), (0.0, "wrong"));
        // 多选：全对、漏选给部分分、错选零分
        let (score, verdict, ..) = grade_objective("multi", "ACD", "DCA", 3.0);
        assert_eq!((score, verdict), (3.0, "correct"));
        let (score, verdict, ..) = grade_objective("multi", "ACD", "AC", 3.0);
        assert_eq!(verdict, "partial");
        assert!((score - 2.0).abs() < 0.01);
        let (score, verdict, ..) = grade_objective("multi", "ACD", "AB", 3.0);
        assert_eq!((score, verdict), (0.0, "wrong"));
        // 判断
        let (score, verdict, ..) = grade_objective("judge", "正确", "对", 1.0);
        assert_eq!((score, verdict), (1.0, "correct"));
        // 填空：多个可接受写法
        let (score, verdict, ..) = grade_objective("blank", "感知模块|Perception", " perception ", 2.0);
        assert_eq!((score, verdict), (2.0, "correct"));
        let (score, verdict, ..) = grade_objective("blank", "感知模块", "推理模块", 2.0);
        assert_eq!((score, verdict), (0.0, "wrong"));
    }

    fn unit_card(section: &str, seq: i64) -> Card {
        Card {
            id: seq + 1, material_id: 1, material_title: "材料".into(), seq,
            section: section.into(), title: format!("{section} 第 {seq} 单元"),
            content: String::new(), summary: String::new(), key_points: vec![],
            question: String::new(), answer: String::new(), tags: vec![],
            state: 0, stability: 0.0, difficulty: 0.0, reps: 0, lapses: 0,
            last_review: None, due: String::new(), learned: true, exam_notes: vec![],
        }
    }

    #[test]
    fn practice_picks_one_to_three_units_per_section() {
        let mut cards: Vec<Card> = Vec::new();
        for (index, section) in ["第一章", "第二章", "第三章"].iter().enumerate() {
            for n in 0..4 {
                cards.push(unit_card(section, (index * 4 + n) as i64));
            }
        }
        let (kinds, points, picked) = practice_slots(&cards);

        assert_eq!(kinds.len(), picked.len());
        assert_eq!(points.len(), picked.len());
        assert!((3..=9).contains(&picked.len()), "每章 1–3 题，实得 {}", picked.len());
        for section in ["第一章", "第二章", "第三章"] {
            assert!(picked.iter().any(|card| card.section == section), "{section} 没出题");
        }
        assert!(kinds.iter().any(|kind| kind == "short"), "至少要有一道简答");
    }

    #[test]
    fn slots_interleave_kinds_and_sum_points() {
        let plan = vec![
            ExamPlan { kind: "single".into(), count: 3 },
            ExamPlan { kind: "judge".into(), count: 1 },
            ExamPlan { kind: "case".into(), count: 1 },
        ];
        let (kinds, points) = build_slots(&plan);
        assert_eq!(kinds.len(), 5);
        assert_eq!(kinds[0], "single");
        assert_eq!(kinds[1], "judge");
        assert_eq!(kinds[2], "case");
        assert_eq!(points.iter().sum::<f64>(), 2.0 * 3.0 + 1.0 + 8.0);
    }
}


#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExamPlan {
    kind: String,
    count: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExamOut {
    exam_id: i64,
    total: i64,
    full_score: f64,
    cards: i64,
}

/// 各题型默认满分与展示顺序。
fn points_of(kind: &str) -> f64 {
    match kind {
        "single" => 2.0,
        "multi" => 3.0,
        "judge" => 1.0,
        "blank" => 2.0,
        "short" => 5.0,
        "case" => 8.0,
        _ => 2.0,
    }
}

/// 把各题型的数量打散成交替的题序，避免前 10 道全是单选。
fn build_slots(plan: &[ExamPlan]) -> (Vec<String>, Vec<f64>) {
    let mut left: Vec<(String, i64)> = plan
        .iter()
        .filter(|item| item.count > 0)
        .map(|item| (item.kind.clone(), item.count))
        .collect();
    let mut kinds = Vec::new();
    while left.iter().any(|(_, n)| *n > 0) {
        for (kind, count) in left.iter_mut() {
            if *count > 0 {
                kinds.push(kind.clone());
                *count -= 1;
            }
        }
    }
    let points = kinds.iter().map(|kind| points_of(kind)).collect();
    (kinds, points)
}

/// 按抽题策略挑单元；平均取样，别把整卷都压在同一章。
fn pick_cards(cards: &[Card], strategy: &str, needed: usize) -> Vec<Card> {
    let mut pool: Vec<Card> = match strategy {
        "learned" => cards.iter().filter(|c| c.learned).cloned().collect(),
        "weak" => {
            let mut learned: Vec<Card> = cards.iter().filter(|c| c.learned).cloned().collect();
            learned.sort_by(|a, b| a.stability.partial_cmp(&b.stability).unwrap_or(std::cmp::Ordering::Equal));
            learned.extend(cards.iter().filter(|c| !c.learned).cloned());
            learned
        }
        _ => cards.to_vec(),
    };
    if pool.is_empty() {
        return pool;
    }
    if pool.len() <= needed {
        return pool;
    }
    let step = pool.len() as f64 / needed as f64;
    let mut picked = Vec::new();
    for i in 0..needed {
        let index = ((i as f64) * step) as usize;
        if let Some(card) = pool.get(index.min(pool.len() - 1)) {
            picked.push(card.clone());
        }
    }
    pool = picked;
    pool
}

#[tauri::command]
fn create_exam(
    state: State<'_, AppState>,
    material_id: Option<i64>,
    strategy: String,
    plan: Vec<ExamPlan>,
    title: Option<String>,
) -> Result<ExamOut, String> {
    let store = state.store.lock().unwrap();
    let (kinds, points) = build_slots(&plan);
    if kinds.is_empty() {
        return Err("至少选一种题型".into());
    }
    let all = store.list_cards(material_id).map_err(err_str)?;
    if all.is_empty() {
        return Err("这份范围里还没有学习单元".into());
    }
    let total = kinds.len();
    let needed = ((total as f64) / 2.0).ceil() as usize;
    let picked = pick_cards(&all, &strategy, needed.max(1));
    if picked.is_empty() {
        return Err("这个抽题范围里没有可用的单元，换个范围或策略再试".into());
    }
    let pairs: Vec<(i64, String)> = picked.iter().map(|c| (c.id, c.title.clone())).collect();
    let scope: Vec<i64> = picked.iter().map(|c| c.id).collect();
    let name = title.unwrap_or_else(|| {
        let base = if material_id.is_some() {
            picked.first().map(|c| c.material_title.clone()).unwrap_or_default()
        } else {
            "全部素材库".to_string()
        };
        format!("{base} · {total} 题")
    });
    let exam_id = store
        .create_exam(&name, material_id, &scope, &kinds, &points, &now_iso(), &pairs, "quiz")
        .map_err(err_str)?;
    Ok(ExamOut {
        exam_id,
        total: total as i64,
        full_score: points.iter().sum(),
        cards: pairs.len() as i64,
    })
}

/// 练习页的随机出题：按章节分组，每章 1–3 道，题型随机，至少留一道简答。
fn practice_slots(cards: &[Card]) -> (Vec<String>, Vec<f64>, Vec<Card>) {
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15)
        | 1;
    let mut roll = |bound: usize| -> usize {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 33) as usize % bound.max(1)
    };

    let mut groups: Vec<(String, Vec<Card>)> = Vec::new();
    for card in cards {
        let key = if card.section.trim().is_empty() {
            "未分类".to_string()
        } else {
            card.section.clone()
        };
        match groups.iter_mut().find(|(name, _)| *name == key) {
            Some((_, list)) => list.push(card.clone()),
            None => groups.push((key, vec![card.clone()])),
        }
    }

    let mut picked: Vec<Card> = Vec::new();
    for (_, list) in groups {
        let take = (1 + roll(3)).min(list.len());
        let mut pool = list;
        for _ in 0..take {
            let index = roll(pool.len());
            picked.push(pool.remove(index));
        }
    }

    const KINDS: [&str; 5] = ["single", "judge", "blank", "multi", "short"];
    let mut kinds: Vec<String> = picked
        .iter()
        .map(|_| KINDS[roll(KINDS.len())].to_string())
        .collect();
    if !kinds.iter().any(|kind| kind == "short") {
        let index = roll(kinds.len());
        kinds[index] = "short".to_string();
    }
    let points = kinds.iter().map(|kind| points_of(kind)).collect();
    (kinds, points, picked)
}

/// 随机出一份练习卷：只考已经学过的单元，题量与题型都由系统随机。
#[tauri::command]
fn create_practice(state: State<'_, AppState>, material_id: Option<i64>) -> Result<ExamOut, String> {
    let store = state.store.lock().unwrap();
    let learned: Vec<Card> = store
        .list_cards(material_id)
        .map_err(err_str)?
        .into_iter()
        .filter(|card| card.learned)
        .collect();
    if learned.is_empty() {
        return Err("还没有学过的单元，先去「学习」页学几节".into());
    }
    let (kinds, points, picked) = practice_slots(&learned);
    let pairs: Vec<(i64, String)> = picked.iter().map(|card| (card.id, card.title.clone())).collect();
    let scope: Vec<i64> = picked.iter().map(|card| card.id).collect();
    let total = kinds.len() as i64;
    let name = format!("练习 · {total} 题");
    let exam_id = store
        .create_exam(&name, material_id, &scope, &kinds, &points, &now_iso(), &pairs, "practice")
        .map_err(err_str)?;
    Ok(ExamOut {
        exam_id,
        total,
        full_score: points.iter().sum(),
        cards: pairs.len() as i64,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExamStepOut {
    made: i64,
    remaining: i64,
    done: bool,
    error: Option<String>,
}

/// 出题分步走：一次处理一小批单元，界面能看到进度，失败也能续。
#[tauri::command]
async fn exam_step(
    state: State<'_, AppState>,
    exam_id: i64,
    batch: usize,
) -> Result<ExamStepOut, String> {
    let pending: Vec<store::ExamItem> = {
        let store = state.store.lock().unwrap();
        store.pending_exam_items(exam_id, batch.max(1) as i64).map_err(err_str)?
    };

    if pending.is_empty() {
        let made = {
            let store = state.store.lock().unwrap();
            store.exam_items(exam_id).map_err(err_str)?.iter().filter(|i| !i.stem.is_empty()).count() as i64
        };
        return Ok(ExamStepOut { made, remaining: 0, done: true, error: None });
    }

    // 同一张卡上的题合并成一次调用，省次数也不容易串味
    let mut groups: Vec<(i64, Vec<store::ExamItem>)> = Vec::new();
    for item in pending {
        match item.card_id {
            Some(card_id) => match groups.iter_mut().find(|(id, _)| *id == card_id) {
                Some((_, list)) => list.push(item),
                None => groups.push((card_id, vec![item])),
            },
            None => groups.push((0, vec![item])),
        }
    }

    let llm = state.llm.read().unwrap().clone();
    let cards: Vec<Card> = {
        let store = state.store.lock().unwrap();
        store.list_cards(None).map_err(err_str)?
    };

    let mut first_error: Option<String> = None;
    let mut filled = 0i64;
    for (card_id, items) in groups {
        let card = cards.iter().find(|c| c.id == card_id).cloned();
        let Some(card) = card else { continue };
        let asks: Vec<(String, f64)> = items.iter().map(|i| (i.kind.clone(), i.points)).collect();
        match llm.make_questions(&card.title, &card.content, &asks).await {
            Ok(questions) => {
                let store = state.store.lock().unwrap();
                for (index, item) in items.iter().enumerate() {
                    let question = questions.get(index);
                    let (stem, options, answer, explain) = match question {
                        Some(q) if !q.stem.trim().is_empty() => (
                            q.stem.clone(),
                            q.options.clone(),
                            q.answer.clone(),
                            q.explain.clone(),
                        ),
                        _ => (
                            format!("（这道题没出出来，点「继续出题」重试）{}", card.title),
                            Vec::new(),
                            String::new(),
                            String::new(),
                        ),
                    };
                    store.fill_exam_item(item.id, &stem, &options, &answer, &explain).map_err(err_str)?;
                    filled += 1;
                }
            }
            Err(error) => {
                eprintln!("[出题] 失败：{error}");
                if first_error.is_none() {
                    first_error = Some(error.to_string());
                }
            }
        }
    }

    let (made, remaining) = {
        let store = state.store.lock().unwrap();
        let items = store.exam_items(exam_id).map_err(err_str)?;
        let made = items.iter().filter(|i| !i.stem.is_empty()).count() as i64;
        let remaining = items.iter().filter(|i| i.stem.is_empty()).count() as i64;
        (made, remaining)
    };
    let _ = filled;
    Ok(ExamStepOut { made, remaining, done: remaining == 0, error: first_error })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ExamPaper {
    exam: store::Exam,
    items: Vec<store::ExamItem>,
}

#[tauri::command]
fn get_exam(state: State<'_, AppState>, exam_id: i64) -> Result<ExamPaper, String> {
    let store = state.store.lock().unwrap();
    let exam = store.get_exam(exam_id).map_err(err_str)?.ok_or("卷子不存在")?;
    let items = store.exam_items(exam_id).map_err(err_str)?;
    Ok(ExamPaper { exam, items })
}

#[tauri::command]
fn list_exams(state: State<'_, AppState>) -> Result<Vec<store::Exam>, String> {
    state.store.lock().unwrap().list_exams(50).map_err(err_str)
}

#[tauri::command]
fn save_exam_answer(state: State<'_, AppState>, item_id: i64, answer: String) -> Result<(), String> {
    state.store.lock().unwrap().save_exam_answer(item_id, &answer).map_err(err_str)
}

#[tauri::command]
fn delete_exam(state: State<'_, AppState>, exam_id: i64) -> Result<(), String> {
    state.store.lock().unwrap().delete_exam(exam_id).map_err(err_str)
}

/// 客观题本地判：归一化后比对，不花模型调用。
fn grade_objective(kind: &str, answer: &str, user: &str, points: f64) -> (f64, &'static str, String, Vec<String>, String) {
    let norm = |text: &str| -> String {
        text.chars()
            .filter(|c| !c.is_whitespace() && !"，。、；：（）()[],.！？!?".contains(*c))
            .collect::<String>()
            .to_lowercase()
    };
    let letters = |text: &str| -> Vec<String> {
        text.chars()
            .filter(|c| c.is_ascii_alphabetic())
            .map(|c| c.to_uppercase().to_string())
            .collect()
    };
    let right_answer = format!("正确答案：{}", answer.trim());

    match kind {
        "single" => {
            let ok = norm(user) == norm(answer) || letters(user) == letters(answer);
            let score = if ok { points } else { 0.0 };
            let feedback = if ok { "选对了。".to_string() } else { format!("选错了。{right_answer}") };
            (score, if ok { "correct" } else { "wrong" }, feedback, vec![], String::new())
        }
        "multi" => {
            let mut want = letters(answer);
            let mut got = letters(user);
            want.sort();
            want.dedup();
            got.sort();
            got.dedup();
            if !want.is_empty() && got == want {
                (points, "correct", "全选对了。".to_string(), vec![], String::new())
            } else if !got.is_empty() && got.iter().all(|item| want.contains(item)) {
                let score = points * (got.len() as f64 / want.len() as f64);
                let missed: Vec<String> = want.iter().filter(|item| !got.contains(item)).cloned().collect();
                (
                    score,
                    "partial",
                    format!("漏选。{right_answer}"),
                    missed,
                    String::new(),
                )
            } else {
                (0.0, "wrong", format!("选错了。{right_answer}"), vec![], String::new())
            }
        }
        "judge" => {
            let judge = |text: &str| -> bool {
                let value = norm(text);
                value.contains("对") || value.contains("正确") || value == "true" || value == "t" || value == "yes"
            };
            let ok = judge(user) == judge(answer);
            let score = if ok { points } else { 0.0 };
            let feedback = if ok { "判断对了。".to_string() } else { format!("判断错了。{right_answer}") };
            (score, if ok { "correct" } else { "wrong" }, feedback, vec![], String::new())
        }
        _ => {
            let accepted: Vec<String> = answer.split('|').map(norm).filter(|s| !s.is_empty()).collect();
            let value = norm(user);
            let ok = !value.is_empty()
                && accepted.iter().any(|item| *item == value || (item.len() >= 2 && value.contains(item)));
            let score = if ok { points } else { 0.0 };
            let feedback = if ok { "填对了。".to_string() } else { format!("填错了。{right_answer}") };
            (score, if ok { "correct" } else { "wrong" }, feedback, vec![], String::new())
        }
    }
}

fn is_subjective(kind: &str) -> bool {
    matches!(kind, "short" | "case")
}

/// 交卷：客观题本地判，主观题交模型；然后汇总成绩、写复习要点、把要点写回卡片。
#[tauri::command]
async fn submit_exam(state: State<'_, AppState>, exam_id: i64) -> Result<ExamPaper, String> {
    let items = {
        let store = state.store.lock().unwrap();
        store.exam_items(exam_id).map_err(err_str)?
    };
    let cards: Vec<Card> = {
        let store = state.store.lock().unwrap();
        store.list_cards(None).map_err(err_str)?
    };
    let llm = state.llm.read().unwrap().clone();

    let mut results: Vec<(i64, f64, String, String, Vec<String>, String)> = Vec::new();
    // 主观题连参考答案一起先算好，后面并发判分时就不用再借 cards
    let mut subjective: Vec<(store::ExamItem, String)> = Vec::new();
    for item in &items {
        if is_subjective(&item.kind) {
            let card_answer = cards
                .iter()
                .find(|c| Some(c.id) == item.card_id)
                .map(|c| c.answer.clone())
                .unwrap_or_default();
            let reference = if !item.answer.trim().is_empty() { item.answer.clone() } else { card_answer };
            subjective.push((item.clone(), reference));
        } else {
            let (score, verdict, feedback, missed, suggestion) =
                grade_objective(&item.kind, &item.answer, &item.user_answer, item.points);
            results.push((item.id, score, verdict.to_string(), feedback, missed, suggestion));
        }
    }

    let graded: Vec<(i64, anyhow::Result<llm::ItemGrade>)> = futures::stream::iter(subjective.into_iter())
        .map(|(item, reference)| {
            let llm = llm.clone();
            async move {
                let result = llm
                    .grade_exam_answer(&item.stem, &reference, &item.user_answer, item.points)
                    .await;
                (item.id, result)
            }
        })
        .buffer_unordered(4)
        .collect()
        .await;

    for (id, result) in graded {
        match result {
            Ok(grade) => results.push((id, grade.score, grade.verdict, grade.feedback, grade.missed, grade.suggestion)),
            Err(error) => {
                eprintln!("[判分] 失败：{error}");
                results.push((id, 0.0, "wrong".to_string(), format!("判分失败：{error}"), vec![], String::new()));
            }
        }
    }

    let mut score = 0.0;
    let mut correct = 0i64;
    for (id, item_score, verdict, feedback, missed, suggestion) in &results {
        let store = state.store.lock().unwrap();
        store
            .grade_exam_item(*id, *item_score, verdict, feedback, missed, suggestion)
            .map_err(err_str)?;
        score += *item_score;
        if verdict == "correct" {
            correct += 1;
        }
    }

    // 复习要点：错题与部分对题的解析汇总，同时写回卡片
    let fresh_items = {
        let store = state.store.lock().unwrap();
        store.exam_items(exam_id).map_err(err_str)?
    };
    let mut report = String::new();
    let mut weak = 0;
    for item in &fresh_items {
        if item.verdict == "correct" || item.stem.is_empty() {
            continue;
        }
        weak += 1;
        report.push_str(&format!(
            "**{} · {} · 得分 {:.1}/{:.0}**\n\n{}\n\n- 正确答案：{}\n- 你的作答：{}\n",
            item.seq,
            llm::kind_label(&item.kind),
            item.score.unwrap_or(0.0),
            item.points,
            item.stem,
            item.answer,
            if item.user_answer.trim().is_empty() { "（没答）" } else { item.user_answer.trim() }
        ));
        if !item.explain.trim().is_empty() {
            report.push_str(&format!("- 解析：{}\n", item.explain.trim()));
        }
        if !item.suggestion.trim().is_empty() {
            report.push_str(&format!("- 要点：{}\n", item.suggestion.trim()));
        }
        if !item.missed.is_empty() {
            report.push_str(&format!("- 漏掉：{}\n", item.missed.join("；")));
        }
        report.push('\n');

        if let Some(card_id) = item.card_id {
            let note = if !item.suggestion.trim().is_empty() {
                item.suggestion.trim().to_string()
            } else if !item.explain.trim().is_empty() {
                item.explain.trim().to_string()
            } else {
                item.answer.trim().to_string()
            };
            let note: String = note.chars().take(160).collect();
            let store = state.store.lock().unwrap();
            store
                .append_card_note(card_id, &format!("{}（{} 测验）", note, crate::today().get(5..).unwrap_or("")))
                .map_err(err_str)?;
        }
    }

    let head = if weak == 0 {
        "全对，没有需要重点复习的题。\n".to_string()
    } else {
        format!("共 {} 道题需要回看。\n\n", weak)
    };
    let report = format!("## 复习要点\n\n{head}{report}");

    let store = state.store.lock().unwrap();
    store.finish_exam(exam_id, score, correct, report.trim(), &now_iso()).map_err(err_str)?;
    let exam = store.get_exam(exam_id).map_err(err_str)?.ok_or("卷子不存在")?;
    let items = store.exam_items(exam_id).map_err(err_str)?;
    Ok(ExamPaper { exam, items })
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let data_dir = match std::env::var("STUDYFORGE_DATA_DIR") {
                Ok(dir) if !dir.trim().is_empty() => PathBuf::from(dir.trim()),
                _ => app
                    .path()
                    .app_data_dir()
                    .expect("无法定位应用数据目录"),
            };
            std::fs::create_dir_all(&data_dir).ok();
            let store = Store::open(&data_dir.join("zhizhi.db")).expect("打开数据库失败");
            let settings_path = data_dir.join("config.json");
            let settings = Settings::load(&settings_path);
            let llm = llm::LlmClient::from_settings(&settings).expect("初始化模型客户端失败");
            let http = settings.build_http(60).expect("初始化 HTTP 客户端失败");
            app.manage(AppState {
                store: Mutex::new(store),
                settings: RwLock::new(settings),
                settings_path,
                llm: RwLock::new(llm),
                http: RwLock::new(http),
                data_dir,
            });
            setup_tray(app)?;
            notify::register_identity(&app.config().identifier, "致知");
            notify::spawn(app.handle().clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            // 关窗口只是收进托盘：开机自启时要能一直待在后台
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            get_settings,
            save_settings,
            detect_proxy,
            test_llm,
            test_fetch,
            ingest_query,
            ingest_bytes,
            ingest_text,
            create_material,
            polish_units_step,
            list_materials,
            autostart_on,
            set_autostart,
            set_notify,
            create_exam,
            create_practice,
            exam_step,
            get_exam,
            list_exams,
            save_exam_answer,
            submit_exam,
            delete_exam,
            create_plan,
            list_plans,
            delete_plan,
            plan_today,
            rename_material,
            export_material,
            delete_material,
            list_cards,
            mark_learned,
            today_queue,
            grade_answer,
            submit_review,
            stats,
            recent_reviews,
            render_knowledge_map,
            render_review_plan,
            render_stage_guide,
            export_svg,
            reveal_path,
            save_snapshot
        ])
        .run(tauri::generate_context!())
        .expect("致知启动失败");
}