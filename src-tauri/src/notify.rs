//! 学习提醒：过了提醒时间、当天还提醒过，就弹一条系统通知；一天只响一次。

use std::path::Path;
use std::time::Duration;

use chrono::{Local, Timelike};
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::AppState;

/// 提醒时间：九点之前不打扰，过了九点当天第一次检查就发。
const REMIND_HOUR: u32 = 9;

/// 每天提醒一次，所以标记文件只记最后一次提醒的日期。
const STAMP: &str = "notify.json";

/// 注册应用的 AppUserModelID：免安装程序发通知时，Windows 靠这个键显示
/// 通知标题（否则会显示成「Windows PowerShell」）。
#[cfg(target_os = "windows")]
pub fn register_identity(app_id: &str, display_name: &str) {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_WRITE};
    use winreg::RegKey;

    let path = format!(r"Software\Classes\AppUserModelId\{app_id}");
    match RegKey::predef(HKEY_CURRENT_USER).create_subkey_with_flags(path, KEY_WRITE) {
        Ok((key, _)) => {
            let _ = key.set_value("DisplayName", &display_name);
            let _ = key.set_value("IconBackgroundColor", &"0");
        }
        Err(error) => eprintln!("[提醒] 注册通知标识失败：{error}"),
    }
}

#[cfg(not(target_os = "windows"))]
pub fn register_identity(_app_id: &str, _display_name: &str) {}

/// 起一个后台线程轮询：应用可能一直在托盘里待着，不能只在启动时判断一次。
pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || loop {
        if let Err(error) = tick(&app) {
            eprintln!("[提醒] {error}");
        }
        std::thread::sleep(Duration::from_secs(30));
    });
}

fn tick(app: &AppHandle) -> anyhow::Result<()> {
    let now = Local::now();
    if now.hour() < REMIND_HOUR {
        return Ok(());
    }
    let today = now.format("%Y-%m-%d").to_string();

    let state = app.state::<AppState>();
    if !state.settings.read().unwrap().notify {
        return Ok(());
    }
    let stamp = state.data_dir.join(STAMP);
    if last_sent(&stamp).as_deref() == Some(today.as_str()) {
        return Ok(());
    }

    let (due, fresh) = {
        let store = state.store.lock().unwrap();
        let stats = store.stats(&today)?;
        (stats.due_today, (stats.cards - stats.learned).max(0))
    };

    // 先记日期再发：发失败也不该每分钟重试一次
    let _ = std::fs::write(&stamp, format!("{{\"date\":\"{today}\"}}"));
    app.notification()
        .builder()
        .title("致知 · 该学习了")
        .body(body_for(due, fresh))
        .show()?;
    Ok(())
}

fn body_for(due: i64, fresh: i64) -> String {
    match (due, fresh) {
        (0, 0) => "材料已经全部学过一遍，去考试页自评几张卡吧。".to_string(),
        (0, n) => format!("今天没有要复习的卡，可以推进 {n} 个新知识点。"),
        (n, _) => format!("今天有 {n} 张卡等着复习，清掉它们就收工。"),
    }
}

fn last_sent(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value["date"].as_str().map(|value| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_prefers_due_cards() {
        assert!(body_for(7, 90).contains("7"));
        assert!(body_for(0, 90).contains("90"));
        assert!(body_for(0, 0).contains("考试页"));
    }

    #[test]
    fn last_sent_reads_only_today_marker() {
        let dir = std::env::temp_dir().join("zhizhi-notify-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("notify.json");
        std::fs::write(&path, "{\"date\":\"2026-10-12\"}").unwrap();
        assert_eq!(last_sent(&path).as_deref(), Some("2026-10-12"));
        std::fs::remove_file(&path).ok();
        assert_eq!(last_sent(&path), None);
    }
}
