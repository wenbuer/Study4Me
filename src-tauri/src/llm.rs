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

const PAPER_SYSTEM: &str = "你是命题老师。用户会给你一个知识点的标题与正文，以及要出的题型与分值。\
请严格按用户要求的题型和数量出题，输出 JSON 数组，每个元素形如：\
{\"kind\":\"题型（原样用用户给的英文标识）\",\
\"stem\":\"题干\",\
\"options\":[\"选项 A 的内容\",\"选项 B 的内容\"],\
\"answer\":\"标准答案\",\
\"explain\":\"解析，1-3 句，讲清为什么\",\
\"points\":分值数字}\
题型与写法：single 单选（4 个选项，answer 写选项字母如 B）；multi 多选（4-5 个选项，answer 写字母如 ACD）；\
judge 判断（options 固定为 [\"正确\",\"错误\"]，answer 写 正确 或 错误）；blank 填空（options 给空数组，answer 是填空答案，多个可接受写法用 | 分隔）；\
short 简答（options 空数组，answer 写参考答案的 3-5 个要点，用；分隔）；case 案例（options 空数组，answer 写解题要点）。\
硬要求：只考正文里明确写过的事实，不要编造；题干必须自足，不要出现「根据材料」「上文」这类指代；\
填空题不要给提示词位置标记以外的空；只输出 JSON 数组，不要任何解释或代码块围栏。";

const GRADE_ITEM_SYSTEM: &str = "你是阅卷老师。用户会给你一道题的题干、参考答案与考生作答，请按点给分。\
输出 JSON 对象：score（得分数字，0 到该题满分之间，可以给小数）、\
verdict（correct 全对 / partial 部分对 / wrong 基本不对）、\
feedback（2-4 句评语，先肯定答对的部分，再指出问题）、\
missed（遗漏的关键点字符串数组，没有则空数组）、\
suggestion（2-4 句理想答案）。只输出 JSON 对象。";

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

    /// 给一个知识点出题：一次调用出一组题（用户指定题型与分值）。
    pub async fn make_questions(
        &self,
        title: &str,
        content: &str,
        asks: &[(String, f64)],
    ) -> Result<Vec<PaperQuestion>> {
        let list = asks
            .iter()
            .map(|(kind, points)| format!("- {}（{} 分）", kind_label(kind), points))
            .collect::<Vec<_>>()
            .join("\n");
        let user = format!(
            "知识点标题：{title}\n\n知识点正文：\n{}\n\n请按下面这些要求出 {} 道题：\n{list}",
            crate::ingest::truncate_chars(content, 6000),
            asks.len()
        );
        let messages = json!([
            { "role": "system", "content": PAPER_SYSTEM },
            { "role": "user", "content": user },
        ]);
        let raw = self.chat(messages, &self.model, 3000, 0.4).await?;
        let json_text = extract_json(&raw, '[', ']')?;
        let items: Vec<PaperQuestion> = serde_json::from_str(&json_text)
            .map_err(|e| anyhow!("出题 JSON 解析失败：{e}"))?;
        Ok(items)
    }

    /// 主观题判分（客观题在本地判，不用模型）。
    pub async fn grade_exam_answer(
        &self,
        stem: &str,
        reference: &str,
        user_answer: &str,
        points: f64,
    ) -> Result<ItemGrade> {
        let user = format!(
            "题干：{stem}\n\n参考答案：{reference}\n\n本题满分：{points}\n\n考生作答：\n{user_answer}"
        );
        let messages = json!([
            { "role": "system", "content": GRADE_ITEM_SYSTEM },
            { "role": "user", "content": user },
        ]);
        let raw = self.chat(messages, &self.model, 1500, 0.2).await?;
        let json_text = extract_json(&raw, '{', '}')?;
        let mut grade: ItemGrade = serde_json::from_str(&json_text)
            .map_err(|e| anyhow!("判分 JSON 解析失败：{e}"))?;
        grade.score = grade.score.clamp(0.0, points);
        grade.verdict = match grade.verdict.as_str() {
            "correct" | "partial" | "wrong" => grade.verdict.clone(),
            _ if grade.score >= points * 0.99 => "correct".to_string(),
            _ if grade.score <= 0.0 => "wrong".to_string(),
            _ => "partial".to_string(),
        };
        Ok(grade)
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

/// 题型的展示名，出题提示词与界面共用。
pub fn kind_label(kind: &str) -> &'static str {
    match kind {
        "single" => "单选题",
        "multi" => "多选题",
        "judge" => "判断题",
        "blank" => "填空题",
        "short" => "简答题",
        "case" => "案例题",
        _ => "题目",
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PaperQuestion {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub stem: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub answer: String,
    #[serde(default)]
    pub explain: String,
    #[serde(default)]
    pub points: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemGrade {
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub verdict: String,
    #[serde(default)]
    pub feedback: String,
    #[serde(default)]
    pub missed: Vec<String>,
    #[serde(default)]
    pub suggestion: String,
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