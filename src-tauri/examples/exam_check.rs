//! 临时工具：验证出题提示词与 JSON 解析（跑完即删）。

use std::path::PathBuf;

use zhizhi_lib::llm::LlmClient;
use zhizhi_lib::settings::Settings;
use zhizhi_lib::store::Store;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let settings = Settings::load(&PathBuf::from(std::env::var("APPDATA")?)
        .join("com.zhizhi.desktop")
        .join("config.json"));
    let llm = LlmClient::from_settings(&settings)?;
    let store = Store::open(&PathBuf::from(std::env::var("APPDATA")?)
        .join("com.zhizhi.desktop")
        .join("zhizhi.db"))?;
    let cards = store.list_cards(None)?;
    let card = cards.iter().find(|c| c.content.chars().count() > 500).unwrap();
    println!("卡片：{}（{} 字正文）", card.title, card.content.chars().count());

    let asks = vec![
        ("single".to_string(), 2.0),
        ("judge".to_string(), 1.0),
        ("blank".to_string(), 2.0),
        ("short".to_string(), 5.0),
    ];
    let questions = llm.make_questions(&card.title, &card.content, &asks).await?;
    println!("出了 {} 道题", questions.len());
    for q in &questions {
        println!("--- [{}] {} 分", q.kind, q.points);
        println!("题干：{}", q.stem);
        for (i, option) in q.options.iter().enumerate() {
            println!("  {}. {}", (b'A' + i as u8) as char, option);
        }
        println!("答案：{}", q.answer);
        println!("解析：{}", q.explain);
    }
    Ok(())
}
