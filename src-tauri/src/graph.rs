//! 程序化生成看板三张图：知识地图 / 复习计划 / 阶段指导（纯 SVG 字符串，无渲染依赖）。

use crate::store::{Card, Stats};

const BG: &str = "#ffffff";
const PANEL: &str = "#f7f8fa";
const LINE: &str = "#e4e7ec";
const FG: &str = "#1a1d21";
const MUTED: &str = "#5b6472";
const DIM: &str = "#c8cfd9";
const ACCENT: &str = "#059669";
const INFO: &str = "#0284c7";
const WARN: &str = "#d97706";
const BAD: &str = "#e11d48";
const FONT: &str = "Microsoft YaHei, Segoe UI, PingFang SC, sans-serif";

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}


fn clip(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    if count <= limit {
        return text.to_string();
    }
    let head: String = text.chars().take(limit.saturating_sub(1)).collect();
    format!("{head}…")
}

fn status_color(card: &Card, today: &str) -> &'static str {
    if !card.learned {
        DIM
    } else if card.due.as_str() <= today {
        WARN
    } else if card.reps > 0 {
        ACCENT
    } else {
        INFO
    }
}


fn svg_open(width: f64, height: f64) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\" font-family=\"{FONT}\">\
         <rect width=\"{w}\" height=\"{h}\" fill=\"{BG}\"/>",
        w = width, h = height, FONT = FONT, BG = BG
    )
}

fn text(x: f64, y: f64, size: f64, fill: &str, weight: &str, anchor: &str, content: &str) -> String {
    format!(
        "<text x=\"{x}\" y=\"{y}\" font-size=\"{size}\" fill=\"{fill}\" font-weight=\"{weight}\" text-anchor=\"{anchor}\">{c}</text>",
        x = x, y = y, size = size, fill = fill, weight = weight, anchor = anchor, c = esc(content)
    )
}

fn legend(x: f64, y: f64, color: &str, label: &str) -> String {
    format!(
        "<circle cx=\"{cx}\" cy=\"{cy}\" r=\"6\" fill=\"{color}\"/>{}",
        text(x + 14.0, y + 4.0, 13.0, MUTED, "400", "start", label),
        cx = x, cy = y, color = color
    )
}

