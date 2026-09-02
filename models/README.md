# LocalTeX model card

Weights stay on disk, not in git and not inside the Rust binary. Release
packages ship `opendoc/` and `handwriting/` beside the executable. From a
source checkout, install the dated tarballs on the
[`v0.0.0` GitHub Release](https://github.com/kenanking/LocalTeX/releases/tag/v0.0.0).

```bash
./scripts/download-models.sh
```

```powershell
.\scripts\download-models.ps1
```

Destination is `$LOCALTEX_MODELS`, else `~/.local/share/localtex/models` (Windows: `%LOCALAPPDATA%\localtex\models`).

## Directory layout

```
models/
  opendoc/
  handwriting/
  manifest.json
```

ocr-pipeline keeps dated directories plus `current/` because it holds
several pack versions. LocalTeX only installs the pair in use.

`manifest.json` on the release is the machine-readable copy of this card.

## OpenDoc-0.1B (`opendoc_dynamic_int8_20260902_45af38b`)

Tarball: `opendoc_dynamic_int8_20260902_45af38b.tar.gz` (~244 MB unpacked). Installed as `opendoc/`.

| File | Role |
|---|---|
| `layout.onnx` | PP-DocLayoutV2, dynamic H/W, raw layout outputs |
| `encoder.onnx` | UniRec-0.1B encoder |
| `decoder.onnx` | UniRec-0.1B decoder (GQA, `cross_kt_0` + `seqlens_k`) |
| `unirec_tokenizer_mapping.json` | UniRec tokenizer |

Strategy: dynamic layout with FP32 position trigonometry, INT8 conv weights,
WOQ INT8 MatMul, and decoder GQA. LocalTeX selects `800×800` for ordinary
pages and `1280×320` for images whose width/height ratio is at least 4.

## Handwriting (`handwriting_e10_20260831_cf27b99`)

Tarball: `handwriting_e10_20260831_cf27b99.tar.gz` (~23 MB unpacked). Installed as `handwriting/`. Filenames match ocr-pipeline. The packs live in separate directories, so they do not collide with UniRec `encoder.onnx` / `decoder.onnx`.

Rust/WOQ scores: MW 63.47 / C23 50.87.

| File | Role |
|---|---|
| `encoder.onnx` | Stroke encoder (12-dim point features, KV memory) |
| `decoder_step.onnx` | Autoregressive decoder step (greedy, KV cache) |
| `vocab.json` | LaTeX vocab (258 tokens, JSON key order is the id) |

Strategy: weights-only quantization (MatMulConstBOnly). Input is online ink `(x, y, t)`, not a raster. Output is bare LaTeX.
