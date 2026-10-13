//! 把看板三张图渲染成 SVG 文件，供离线查看与回归对比。
//!
//! 用法：cargo run --example render_graphs [输出目录]

use std::path::PathBuf;

use zhizhi_lib::{graph, srs, store::Store};

fn main() {
    let out_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../shots"));
    std::fs::create_dir_all(&out_dir).expect("创建输出目录失败");

    let db = std::env::args().nth(2).map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(std::env::var("APPDATA").expect("缺少 APPDATA"))
            .join("com.zhizhi.desktop")
            .join("zhizhi.db")
    });
    let store = Store::open(&db).expect("打开数据库失败");

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let cards = store.list_cards(None).expect("读取卡片失败");
    let stats = store.stats(&today).expect("统计失败");
    let due = store.due_cards(&today, 200).expect("读取到期队列失败");
    let forecast = store.due_forecast(&today, 30).expect("读取预测失败");

    let jobs: [(&str, String); 3] = [
        ("knowledge-map", graph::knowledge_map("全部材料", &cards, &today)),
        ("review-plan", graph::review_plan(&forecast, &today, &due)),
        ("stage-guide", graph::stage_guide(&stats, &cards, &today)),
    ];
    for (name, svg) in jobs {
        let path = out_dir.join(format!("{name}.svg"));
        std::fs::write(&path, &svg).expect("写 SVG 失败");
        println!("{}  {} 字节", path.display(), svg.len());
    }
    println!("下一步复习：{}", srs::add_days(&today, 7));
}