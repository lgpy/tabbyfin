"""Writes the launcher icon's foreground, the mascot, as an Android vector drawable.

    python packaging/android/make-icon.py

Reads `packaging/tabbyfin.svg` and writes `src/main/res/drawable/ic_launcher_foreground.xml`:
the same paths, polygons and ellipses, with the SVG's gradients as `aapt:attr` gradients. A
vector stays sharp at every density, and the build needs no SVG renderer.

An adaptive icon is 108 dp square, of which a launcher's mask may show as little as the circle
66 dp across in the middle, so the mascot is scaled to fit inside that circle, tentacle tips
and ears included. The background (`ic_launcher_background.xml`) is drawn by hand.
"""

import math
import re
import xml.etree.ElementTree as ET
from pathlib import Path

HERE = Path(__file__).resolve().parent
SVG = HERE.parent / "tabbyfin.svg"
OUT = HERE / "src/main/res/drawable/ic_launcher_foreground.xml"

NS = {"svg": "http://www.w3.org/2000/svg"}
CANVAS = 108.0
# The mascot's farthest point from its middle lands this far from the canvas's centre: inside
# the 33-dp safe circle, with a point to spare.
REACH = 32.0


def colour(hex_colour: str, opacity: float = 1.0) -> str:
    hex_colour = hex_colour.lstrip("#")
    if len(hex_colour) == 3:
        hex_colour = "".join(c * 2 for c in hex_colour)
    return f"#{round(opacity * 255):02X}{hex_colour.upper()}"


def numbers(text: str) -> list[float]:
    return [float(n) for n in re.findall(r"-?\d+(?:\.\d+)?", text)]


def polygon_path(points: str) -> str:
    values = numbers(points)
    pairs = [f"{values[i]:g},{values[i + 1]:g}" for i in range(0, len(values), 2)]
    return "M" + " L".join(pairs) + " Z"


def ellipse_path(cx: float, cy: float, rx: float, ry: float) -> str:
    return (
        f"M{cx - rx:g},{cy:g} A{rx:g},{ry:g} 0 1,0 {cx + rx:g},{cy:g} "
        f"A{rx:g},{ry:g} 0 1,0 {cx - rx:g},{cy:g} Z"
    )


def path_points(d: str) -> list[tuple[float, float]]:
    """Every coordinate pair in an absolute path: its ends and control points, which bound it."""
    values = numbers(d)
    return [(values[i], values[i + 1]) for i in range(0, len(values) - 1, 2)]


def gradient_stops(element: ET.Element) -> str:
    items = []
    for stop in element.findall("svg:stop", NS):
        opacity = float(stop.get("stop-opacity", "1"))
        items.append(
            f'                <item android:offset="{float(stop.get("offset")):g}" '
            f'android:color="{colour(stop.get("stop-color"), opacity)}" />'
        )
    return "\n".join(items)


