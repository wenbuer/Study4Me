"""用参数方程画应用图标：一只抱着书躺在摇椅上的老鼠。

    python scripts/make_icon.py            # 只生成 design/icon.svg
    python scripts/make_icon.py --all      # 再光栅化各尺寸并写好 src-tauri/icons

只用数学曲线（椭圆弧、贝塞尔）构造。
"""

import math
import struct
import subprocess
import sys
import tempfile
from pathlib import Path

SIZE = 512
BG = "#0F4C3A"
INK = "#F6F1E7"
PAGE = "#EADFC6"
ROOT = Path(__file__).resolve().parent.parent
EDGE = r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe"
ICO_SIZES = (16, 24, 32, 48, 64, 128, 256)


def arc(cx, cy, rx, ry, a0, a1, steps=200):
    return [(cx + rx * math.cos(math.radians(a0 + (a1 - a0) * i / steps)),
             cy + ry * math.sin(math.radians(a0 + (a1 - a0) * i / steps)))
            for i in range(steps + 1)]


def ellipse(cx, cy, rx, ry, rot=0.0, steps=240):
    rad = math.radians(rot)
    pts = []
    for i in range(steps + 1):
        a = math.radians(360 * i / steps)
        x, y = rx * math.cos(a), ry * math.sin(a)
        pts.append((cx + x * math.cos(rad) - y * math.sin(rad),
                    cy + x * math.sin(rad) + y * math.cos(rad)))
    return pts


def bezier(p0, p1, p2, p3, steps=140):
    pts = []
    for i in range(steps + 1):
        t = i / steps
        u = 1 - t
        pts.append((
            u**3 * p0[0] + 3 * u**2 * t * p1[0] + 3 * u * t**2 * p2[0] + t**3 * p3[0],
            u**3 * p0[1] + 3 * u**2 * t * p1[1] + 3 * u * t**2 * p2[1] + t**3 * p3[1],
        ))
    return pts


def d(pts, close=False):
    return "M " + " L ".join(f"{x:.1f} {y:.1f}" for x, y in pts) + (" Z" if close else "")


def line(pts, width=13, color=INK, close=False):
    return (f'<path d="{d(pts, close)}" fill="none" stroke="{color}" stroke-width="{width}" '
            f'stroke-linecap="round" stroke-linejoin="round"/>')


def solid(pts, fill, width=13, stroke_color=INK, close=True):
    return (f'<path d="{d(pts, close)}" fill="{fill}" stroke="{stroke_color}" '
            f'stroke-width="{width}" stroke-linejoin="round"/>')


def build(responsive: bool = False) -> str:
    dims = 'width="100%" height="100%"' if responsive else f'width="{SIZE}" height="{SIZE}"'
    s = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {SIZE} {SIZE}" {dims}>',
         f'<rect width="{SIZE}" height="{SIZE}" rx="106" ry="106" fill="{BG}"/>']

    # ── 耳朵（两侧对称，先画，压在头后面）──
    s.append(solid(ellipse(146, 150, 60, 60), BG, width=14))
    s.append(solid(ellipse(366, 150, 60, 60), BG, width=14))

    # ── 头 ──
    s.append(solid(ellipse(256, 250, 112, 106), BG, width=15))

    # ── 眼睛：两个圆点，正正经经看书的样子 ──
    s.append(f'<circle cx="210" cy="234" r="17" fill="{INK}"/>')
    s.append(f'<circle cx="302" cy="234" r="17" fill="{INK}"/>')

    # ── 鼻子与嘴 ──
    s.append(solid(ellipse(256, 300, 19, 14), INK, width=0, stroke_color=INK))
    s.append(line(bezier((228, 334), (241, 350), (271, 350), (284, 334)), width=10))

    # ── 胡须：两侧各三条，短而对称 ──
    for dy in (-16, 0, 16):
        s.append(line([(154, 298 + dy * 0.6), (96, 288 + dy)], width=8))
        s.append(line([(358, 298 + dy * 0.6), (416, 288 + dy)], width=8))

    # ── 手里摊开的书 ──
    left = (bezier((100, 398), (156, 372), (208, 370), (256, 386), steps=70)
            + bezier((256, 386), (256, 412), (256, 440), (256, 458), steps=20)
            + bezier((256, 458), (208, 446), (156, 448), (100, 464), steps=70))
    right = (bezier((256, 386), (304, 370), (356, 372), (412, 398), steps=70)
             + bezier((412, 398), (412, 424), (412, 446), (412, 464), steps=20)
             + bezier((412, 464), (356, 448), (304, 446), (256, 458), steps=70))
    s.append(solid(left, PAGE, width=12))
    s.append(solid(right, PAGE, width=12))
    s.append(line([(256, 390), (256, 456)], width=10))

    # 书页上的字（浅色短线，示意而已）
    for index, y in enumerate((414, 430, 446)):
        s.append(line([(132, y), (226 - index * 18, y)], width=6, color="#CFC4A8"))
        s.append(line([(286 + index * 18, y), (380, y)], width=6, color="#CFC4A8"))

    s.append('</svg>')
    return "\n".join(s)


