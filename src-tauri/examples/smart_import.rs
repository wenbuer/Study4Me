//! 命令行验证智能导入链路。
//!
//! cargo run --example smart_import -- "https://..." 或 "RAG 分块策略"

use zhizhi_lib::{ingest, llm::LlmClient, settings::Settings};

#[tokio::main]
async fn main() {
    let input = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "RAG 分块策略".to_string());
    let settings = Settings::default();
    let llm = LlmClient::from_settings(&settings).expect("初始化模型客户端失败");
    println!("模型就绪：{}（{}）", llm.ready(), llm.model_name());
    let http = settings.build_http(60).expect("初始化 HTTP 客户端失败");
    match settings.resolved_proxy() {
        Some(proxy) => println!("代理：{proxy}"),
        None => println!("代理：直连"),
    }

    let started = std::time::Instant::now();
    let result = if input.starts_with("http") {
        ingest::smart_from_url(&http, &llm, &input).await
    } else {
        ingest::smart_from_topic(&http, &llm, &input).await
    };

    match result {
        Ok(item) => {
            println!("耗时：{:.1} 秒", started.elapsed().as_secs_f32());
            println!("标题：{}", item.title);
            println!("方式：{}", item.via);
            println!("来源：{}", item.source);
            println!("字数：{}", item.chars);
            let preview: String = item.text.chars().take(600).collect();
            println!("--- 正文开头 ---\n{preview}");

            if std::env::args().any(|arg| arg == "--split") {
                let units = ingest::split_units(&item.text, &item.kind);
                println!("\n=== 切分结果 ===");
                println!("学习单元：{} 个", units.len());
                let mut sections: Vec<(String, usize)> = Vec::new();
                for unit in &units {
                    match sections.iter_mut().find(|(name, _)| *name == unit.section) {
                        Some((_, count)) => *count += 1,
                        None => sections.push((unit.section.clone(), 1)),
                    }
                }
                for (name, count) in &sections {
                    println!("  {} · {} 单元", name, count);
                }
                for unit in units.iter().take(2) {
                    let preview: String = unit.content.chars().take(150).collect();
                    println!("\n[{}] {}", unit.section, unit.title);
                    println!("  正文：{}…", preview.replace('\n', " "));
                }
            }

            if std::env::args().any(|arg| arg == "--cards") {
                let units = ingest::split_units(&item.text, &item.kind);
                println!("\n=== 单元考点（取前 2 个单元）===");
                for unit in units.iter().take(2) {
                    match llm.digest_unit(&unit.title, &unit.content).await {
                        Ok(digest) => {
                            println!("\n[{}] {}", unit.section, unit.title);
                            println!("概括：{}", digest.summary);
                            println!("要点：{}", digest.key_points.join(" / "));
                            println!("考题：{}", digest.question);
                        }
                        Err(error) => println!("考点生成失败：{error}"),
                    }
                }
            }
        }
        Err(error) => println!("失败：{error}"),
    }
}