/// 知识地图：按章节分组的单元簇，比平铺网格更容易看出结构和缺口。
pub fn knowledge_map(material_title: &str, cards: &[Card], today: &str) -> String {
    const COL_W: f64 = 636.0;
    const GAP: f64 = 22.0;
    const MARGIN: f64 = 44.0;
    const NODE_W: f64 = 188.0;
    const NODE_H: f64 = 40.0;
    const NODE_GAP: f64 = 9.0;
    const PER_ROW: usize = 3;
    const HEADER: f64 = 168.0;
    const COLUMNS: usize = 2;

    // 按章节归并，保持材料里的顺序
    let mut groups: Vec<(String, Vec<&Card>)> = Vec::new();
    for card in cards {
        let key = if card.section.trim().is_empty() {
            "未分类".to_string()
        } else {
            card.section.clone()
        };
        match groups.iter_mut().find(|(name, _)| *name == key) {
            Some((_, list)) => list.push(card),
            None => groups.push((key, vec![card])),
        }
    }

    let block_height = |count: usize| -> f64 {
        let rows = count.div_ceil(PER_ROW);
        78.0 + rows as f64 * (NODE_H + NODE_GAP)
    };

    // 瀑布流：章节块依次放进当前最矮的一列
    let width = MARGIN * 2.0 + COL_W * COLUMNS as f64 + GAP;
    let mut column_heights = vec![0.0f64; COLUMNS];
    let mut placements: Vec<(f64, f64, usize)> = Vec::new();
    for (_, list) in &groups {
        let column = column_heights
            .iter()
            .enumerate()
            .min_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(index, _)| index)
            .unwrap_or(0);
        let x = MARGIN + column as f64 * (COL_W + GAP);
        placements.push((x, HEADER + column_heights[column], column));
        column_heights[column] += block_height(list.len()) + GAP;
    }
    let body_height = column_heights.iter().cloned().fold(0.0, f64::max);
    let height = HEADER + body_height + 48.0;

    let learned = cards.iter().filter(|c| c.learned).count();
    let due = cards.iter().filter(|c| c.learned && c.due.as_str() <= today).count();

    let mut s = svg_open(width, height);
    s.push_str(&text(MARGIN, 66.0, 30.0, FG, "700", "start", "知识地图"));
    s.push_str(&text(
        MARGIN,
        96.0,
        15.0,
        MUTED,
        "400",
        "start",
        &format!(
            "{} · {} 个单元 · {} 个章节 · 生成于 {}",
            material_title,
            cards.len(),
            groups.len(),
            today
        ),
    ));

    // 总体进度条
    let bar_w = width - MARGIN * 2.0;
    let ratio = if cards.is_empty() { 0.0 } else { learned as f64 / cards.len() as f64 };
    s.push_str(&format!(
        "<rect x=\"{x}\" y=\"118\" width=\"{w}\" height=\"10\" rx=\"5\" fill=\"#eceff3\"/>\
         <rect x=\"{x}\" y=\"118\" width=\"{fw}\" height=\"10\" rx=\"5\" fill=\"{ACCENT}\"/>",
        x = MARGIN,
        w = bar_w,
        fw = (bar_w * ratio).max(0.0),
        ACCENT = ACCENT
    ));
    s.push_str(&text(
        width - MARGIN,
        108.0,
        14.0,
        MUTED,
        "500",
        "end",
        &format!("已学 {learned} / {} · 待复习 {due}", cards.len()),
    ));

    // 图例
    let mut legend_x = MARGIN;
    for (color, label) in [
        (INFO, "已学待测"),
        (ACCENT, "已进复习"),
        (WARN, format!("待复习 {due}").as_str()),
        (DIM, "未学"),
    ] {
        s.push_str(&legend(legend_x, 146.0, color, label));
        legend_x += 150.0;
    }

    for (index, (name, list)) in groups.iter().enumerate() {
        let (x, y, _) = placements[index];
        let done = list.iter().filter(|card| card.learned).count();
        let block_h = block_height(list.len());
        s.push_str(&format!(
            "<rect x=\"{x}\" y=\"{y}\" width=\"{COL_W}\" height=\"{h}\" rx=\"14\" fill=\"{PANEL}\" stroke=\"{LINE}\"/>",
            x = x,
            y = y,
            COL_W = COL_W,
            h = block_h,
            PANEL = PANEL,
            LINE = LINE
        ));
        s.push_str(&text(x + 18.0, y + 30.0, 15.0, FG, "600", "start", name));
        s.push_str(&text(
            x + COL_W - 18.0,
            y + 30.0,
            13.0,
            MUTED,
            "400",
            "end",
            &format!("{done}/{}", list.len()),
        ));
        let inner_w = COL_W - 36.0;
        let block_ratio = done as f64 / list.len().max(1) as f64;
        s.push_str(&format!(
            "<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"5\" rx=\"3\" fill=\"#e6e9ee\"/>\
             <rect x=\"{x}\" y=\"{y}\" width=\"{fw}\" height=\"5\" rx=\"3\" fill=\"{INFO}\"/>",
            x = x + 18.0,
            y = y + 44.0,
            w = inner_w,
            fw = (inner_w * block_ratio).max(0.0),
            INFO = INFO
        ));

        for (position, card) in list.iter().enumerate() {
            let node_x = x + 18.0 + (position % PER_ROW) as f64 * (NODE_W + NODE_GAP);
            let node_y = y + 62.0 + (position / PER_ROW) as f64 * (NODE_H + NODE_GAP);
            let color = status_color(card, today);
            s.push_str(&format!(
                "<rect x=\"{nx}\" y=\"{ny}\" width=\"{NODE_W}\" height=\"{NODE_H}\" rx=\"9\" fill=\"#ffffff\" stroke=\"{LINE}\"/>\
                 <rect x=\"{nx}\" y=\"{ny}\" width=\"4\" height=\"{NODE_H}\" rx=\"2\" fill=\"{color}\"/>",
                nx = node_x,
                ny = node_y,
                NODE_W = NODE_W,
                NODE_H = NODE_H,
                LINE = LINE,
                color = color
            ));
            s.push_str(&text(
                node_x + 12.0,
                node_y + 17.0,
                10.0,
                MUTED,
                "600",
                "start",
                &format!("#{:03}", card.seq),
            ));
            s.push_str(&text(
                node_x + 12.0,
                node_y + 32.0,
                11.5,
                FG,
                "400",
                "start",
                &clip(&card.title, 13),
            ));
        }
    }

    if cards.is_empty() {
        s.push_str(&text(
            width / 2.0,
            HEADER + 60.0,
            18.0,
            MUTED,
            "400",
            "middle",
            "还没有学习单元，先导入材料",
        ));
    }
    s.push_str("</svg>");
    s
}

