//! LLM 调用：整理材料、生成考点、评判作答（OpenAI 兼容接口）。

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::settings::Settings;
use crate::store::CardDigest;

const GRADE_SYSTEM: &str = "你是一位严格但公正的技术面试官。\
对照参考答案评判面试者的回答，判断概念是否准确、机制与权衡是否讲清。\
输出 JSON 对象：score（0-10 的数字，6 分及格）、\
feedback（2-4 句评语，先肯定答对的部分，再指出问题）、\
missed（遗漏的关键点字符串数组，没有则空数组）、\
suggestion（2-4 句理想回答）。\
只输出 JSON 对象，不要任何解释。";

const EXTRACT_SYSTEM: &str = "你是资料整理助手。用户会给你一段从网页或搜索结果里抓来的原始文本，\
把它整理成干净、可直接用于学习的中文 Markdown 材料：\
1. 完整保留原文的信息：事实、数据、步骤、代码、表格、注意事项与结论，一个都不要省；\
2. 只删掉导航、广告、登录提示、相关推荐、作者署名这类与内容无关的噪音；\
3. 保留层级结构，用 Markdown 标题、列表、表格、代码块组织；\
4. 多篇来源时按主题合并去重，不要逐篇罗列；\
5. 不要概括，不要写「等等」「略」，也不要写「本文介绍了」这类套话；\
6. 不要编造原文没有的内容；\
7. 如果文本只是长材料的一部分，就只整理这一段，不要加导语、总结或结尾。\
直接输出整理后的 Markdown，不要 JSON、不要代码块围栏、不要任何解释。";

const DIGEST_SYSTEM: &str = "你是面试题设计者。用户会给你一篇技术学习材料的标题与正文，请为它出一份考点，供读者读完后自测：\
summary（一句话说明这篇讲什么，不超过 40 字）、\
keyPoints（读者必须掌握的 3-6 个要点，每条一句话，尽量具体到机制、取舍或数字）、\
question（一道检验是否真的读懂的面试题，必须是问句）、\
answer（参考答案，覆盖全部 keyPoints）。\
正文已经完整保留给读者，所以不要复述、不要压缩、不要摘抄正文，只给考点本身。\
只输出 JSON 对象：{\"summary\": \"\", \"keyPoints\": [], \"question\": \"\", \"answer\": \"\"}，不要任何解释。";

/// 模型整理出来的材料。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedMaterial {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GradeResult {
    pub score: f64,
    pub feedback: String,
    #[serde(default)]
    pub missed: Vec<String>,
    #[serde(default)]
    pub suggestion: String,
}

#[derive(Clone)]
pub struct LlmClient {
    http: reqwest::Client,
    api_key: String,
    base: String,
    model: String,
}

impl LlmClient {
    /// 从运行期设置构建客户端：接口地址、Key、代理都来自设置。
    pub fn from_settings(settings: &Settings) -> Result<Self> {
        let http = settings.build_http(240)?;
        let base = settings.api_base.trim().trim_end_matches('/').to_string();
        let key = settings.api_key.trim().to_string();
        Ok(Self {
            http,
            api_key: key,
            base,
            model: settings.model.trim().to_string(),
        })
    }

    /// 最小连通性测试：能拿到回复就说明 Key、地址、网络都通。
    pub async fn ping(&self) -> Result<String> {
        let messages = json!([{ "role": "user", "content": "只回复两个字：正常" }]);
        let reply = self.chat(messages, &self.model, 16, 0.0).await?;
        Ok(reply.trim().chars().take(40).collect())
    }

    pub fn ready(&self) -> bool {
        !self.api_key.is_empty()
    }

    pub fn model_name(&self) -> &str {
        &self.model
    }

    async fn chat(&self, messages: Value, model: &str, max_tokens: u32, temperature: f32) -> Result<String> {
        self.chat_at(&self.base, &self.api_key, messages, model, max_tokens, temperature)
            .await
    }

