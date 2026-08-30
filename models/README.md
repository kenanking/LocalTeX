# LocalTeX model card

Weights stay on disk, not in git. Install from the [`v0.0.0` GitHub Release](https://github.com/kenanking/LocalTeX/releases/tag/v0.0.0):

```bash
./scripts/download-models.sh
```

Destination is `$LOCALTEX_MODELS`, else `~/.local/share/localtex/models` (Windows: `%LOCALAPPDATA%\localtex\models`). Both packs use the same channel: GitHub release tarball, `SHA256SUMS`, lazy ONNX load, clear error if a file is missing.

`manifest.json` on the release is the machine-readable copy of this card.

## OpenDoc-0.1B ship (printed snips)

Tarball: `opendoc-0.1b-ship.tar.gz` (~244 MB unpacked).

| File | Role |
|---|---|
| `layout.onnx` | PP-DocLayoutV2, freeze-fold, image-only |
| `encoder.onnx` | UniRec-0.1B encoder |
| `decoder.onnx` | UniRec-0.1B decoder (GQA, `cross_kt_0` + `seqlens_k`) |
| `unirec_tokenizer_mapping.json` | UniRec tokenizer |

Strategy: layout-freeze-fold + INT8 conv weights + WOQ INT8 MatMul + decoder GQA.

## inktex WOQ (draw-a-formula)

Tarball: `inktex-woq.tar.gz` (~23 MB unpacked). Unpacks into `handwriting/`. Release also publishes the two ONNX files as `inktex-encoder.onnx` and `inktex-decoder-step.onnx` so they do not collide with UniRec `encoder.onnx` / `decoder.onnx` at the release root.

| File | Role |
|---|---|
| `handwriting/inktex-encoder.onnx` | Stroke encoder (12-dim point features, KV memory) |
| `handwriting/inktex-decoder-step.onnx` | Autoregressive decoder step (greedy, KV cache) |
| `handwriting/inktex-vocab.json` | LaTeX vocab (258 tokens, JSON key order is the id) |

Strategy: weights-only quantization (MatMulConstBOnly), same family as the OpenDoc ship MatMul WOQ. Input is online ink `(x, y, t)`, not a raster. Output is bare LaTeX. Source export is ink2tex `full-v1-woq`.