/// 复习计划：未来 30 天每日到期量 + 今日队列。
pub fn review_plan(forecast: &[(String, i64)], today: &str, queue: &[Card]) -> String {
    let width = 1400.0f64;
    let chart_top = 150.0f64;
    let chart_h = 300.0f64;
    let left = 70.0f64;
    let right = width - 70.0;
    let days = 30usize;
    let slot = (right - left) / days as f64;
    let max_count = forecast.iter().map(|(_, n)| *n).max().unwrap_or(1).max(1) as f64;

    let mut dates: Vec<String> = Vec::with_capacity(days);
    for i in 0..days {
        dates.push(crate::srs::add_days(today, i as i64));
    }
    let counts: Vec<i64> = dates
        .iter()
        .map(|d| forecast.iter().find(|(dd, _)| dd == d).map(|(_, n)| *n).unwrap_or(0))
        .collect();
    let total: i64 = counts.iter().sum();

    let queue_rows = queue.len().min(14);
    let height = chart_top + chart_h + 120.0 + queue_rows as f64 * 30.0 + 120.0;

    let mut s = svg_open(width, height);
    s.push_str(&text(left, 62.0, 30.0, FG, "700", "start", "复习计划"));
    s.push_str(&text(
        left,
        92.0,
        15.0,
        MUTED,
        "400",
        "start",
        &format!("FSRS 记忆曲线排程 · 未来 30 天共 {total} 次复习 · 今天 {today}"),
    ));

    // 坐标轴
    s.push_str(&format!(
        "<line x1=\"{left}\" y1=\"{y}\" x2=\"{right}\" y2=\"{y}\" stroke=\"{LINE}\"/>",
        left = left, right = right, y = chart_top + chart_h, LINE = LINE
    ));
    let mut last_tick = i64::MIN;
    for g in 0..=4 {
        let gy = chart_top + chart_h - chart_h * (g as f64) / 4.0;
        let val = (max_count * (g as f64) / 4.0).round() as i64;
        if val == last_tick {
            continue;
        }
        last_tick = val;
        s.push_str(&format!(
            "<line x1=\"{left}\" y1=\"{gy}\" x2=\"{right}\" y2=\"{gy}\" stroke=\"{LINE}\" stroke-dasharray=\"3 6\"/>",
            left = left, right = right, gy = gy, LINE = LINE
        ));
        s.push_str(&text(left - 12.0, gy + 4.0, 12.0, MUTED, "400", "end", &val.to_string()));
    }

    for (i, count) in counts.iter().enumerate() {
        let bar_h = if max_count > 0.0 { chart_h * (*count as f64) / max_count } else { 0.0 };
        let x = left + i as f64 * slot + slot * 0.2;
        let w = slot * 0.6;
        let y = chart_top + chart_h - bar_h;
        let color = if i == 0 { WARN } else if *count == 0 { LINE } else { ACCENT };
        let h = if *count == 0 { 3.0 } else { bar_h.max(3.0) };
        s.push_str(&format!(
            "<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" rx=\"4\" fill=\"{color}\"/>",
            x = x, y = y, w = w, h = h, color = color
        ));
        if i % 3 == 0 || i == 0 {
            s.push_str(&text(x + w / 2.0, chart_top + chart_h + 22.0, 11.5, MUTED, "400", "middle", &dates[i][5..]));
        }
    }

    // 今日队列
    let list_top = chart_top + chart_h + 76.0;
    s.push_str(&text(left, list_top, 18.0, FG, "600", "start", &format!("今天要做（{} 张）", queue.len())));
    if queue.is_empty() {
        s.push_str(&text(left, list_top + 32.0, 14.0, MUTED, "400", "start", "今天没有要复习的卡：要么去学新卡，要么休息。"));
    }
    for (i, card) in queue.iter().take(queue_rows).enumerate() {
        let y = list_top + 34.0 + i as f64 * 30.0;
        let color = status_color(card, today);
        s.push_str(&format!("<circle cx=\"{cx}\" cy=\"{cy}\" r=\"4\" fill=\"{color}\"/>", cx = left + 8.0, cy = y - 4.0, color = color));
        s.push_str(&text(left + 24.0, y, 14.0, FG, "400", "start", &format!("{} — {}", card.material_title, card.title)));
        s.push_str(&text(right, y, 12.5, MUTED, "400", "end", &format!("到期 {} · 已复习 {} 次", card.due, card.reps)));
    }
    s.push_str("</svg>");
    s
}