def build_mark(color: str = "currentColor") -> str:
    """界面左上角用的品牌标记：透明背景 + 单色线条。

    颜色可以写死（作为 SVG 文件给 img 用时），也可以是 currentColor（内联时交给 CSS）。
    """
    s = [f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {SIZE} {SIZE}" '
         f'width="{SIZE}" height="{SIZE}" fill="none" stroke="currentColor" '
         f'stroke-linecap="round" stroke-linejoin="round">']

    # 耳朵与头
    s.append(line(ellipse(150, 152, 76, 76), width=40, color="currentColor"))
    s.append(line(ellipse(362, 152, 76, 76), width=40, color="currentColor"))
    s.append(line(ellipse(256, 266, 126, 122), width=40, color="currentColor"))

    # 眼睛
    s.append('<circle cx="212" cy="248" r="27" fill="currentColor" stroke="none"/>')
    s.append('<circle cx="300" cy="248" r="27" fill="currentColor" stroke="none"/>')

    # 摊开的书：只有轮廓
    left = (bezier((112, 420), (172, 394), (220, 392), (256, 406), steps=60)
            + bezier((256, 406), (256, 430), (256, 456), (256, 476), steps=16)
            + bezier((256, 476), (220, 464), (172, 466), (112, 490), steps=60))
    right = (bezier((256, 406), (292, 392), (340, 394), (400, 420), steps=60)
             + bezier((400, 420), (400, 446), (400, 470), (400, 490), steps=16)
             + bezier((400, 490), (340, 466), (292, 464), (256, 476), steps=60))
    s.append(line(left, width=36, color="currentColor", close=True))
    s.append(line(right, width=36, color="currentColor", close=True))
    s.append(line([(256, 408), (256, 474)], width=30, color="currentColor"))

    s.append('</svg>')
    svg = "\n".join(s)
    if color != "currentColor":
        svg = svg.replace("currentColor", color)
    return svg


def rasterize(target_dir: Path, sizes) -> dict:
    """用无头 Edge 把 SVG 渲染成各尺寸 PNG（SVG 自适应视口）。"""
    responsive = target_dir / "__icon-responsive.svg"
    responsive.write_text(build(responsive=True), encoding="utf-8")
    uri = "file:///" + str(responsive).replace("\\", "/")

    made = {}
    for size in sizes:
        png = target_dir / f"__icon-{size}.png"
        png.unlink(missing_ok=True)
        # 每个尺寸用独立 profile：共用 profile 时多个无头实例会互相锁住。
        profile = Path(tempfile.gettempdir()) / f"edge-icon-{size}"
        command = [
            EDGE, "--headless=new", "--disable-gpu", "--no-sandbox", "--hide-scrollbars",
            "--no-first-run", "--disable-extensions", "--disable-background-networking",
            "--force-device-scale-factor=1", f"--user-data-dir={profile}",
            f"--window-size={size},{size}", f"--screenshot={png}", uri,
        ]
        for attempt in range(2):
            try:
                subprocess.run(command, capture_output=True, timeout=45)
            except subprocess.TimeoutExpired:
                pass
            if png.exists():
                break
        if png.exists():
            made[size] = png
        else:
            print(f"  ! {size}px 渲染失败，跳过")
    responsive.unlink(missing_ok=True)
    return made


def write_ico(pngs: dict, out_path: Path) -> None:
    """把多个尺寸的 PNG 打包成 ICO（Vista 之后支持内嵌 PNG）。"""
    items = [(size, pngs[size].read_bytes()) for size in sorted(pngs)]
    header = struct.pack("<HHH", 0, 1, len(items))
    offset = 6 + 16 * len(items)
    entries = b""
    payload = b""
    for size, blob in items:
        dim = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(blob), offset)
        offset += len(blob)
        payload += blob
    out_path.write_bytes(header + entries + payload)


def main() -> None:
    design = ROOT / "design"
    design.mkdir(parents=True, exist_ok=True)
    svg_path = design / "icon.svg"
    svg_path.write_text(build(), encoding="utf-8")
    print(f"svg   -> {svg_path}")

    mark_path = design / "mark.svg"
    mark_path.write_text(build_mark("currentColor"), encoding="utf-8")
    print(f"mark  -> {mark_path}")

    # 界面直接引用的那份：颜色写死为强调色，避开 currentColor 在 img 里失效的问题
    ui_mark = ROOT / "src" / "mark.svg"
    ui_mark.write_text(build_mark("#047857"), encoding="utf-8")
    print(f"mark  -> {ui_mark}")

    if "--all" not in sys.argv:
        return

    icons = ROOT / "src-tauri" / "icons"
    icons.mkdir(parents=True, exist_ok=True)
    made = rasterize(design, (256, 512))
    big = made.get(512) or made.get(256)
    (icons / "icon.png").write_bytes(big.read_bytes())
    print(f"png   -> {icons / 'icon.png'} ({big.name})")

    ico_sources = rasterize(design, ICO_SIZES)
    write_ico(ico_sources, icons / "icon.ico")
    print(f"ico   -> {icons / 'icon.ico'} ({len(ico_sources)} 个尺寸)")

    for png in made.values():
        png.unlink(missing_ok=True)
    for png in ico_sources.values():
        png.unlink(missing_ok=True)
    (design / "icon-preview.png").unlink(missing_ok=True)


if __name__ == "__main__":
    main()