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

use serde::Serialize;
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

/// 今天的计划进度：第几天、今天该学哪些单元、还剩多少没学。
#[tauri::command]
fn plan_today(state: State<'_, AppState>, plan_id: i64) -> Result<Option<store::PlanToday>, String> {
    state
        .store
        .lock()
        .unwrap()
        .plan_today(plan_id, &today())
        .map_err(err_str)
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
        scheduled: store.scheduled_cards(&day, 100).map_err(err_str)?,
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
        .tooltip("致知 · 读进去，记得住")
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