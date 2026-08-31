# LocalTeX model card

Weights stay on disk, not in git. Layout matches ocr-pipeline: dated version
directories plus `current/` links. Install from the [`v0.0.0` GitHub Release](https://github.com/kenanking/LocalTeX/releases/tag/v0.0.0):

```bash
./scripts/download-models.sh
```

Destination is `$LOCALTEX_MODELS`, else `~/.local/share/localtex/models` (Windows: `%LOCALAPPDATA%\localtex\models`).

## Directory layout

```
models/
  current/opendoc      → ../opendoc_int8_20260827_45af38b
  current/handwriting  → ../handwriting_e10_20260831_cf27b99
  opendoc_int8_20260827_45af38b/
  handwriting_e10_20260831_cf27b99/
```

The app loads only `current/opendoc` and `current/handwriting`. Naming is `<name>_<form>_<YYYYMMDD>_<git-short>`.

`manifest.json` on the release is the machine-readable copy of this card.

## OpenDoc-0.1B (`opendoc_int8_20260827_45af38b`)

Tarball: `opendoc_int8_20260827_45af38b.tar.gz` (~244 MB unpacked).

| File | Role |
|---|---|
| `layout.onnx` | PP-DocLayoutV2, freeze-fold, image-only |
| `encoder.onnx` | UniRec-0.1B encoder |
| `decoder.onnx` | UniRec-0.1B decoder (GQA, `cross_kt_0` + `seqlens_k`) |
| `unirec_tokenizer_mapping.json` | UniRec tokenizer |

Strategy: layout-freeze-fold + INT8 conv weights + WOQ INT8 MatMul + decoder GQA.

## Handwriting (`handwriting_e10_20260831_cf27b99`)

Tarball: `handwriting_e10_20260831_cf27b99.tar.gz` (~23 MB unpacked). Filenames match ocr-pipeline. The packs live in separate directories, so they do not collide with UniRec `encoder.onnx` / `decoder.onnx`.

Rust/WOQ scores: MW 63.47 / C23 50.87.

| File | Role |
|---|---|
| `encoder.onnx` | Stroke encoder (12-dim point features, KV memory) |
| `decoder_step.onnx` | Autoregressive decoder step (greedy, KV cache) |
| `vocab.json` | LaTeX vocab (258 tokens, JSON key order is the id) |

Strategy: weights-only quantization (MatMulConstBOnly). Input is online ink `(x, y, t)`, not a raster. Output is bare LaTeX.
