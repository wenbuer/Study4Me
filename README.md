![致知](shots/banner-v3.png)

# 致知 · zhizhiStudy

> 学以致知，孜孜不倦：把一份资料变成一套清晰的学习计划，知识点、自测题、复习安排都在一个窗口里完成。

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Release](https://img.shields.io/github/v/release/wenbuer/zhizhiStudy)](https://github.com/wenbuer/zhizhiStudy/releases)

## 简介

学一门新东西，难的往往不是找资料，而是资料到手之后怎么办：读完了记不住，记住了又不知道什么时候该再看一遍。

致知把这件事做简单了。给它一个网页链接、一份 PDF 或 Word，或者直接粘贴一段文字，它会自动整理成一条条清晰的知识点，每条都配好讲解、要点和自测题。学完随手考一次，好在哪里、漏了什么，当场就知道，下一次该复习哪些内容也自动排好。今天要做什么、还剩多少、哪块最薄弱，打开首页一眼看清。

下载即可使用，界面只有七个页面，第一次打开也能直接上手。

## 界面

<table>
  <tr>
    <td width="50%"><img src="shots/start.png" alt="开始页" /><br /><sub>开始：今天要做什么，一眼看清</sub></td>
    <td width="50%"><img src="shots/materials.png" alt="材料页" /><br /><sub>材料：链接、文件、粘贴都能导入</sub></td>
  </tr>
  <tr>
    <td><img src="shots/learn.png" alt="学习页" /><br /><sub>学习：按章节读讲解，读完自测</sub></td>
    <td><img src="shots/quiz.png" alt="测验出卷" /><br /><sub>测验：选好范围与题型，自动出卷</sub></td>
  </tr>
  <tr>
    <td><img src="shots/quiz-result.png" alt="测验成绩与复习要点" /><br /><sub>成绩：交卷即出分与复习要点</sub></td>
    <td><img src="shots/insight.png" alt="学习看板" /><br /><sub>看板：知识地图、复习计划与阶段指导</sub></td>
  </tr>
</table>

<sub>另有**练习**页负责随机出题（按已学章节每章 1–3 题、题型随机、可跳过续做）与单卡作答复习，**设置**页负责模型接口、网络代理与每日提醒。</sub>

## 功能

- 四种导入：网页链接、一个主题、本地文件（PDF / Word / Markdown）、直接粘贴
- 自动拆解：整理成一条条知识点，每条都有讲解、要点与自测题
- 练习与评分：按已学章节随机出题，作答后自动评分，给出评语、遗漏点与参考答案；没做完的练习下次接着做
- 学习计划：按实际进度推进，落下的天数顺延，不会跳过没学的章节
- 测验出卷：六种题型自动组卷，软件内答题，交卷即出分与复习要点
- 定时复习：按记忆曲线安排每天该复习的内容，到点提醒
- 学习看板：知识地图、复习计划、阶段指导，可导出图片

## 快速开始

到 [Releases](https://github.com/wenbuer/zhizhiStudy/releases) 下载 `Zhizhi.exe`，双击运行即可，无需安装。

首次使用在「设置」页填写三项，保存后立刻可用：

| 字段 | 示例 |
|---|---|
| API 地址 | `https://api.deepseek.com/v1` |
| API Key | 你自己的 Key |
| 模型名 | `deepseek-chat` |

没有填写也能打开软件：材料可以正常导入整理，学习和看板都能用，只是自动出题与评分不可用，界面会说明原因。

### 首次运行被拦截怎么办

程序没有代码签名证书，从浏览器下载后 Windows 会弹「Windows 已保护你的电脑」。两种处理方式，任选其一：

1. 弹窗里点「更多信息」→「仍要运行」；
2. 或者右键 `Zhizhi.exe` → 属性 → 勾选「解除锁定」→ 确定，之后双击即可直接运行。

也可以在 PowerShell 里执行 `Unblock-File .\Zhizhi.exe`（解除锁定后会去掉下载标记，不再提示）。

## 复习怎么排

每次作答后，软件会按记忆曲线算出这张卡下次该复习的日期：答得越好，间隔拉得越久；答错或没把握，很快就再见到它。每天只需要清掉当天该做的部分，不必自己规划。

## 数据

所有内容都保存在本机，卸载或删除软件不影响已学记录。数据目录为 `%APPDATA%\com.zhizhi.desktop\`，删除其中的 `zhizhi.db` 即可从零开始。

## 从源码构建

需要 Rust 工具链（MSVC 目标）。

    cargo run --manifest-path src-tauri/Cargo.toml          # 开发运行
    cargo build --release --manifest-path src-tauri/Cargo.toml   # 生成可执行文件
    cargo test --manifest-path src-tauri/Cargo.toml          # 单元测试

## 许可

MIT，见 [LICENSE](LICENSE)。