/// 阶段指导：当前阶段、关键指标与下一步建议。
pub fn stage_guide(stats: &Stats, cards: &[Card], today: &str) -> String {
    let stages = ["入门", "铺开", "巩固", "精通"];
    let width = 1280.0f64;

    // 建议与薄弱点先算出来，画布高度按实际内容撑开，避免末尾被裁掉
    let advice = advice_for(stats, cards, today);
    let mut weak: Vec<(&Card, f64)> = cards
        .iter()
        .filter(|c| c.learned && c.reps > 0)
        .map(|c| (c, c.stability))
        .collect();
    weak.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let weak_rows = weak.len();
    let list_top = 500.0f64;
    let weak_top = list_top + 34.0 + advice.len() as f64 * 78.0 + 24.0;
    let weak_line = if weak_rows == 0 { 30.0 } else { 32.0 + weak_rows as f64 * 30.0 };
    let height = (weak_top + weak_line + 48.0).max(900.0);

    let mut s = svg_open(width, height);
    s.push_str(&text(64.0, 66.0, 30.0, FG, "700", "start", "阶段学习指导"));
    s.push_str(&text(
        64.0,
        96.0,
        15.0,
        MUTED,
        "400",
        "start",
        &format!("生成于 {} · 阶段判定基于覆盖率与答题平均分", today),
    ));

    // 阶段轨道
    let track_y = 180.0f64;
    let track_left = 64.0f64;
    let track_right = width - 64.0;
    s.push_str(&format!(
        "<line x1=\"{l}\" y1=\"{y}\" x2=\"{r}\" y2=\"{y}\" stroke=\"{LINE}\" stroke-width=\"3\"/>",
        l = track_left, y = track_y, r = track_right, LINE = LINE
    ));
    let step = (track_right - track_left) / (stages.len() as f64 - 1.0);
    for (i, name) in stages.iter().enumerate() {
        let cx = track_left + step * i as f64;
        let active = *name == stats.stage.as_str();
        let passed = stages.iter().position(|x| *x == stats.stage).unwrap_or(0) > i;
        let color = if active { ACCENT } else if passed { INFO } else { DIM };
        s.push_str(&format!(
            "<circle cx=\"{cx}\" cy=\"{y}\" r=\"{r}\" fill=\"{color}\"/>",
            cx = cx, y = track_y, r = if active { 14.0 } else { 10.0 }, color = color
        ));
        s.push_str(&text(cx, track_y + 40.0, 15.0, if active { FG } else { MUTED }, if active { "700" } else { "400" }, "middle", name));
    }
    s.push_str(&text(
        64.0,
        262.0,
        15.0,
        MUTED,
        "400",
        "start",
        &format!("当前处于「{}」阶段，完成度 {:.0}% · 下一阶段：{}", stats.stage, stats.stage_progress * 100.0, stats.next_stage),
    ));
    let bar_w = width - 128.0;
    s.push_str(&format!(
        "<rect x=\"64\" y=\"282\" width=\"{w}\" height=\"12\" rx=\"6\" fill=\"{PANEL}\"/>\
         <rect x=\"64\" y=\"282\" width=\"{fw}\" height=\"12\" rx=\"6\" fill=\"{ACCENT}\"/>",
        w = bar_w, fw = bar_w * stats.stage_progress.clamp(0.0, 1.0), PANEL = PANEL, ACCENT = ACCENT
    ));

    // 指标卡
    let metrics = [
        ("已学知识点", format!("{} / {}", stats.learned, stats.cards), ACCENT),
        ("待复习", format!("{}", stats.due_today), WARN),
        ("答题平均分", format!("{:.1}", stats.avg_score), ACCENT),
        ("连续学习", format!("{} 天", stats.streak_days), FG),
        ("累计练习", format!("{} 次", stats.reviews_total), FG),
    ];
    let card_w = (width - 128.0 - 4.0 * 20.0) / 5.0;
    for (i, (label, value, color)) in metrics.iter().enumerate() {
        let x = 64.0 + i as f64 * (card_w + 20.0);
        let y = 340.0f64;
        s.push_str(&format!(
            "<rect x=\"{x}\" y=\"{y}\" width=\"{card_w}\" height=\"104\" rx=\"14\" fill=\"{PANEL}\" stroke=\"{LINE}\"/>",
            x = x, y = y, card_w = card_w, PANEL = PANEL, LINE = LINE
        ));
        s.push_str(&text(x + 20.0, y + 44.0, 26.0, color, "700", "start", value));
        s.push_str(&text(x + 20.0, y + 76.0, 13.0, MUTED, "400", "start", label));
    }

    // 下一步建议
    s.push_str(&text(64.0, list_top, 20.0, FG, "700", "start", "下一步怎么走"));
    for (i, (headline, detail)) in advice.iter().enumerate() {
        let y = list_top + 34.0 + i as f64 * 78.0;
        s.push_str(&format!(
            "<rect x=\"64\" y=\"{y}\" width=\"{w}\" height=\"64\" rx=\"12\" fill=\"{PANEL}\" stroke=\"{LINE}\"/>\
             <rect x=\"64\" y=\"{y}\" width=\"4\" height=\"64\" rx=\"2\" fill=\"{ACCENT}\"/>",
            y = y, w = width - 128.0, PANEL = PANEL, LINE = LINE, ACCENT = ACCENT
        ));
        s.push_str(&text(86.0, y + 28.0, 15.0, FG, "600", "start", headline));
        s.push_str(&text(86.0, y + 50.0, 13.0, MUTED, "400", "start", detail));
    }

    // 薄弱点：左边标题最长 40 字，右侧留出指标位，避免互相压字
    s.push_str(&text(64.0, weak_top, 20.0, FG, "700", "start", "最薄弱的知识点（优先重学）"));
    if weak.is_empty() {
        s.push_str(&text(64.0, weak_top + 30.0, 14.0, MUTED, "400", "start", "还没有复习记录，先去考试模式答几题。"));
    }
    for (i, (card, stability)) in weak.iter().take(6).enumerate() {
        let y = weak_top + 32.0 + i as f64 * 30.0;
        s.push_str(&format!("<circle cx=\"72\" cy=\"{cy}\" r=\"4\" fill=\"{BAD}\"/>", cy = y - 4.0, BAD = BAD));
        s.push_str(&text(88.0, y, 14.0, FG, "400", "start", &clip(&format!("{} · {}", card.title, card.material_title), 56)));
        s.push_str(&text(width - 64.0, y, 12.5, MUTED, "400", "end", &format!("记忆稳定度 {:.1} · 复习 {} 次", stability, card.reps)));
    }
    if weak_rows > 6 {
        s.push_str(&text(64.0, weak_top + 32.0 + 6.0 * 30.0, 13.0, MUTED, "400", "start", &format!("还有 {} 个薄弱知识点，去考试页按分数排序复习。", weak_rows - 6)));
    }

    s.push_str("</svg>");
    s
}

