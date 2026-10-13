"""生成 README 用的仓库横幅。

    python scripts/make_readme_assets.py

产物：shots/banner.png
依赖：本机 Microsoft Edge（无头模式截图）与 src/mark.svg（由 make_icon.py 生成）。
"""

import pathlib
import subprocess
import sys

EDGE = pathlib.Path(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe")
ROOT = pathlib.Path(__file__).resolve().parent.parent
SHOTS = ROOT / "shots"
FONT = '"Microsoft YaHei", "Segoe UI", "PingFang SC", sans-serif'
WIDTH, HEIGHT = 1600, 420

BANNER_HTML = f"""<!doctype html>
<html lang="zh-CN"><head><meta charset="utf-8" /><style>
  * {{ box-sizing: border-box; }}
  body {{
    margin: 0; width: {WIDTH}px; height: {HEIGHT}px; overflow: hidden;
    font-family: {FONT};
    background: linear-gradient(120deg, #ffffff 0%, #f4f7f8 58%, #edf4f1 100%);
    position: relative;
  }}
  .glow {{
    position: absolute; right: -120px; top: -140px; width: 720px; height: 720px;
    border-radius: 50%;
    background: radial-gradient(circle, rgba(4,120,87,0.13) 0%, rgba(4,120,87,0.04) 45%, rgba(4,120,87,0) 70%);
  }}
  .grid {{
    position: absolute; inset: 0;
    background-image: radial-gradient(rgba(15,23,42,0.055) 1.4px, transparent 1.4px);
    background-size: 26px 26px;
    mask-image: linear-gradient(100deg, rgba(0,0,0,0.9) 0%, rgba(0,0,0,0.25) 55%, rgba(0,0,0,0) 80%);
  }}
  .wrap {{ position: relative; height: 100%; display: flex; align-items: center; gap: 40px; padding: 0 84px; }}
  .mark {{ width: 132px; height: 132px; flex: none; }}
  .name {{ display: flex; align-items: baseline; gap: 22px; font-size: 78px; font-weight: 700; letter-spacing: 0.06em; color: #101828; line-height: 1.05; }}
  .motto {{ font-size: 27px; font-weight: 500; color: #047857; letter-spacing: 0.06em; }}
  .rule {{ width: 92px; height: 4px; border-radius: 2px; background: #047857; margin: 22px 0 0; }}
  .meta {{ margin-top: 18px; font-size: 19px; color: #667085; letter-spacing: 0.06em; }}
  .feat {{ margin-top: 12px; font-size: 16px; color: #98a2b3; letter-spacing: 0.04em; }}
</style></head>
<body>
  <div class="grid"></div>
  <div class="glow"></div>
  <div class="wrap">
    <img class="mark" src="../src/mark.svg" alt="" />
    <div>
      <div class="name">致知<span class="motto">学以致知，孜孜不倦</span></div>
      <div class="rule"></div>
      <div class="meta">把一份资料，变成一套清晰的学习计划</div>
      <div class="feat">大模型出题与批改 · FSRS 记忆曲线复习安排 · 网页 / PDF / Word 一键导入 · 知识地图看板</div>
    </div>
  </div>
</body></html>
"""


def main() -> None:
    if not EDGE.exists():
        raise SystemExit("找不到 Edge，无法截图")
    page = SHOTS / "banner.html"
    # 改文件名是为了绕开 GitHub / 浏览器的图片缓存，内容有更新时把这个编号往上加
    out = SHOTS / "banner-v3.png"
    page.write_text(BANNER_HTML, encoding="utf-8")
    out.unlink(missing_ok=True)
    command = [
        str(EDGE), "--headless=new", "--disable-gpu", "--no-sandbox", "--hide-scrollbars",
        "--force-device-scale-factor=1.5", f"--window-size={WIDTH},{HEIGHT}",
        f"--screenshot={out}", page.as_uri(),
    ]
    result = subprocess.run(command, capture_output=True, timeout=180)
    page.unlink(missing_ok=True)
    if not out.exists():
        raise SystemExit(f"截图失败 rc={result.returncode}: {result.stderr[:200]!r}")
    print(f"{out.name}  {out.stat().st_size // 1024} KB")


if __name__ == "__main__":
    sys.exit(main())