def main() -> None:
    root = ET.parse(SVG).getroot()
    defs = {element.get("id"): element for element in root.iter() if element.get("id")}
    tentacle_gradient = defs["tentGrad"]
    blush_gradient = defs["blushGrad"]

    # How far the drawing reaches, from its strokes' outer edges.
    reach_points: list[tuple[float, float, float]] = []
    body: list[str] = []

    tentacles = defs["tentacles"]
    width = float(tentacles.get("stroke-width"))
    for path in tentacles.findall("svg:path", NS):
        d = path.get("d")
        reach_points += [(x, y, width / 2) for x, y in path_points(d)]
        body.append(
            f"""        <path
            android:pathData="{d}"
            android:strokeWidth="{width:g}"
            android:strokeLineCap="round"
            android:strokeLineJoin="round">
            <aapt:attr name="android:strokeColor">
                <gradient
                    android:type="linear"
                    android:startX="{float(tentacle_gradient.get('x1')):g}"
                    android:startY="{float(tentacle_gradient.get('y1')):g}"
                    android:endX="{float(tentacle_gradient.get('x2')):g}"
                    android:endY="{float(tentacle_gradient.get('y2')):g}">
{gradient_stops(tentacle_gradient)}
                </gradient>
            </aapt:attr>
        </path>"""
        )

    head = defs["head"]
    reach_points += [(x, y, 0) for x, y in path_points(head.get("d"))]
    body.append(
        f"""        <path
            android:fillColor="{colour(head.get('fill'))}"
            android:pathData="{head.get('d')}" />"""
    )

    for ear in ("ear-left", "ear-right"):
        for polygon in defs[ear].findall("svg:polygon", NS):
            stroke = float(polygon.get("stroke-width"))
            reach_points += [(x, y, stroke / 2) for x, y in path_points(polygon.get("points"))]
            body.append(
                f"""        <path
            android:fillColor="{colour(polygon.get('fill'))}"
            android:strokeColor="{colour(polygon.get('stroke'))}"
            android:strokeWidth="{stroke:g}"
            android:strokeLineJoin="round"
            android:pathData="{polygon_path(polygon.get('points'))}" />"""
            )

    # The blush's gradient is in each ellipse's bounding box, so it's an ellipse too: a circle
    # with a round gradient, squashed by its group.
    for ellipse in defs["blush"].findall("svg:ellipse", NS):
        cx, cy, rx, ry = (float(ellipse.get(k)) for k in ("cx", "cy", "rx", "ry"))
        body.append(
            f"""        <group
            android:translateX="{cx:g}"
            android:translateY="{cy:g}"
            android:scaleY="{ry / rx:.4f}">
            <path android:pathData="{ellipse_path(0, 0, rx, rx)}">
                <aapt:attr name="android:fillColor">
                    <gradient
                        android:type="radial"
                        android:centerX="0"
                        android:centerY="0"
                        android:gradientRadius="{rx:g}">
    {gradient_stops(blush_gradient)}
                    </gradient>
                </aapt:attr>
            </path>
        </group>"""
        )

    for ellipse in defs["eyes"].findall("svg:ellipse", NS):
        cx, cy, rx, ry = (float(ellipse.get(k)) for k in ("cx", "cy", "rx", "ry"))
        body.append(
            f"""        <path
            android:fillColor="{colour(defs['eyes'].get('fill'))}"
            android:pathData="{ellipse_path(cx, cy, rx, ry)}" />"""
        )

    mouth = defs["mouth"]
    body.append(
        f"""        <path
            android:pathData="{mouth.get('d')}"
            android:strokeColor="{colour(mouth.get('stroke'))}"
            android:strokeWidth="{float(mouth.get('stroke-width')):g}"
            android:strokeLineCap="round"
            android:strokeLineJoin="round" />"""
    )

    # The middle of the drawing's bounds goes to the canvas's centre, and the point farthest
    # from it to REACH.
    xs = [x for x, _, pad in reach_points for x in (x - pad, x + pad)]
    ys = [y for _, y, pad in reach_points for y in (y - pad, y + pad)]
    mid_x, mid_y = (min(xs) + max(xs)) / 2, (min(ys) + max(ys)) / 2
    farthest = max(math.hypot(x - mid_x, y - mid_y) + pad for x, y, pad in reach_points)
    scale = REACH / farthest
    translate_x = CANVAS / 2 - mid_x * scale
    translate_y = CANVAS / 2 - mid_y * scale

    contents = "\n".join(body)
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(
        f"""<?xml version="1.0" encoding="utf-8"?>
<!-- Made by packaging/android/make-icon.py from packaging/tabbyfin.svg; don't edit by hand. -->
<vector xmlns:android="http://schemas.android.com/apk/res/android"
    xmlns:aapt="http://schemas.android.com/aapt"
    android:width="{CANVAS:g}dp"
    android:height="{CANVAS:g}dp"
    android:viewportWidth="{CANVAS:g}"
    android:viewportHeight="{CANVAS:g}">
    <group
        android:translateX="{translate_x:.4f}"
        android:translateY="{translate_y:.4f}"
        android:scaleX="{scale:.6f}"
        android:scaleY="{scale:.6f}">
{contents}
    </group>
</vector>
""",
        encoding="utf-8",
        newline="\n",
    )
    print(f"wrote {OUT.relative_to(HERE.parent.parent)} (scale {scale:.5f})")


if __name__ == "__main__":
    main()