    async fn chat_at(
        &self,
        base: &str,
        key: &str,
        messages: Value,
        model: &str,
        max_tokens: u32,
        temperature: f32,
    ) -> Result<String> {
        if key.is_empty() {
            return Err(anyhow!("未配置 LLM_API_KEY：请在项目根目录的 .env 里填写后重启"));
        }
        let body = json!({
            "model": model,
            "messages": messages,
            "max_tokens": max_tokens,
            "temperature": temperature,
        });
        let resp = self
            .http
            .post(format!("{base}/chat/completions"))
            .bearer_auth(key)
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await?;
        if !status.is_success() {
            let brief: String = text.chars().take(300).collect();
            return Err(anyhow!("模型接口返回 {}：{}", status.as_u16(), brief));
        }
        let value: Value = serde_json::from_str(&text)?;
        let content = value["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        if content.is_empty() {
            let choice = &value["choices"][0];
            let reasoning = choice["message"]["reasoning_content"].as_str().unwrap_or("");
            let truncated = choice["finish_reason"].as_str() == Some("length");
            return Err(anyhow!(if truncated && !reasoning.is_empty() {
                "模型把 token 全花在思考（reasoning）上就截断了，没有输出正文：                 请把模型换成 deepseek-chat 这类非思考模型"
            } else if !reasoning.is_empty() {
                "模型只返回了思考内容、没有正文：请把模型换成 deepseek-chat 这类非思考模型"
            } else {
                "模型返回内容为空"
            }));
        }
        Ok(content)
    }

    /// 为一段学习材料出考点（摘要 / 要点 / 考题 / 答案）。
    pub async fn digest_unit(&self, title: &str, content: &str) -> Result<CardDigest> {
        let user = format!(
            "材料标题：{title}\n\n材料正文：\n{}",
            crate::ingest::truncate_chars(content, 12_000)
        );
        let messages = json!([
            { "role": "system", "content": DIGEST_SYSTEM },
            { "role": "user", "content": user },
        ]);
        let raw = self.chat(messages, &self.model, 2000, 0.3).await?;
        let json_text = extract_json(&raw, '{', '}')?;
        let digest: CardDigest = serde_json::from_str(&json_text)
            .map_err(|e| anyhow!("考点 JSON 解析失败：{e}"))?;
        Ok(digest)
    }

    /// 把抓来的原始文本整理成干净材料（智能导入的默认路径）。返回 Markdown 正文。
    pub async fn extract_material(&self, origin: &str, section_hint: &str, raw: &str) -> Result<String> {
        let user = format!(
            "来源：{origin}\n位置：{section_hint}\n\n原始文本：\n{raw}"
        );
        let messages = json!([
            { "role": "system", "content": EXTRACT_SYSTEM },
            { "role": "user", "content": user },
        ]);
        let text = self.chat(messages, &self.model, 8000, 0.2).await?;
        Ok(text.trim().to_string())
    }

    pub async fn grade(&self, question: &str, reference: &str, user_answer: &str) -> Result<GradeResult> {
        let user = format!(
            "题目：{}\n\n参考答案：{}\n\n面试者的回答：{}",
            question,
            truncate(reference, 4000),
            user_answer
        );
        let messages = json!([
            { "role": "system", "content": GRADE_SYSTEM },
            { "role": "user", "content": user },
        ]);
        let raw = self.chat(messages, &self.model, 1500, 0.2).await?;
        let json_text = extract_json(&raw, '{', '}')?;
        let mut result: GradeResult = serde_json::from_str(&json_text)
            .map_err(|e| anyhow!("模型输出的评分 JSON 解析失败：{e}"))?;
        result.score = result.score.clamp(0.0, 10.0);
        Ok(result)
    }
}

/// 从模型输出里截出 JSON（容忍代码块围栏与前后解释）。
pub fn extract_json(raw: &str, open: char, close: char) -> Result<String> {
    let start = raw.find(open).ok_or_else(|| anyhow!("模型输出里找不到 JSON"))?;
    let end = raw.rfind(close).ok_or_else(|| anyhow!("模型输出的 JSON 不完整"))?;
    if end <= start {
        return Err(anyhow!("模型输出的 JSON 结构异常"));
    }
    Ok(raw[start..=end].to_string())
}

pub fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let cut: String = text.chars().take(max_chars).collect();
    format!("{cut}\n…（后文因长度截断）")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_json_from_fenced_output() {
        let fence = "\u{0060}\u{0060}\u{0060}";
        let raw = format!("好的，结果如下：\n{fence}json\n[{{\"title\":\"A\"}}]\n{fence}\n希望有帮助");
        let got = extract_json(&raw, '[', ']').unwrap();
        assert_eq!(got, "[{\"title\":\"A\"}]");
    }

    #[test]
    fn truncate_keeps_char_boundaries() {
        let s = "中文abc";
        assert_eq!(truncate(s, 10), s);
        assert!(truncate(s, 2).starts_with("中文"));
    }
}