fn advice_for(stats: &Stats, cards: &[Card], today: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let unlearned = cards.iter().filter(|c| !c.learned).count();

    if stats.cards == 0 {
        out.push(("先导入材料".into(), "在材料页粘贴链接或上传 PDF / Word / Markdown，让模型拆出知识点。".into()));
        return out;
    }
    if stats.due_today > 0 {
        out.push((
            format!("先复习这 {} 张卡", stats.due_today),
            "它们刚好落在记忆快要衰减的位置，先做它们再学新内容最省力。".into(),
        ));
    } else {
        out.push(("今天没有要复习的卡".into(), "可以推进新知识点，或者把最薄弱的卡片手动重来一遍。".into()));
    }
    if unlearned > 0 {
        out.push((
            format!("还有 {unlearned} 个知识点没学"),
            format!("当前阶段「{}」，建议每天新增 5-10 张，学完当天就考一次。", stats.stage),
        ));
    }
    if stats.reviews_total > 0 && stats.avg_score < 7.0 {
        out.push((
            format!("平均分 {:.1}，偏低", stats.avg_score),
            "答题前先用「学习」页把 summary 和要点过一遍，再动手写答案；答完对比遗漏要点。".into(),
        ));
    } else if stats.avg_score >= 8.0 && stats.learned > 0 {
        out.push((
            format!("平均分 {:.1}，掌握不错", stats.avg_score),
            "可以拉长间隔：把自评设为「太简单」，让 FSRS 把复习推到更远的未来。".into(),
        ));
    }
    if stats.streak_days >= 2 {
        out.push((format!("已连续学习 {} 天", stats.streak_days), "保持节奏比一次学很多更重要，今天哪怕只复习几张也算延续。".into()));
    } else {
        out.push(("把节奏固定下来".into(), "每天固定一个时间点打开，先复习到期的再学新的，连续 7 天会明显感到轻松。".into()));
    }
    let _ = today;
    out.truncate(4);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Card, Stats};

    fn card(id: i64, seq: i64, learned: bool, due: &str, reps: i64) -> Card {
        Card {
            id, material_id: 1, material_title: "材料".into(), seq,
            section: String::new(),
            title: format!("知识点 {id}"), content: String::new(),
            summary: String::new(), key_points: vec![],
            question: String::new(), answer: String::new(), tags: vec![],
            state: 0, stability: 3.0, difficulty: 5.0, reps, lapses: 0,
            last_review: None, due: due.into(), learned, exam_notes: vec![],
        }
    }

    #[test]
    fn knowledge_map_renders_every_card() {
        let cards = vec![card(1, 1, false, "2026-10-10", 0), card(2, 2, true, "2026-10-11", 2)];
        let svg = knowledge_map("测试材料", &cards, "2026-10-10");
        assert!(svg.starts_with("<svg"));
        assert!(svg.contains("知识点 1") && svg.contains("知识点 2"));
        assert!(svg.contains("知识地图"));
    }

    #[test]
    fn review_plan_handles_empty_forecast() {
        let svg = review_plan(&[], "2026-10-10", &[]);
        assert!(svg.contains("今天没有要复习的卡"));
    }

    #[test]
    fn stage_guide_produces_advice() {
        let stats = Stats {
            materials: 1, cards: 10, learned: 4, due_today: 3, reviews_total: 12, reviews_today: 5,
            avg_score: 6.5, streak_days: 3, active_days: 4,
            stage: "铺开".into(), stage_progress: 0.5, next_stage: "巩固".into(),
        };
        let svg = stage_guide(&stats, &[card(1, 1, true, "2026-10-10", 2)], "2026-10-10");
        assert!(svg.contains("下一步怎么走"));
        assert!(svg.contains("先复习这 3 张卡"));
    }

    #[test]
    fn escapes_xml_specials() {
        assert_eq!(esc("a<b>&\"c\""), "a&lt;b&gt;&amp;&quot;c&quot;");
    }
}