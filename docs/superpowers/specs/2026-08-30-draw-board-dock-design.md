# Draw board dock toolbar

Approved from `/tmp/localtex-copy-ui/draw-board.html` (Dock variant). HTML stays throwaway.

## Problem

Draw a formula only supports freehand ink, Clear, Cancel, and Recognize. There is no pen/eraser switch, undo/redo, paper background, or keyboard shortcuts. The sheet chrome (44px topbar, 40px “Draw a formula” bar, 28px footer) is correct and must not be redesigned.

## Agreed UI

- **Dock** floats on the canvas: bottom-centered pill, white, 1px `#e6e6e6`, 6px radius, 32px icon buttons (same as the topbar).
- Tools left-to-right: **Pen (1)**, **Eraser (2)**, divider, **Undo (3)**, **Redo (4)**, then a settings-style segmented control **Dots / Lines / Blank**.
- Active tool uses `#eff6ff`. Disabled undo/redo use the existing 0.38 opacity.
- Digit badges: 9px monospace, `#6b6b6b`, bottom-right of each of the four tool buttons (Excalidraw).
- **Paper:** all three modes use cream `#fbfaf7`. Dots = 20px pitch, `#c8c4bc`. Lines = 28px ruled, `#c9d3e4`. Blank = cream only.
- Paper is a drawing aid. `traces()` / inktex still see stroke polylines only. Recognize rasterizes the library PNG as today (white/ink, not the cream grid).
- Sheet bar copy and buttons stay **Draw a formula / Clear / Cancel / Recognize**. Clear does not reset tool or paper.

## Ink model

- Pen appends polylines with `(x, y, t_ms)` as today.
- Eraser is **vector**, radius 7px: delete points (and split traces) near the pointer. Do not composite `destination-out` and do not send erase marks to inktex.
- Undo/redo are snapshot stacks of the polyline list. A mouse-up that changed ink pushes one undo entry and clears redo. A new gesture after undo clears redo. Clear empties ink and both stacks.

## Keys (DrawBoard context only)

| Chord | Action |
| --- | --- |
| `1` | Pen |
| `2` | Eraser |
| `3` | Undo |
| `4` | Redo |
| `ctrl-z` | Undo |
| `ctrl-shift-z` | Redo |

Not user-rebindable. Escape still closes the sheet. Do not add these to the settings shortcut catalog.

## Out of scope

- Changing inktex, OpenDoc, ingest, or stroke file format.
- Persisting paper/tool in prefs.
- Stroke width, colors, or extra tools.
- Wayland / claiming Windows or macOS verification from this host.
