//! FSRS 记忆曲线排程：把「答得怎样」换算成下次复习日期。

use anyhow::Result;
use chrono::{Duration, NaiveDate};
use fsrs::{MemoryState, FSRS};

/// 期望记忆保持率：0.9 表示「九成把握还记得」时才安排下次复习。
pub const DESIRED_RETENTION: f32 = 0.9;

/// 卡片状态（与前端约定）：1 学习中 / 2 复习中 / 3 重学。
pub const STATE_LEARNING: i64 = 1;
pub const STATE_REVIEW: i64 = 2;
pub const STATE_RELEARNING: i64 = 3;

#[derive(Debug, Clone)]
pub struct Scheduled {
    pub state: i64,
    pub stability: f32,
    pub difficulty: f32,
    pub interval_days: i64,
    pub due: String,
}

/// 评分：1 完全忘记 / 2 有点难 / 3 记得 / 4 太简单。
pub fn schedule(current: Option<(f32, f32)>, rating: u32, today: &str) -> Result<Scheduled> {
    let fsrs = FSRS::default();
    let memory = current.and_then(|(s, d)| {
        if s > 0.0 { Some(MemoryState { stability: s, difficulty: d }) } else { None }
    });
    let elapsed = 0u32;
    let next = fsrs.next_states(memory, DESIRED_RETENTION, elapsed)?;
    let item = match rating {
        1 => next.again,
        2 => next.hard,
        3 => next.good,
        _ => next.easy,
    };
    let interval_days = item.interval.round().max(0.0) as i64;
    let due = add_days(today, interval_days.max(if rating == 1 { 0 } else { 1 }));
    let state = match (memory.is_some(), rating) {
        (false, 1) => STATE_LEARNING,
        (false, _) => STATE_LEARNING,
        (true, 1) => STATE_RELEARNING,
        (true, _) => STATE_REVIEW,
    };
    Ok(Scheduled {
        state,
        stability: item.memory.stability,
        difficulty: item.memory.difficulty,
        interval_days,
        due,
    })
}

/// 由考试得分（0-10）映射到 FSRS 四档评分。
pub fn rating_from_score(score: f64) -> u32 {
    if score < 5.0 {
        1
    } else if score < 7.0 {
        2
    } else if score < 9.0 {
        3
    } else {
        4
    }
}

pub fn add_days(date: &str, days: i64) -> String {
    match NaiveDate::parse_from_str(date, "%Y-%m-%d") {
        Ok(d) => (d + Duration::days(days)).format("%Y-%m-%d").to_string(),
        Err(_) => date.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_card_first_review_schedules_ahead() {
        let s = schedule(None, 3, "2026-10-10").unwrap();
        assert!(s.interval_days >= 1, "首次记得应至少排到明天");
        assert!(s.due > "2026-10-10".to_string());
    }

    #[test]
    fn forgetting_shortens_the_interval() {
        let again = schedule(Some((10.0, 5.0)), 1, "2026-10-10").unwrap();
        let good = schedule(Some((10.0, 5.0)), 3, "2026-10-10").unwrap();
        assert_eq!(again.state, STATE_RELEARNING);
        assert!(
            again.interval_days < good.interval_days,
            "忘记后的间隔必须短于记得：again={} good={}",
            again.interval_days,
            good.interval_days
        );
        assert!(again.stability < good.stability, "忘记后记忆稳定度应下降");
    }

    #[test]
    fn easy_beats_hard_interval() {
        let hard = schedule(Some((10.0, 5.0)), 2, "2026-10-10").unwrap();
        let easy = schedule(Some((10.0, 5.0)), 4, "2026-10-10").unwrap();
        assert!(easy.interval_days > hard.interval_days);
    }

    #[test]
    fn score_maps_to_rating() {
        assert_eq!(rating_from_score(3.0), 1);
        assert_eq!(rating_from_score(6.0), 2);
        assert_eq!(rating_from_score(8.0), 3);
        assert_eq!(rating_from_score(9.5), 4);
    }
}
