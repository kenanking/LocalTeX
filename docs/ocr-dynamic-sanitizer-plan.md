# Dynamic layout and conservative LaTeX normalization

## Goal

Use one dynamically shaped PP-DocLayoutV2 model in the Python reference
pipeline, the standalone Rust pipeline, and LocalTeX, while keeping LaTeX
postprocessing conservative and identical across implementations.

The normalization layer is an error-repair boundary, not a style rewriter. It
must not guess mathematical meaning, insert `\left` / `\right`, reinterpret
vertical bars, rewrite operators, or manufacture an equation number from OCR
text alone.

## Pipeline contract

1. Invert dark inputs using the existing heuristic.
2. Choose a layout bucket from the original aspect ratio:
   - `width / height < 4`: `800 x 800` (`H x W`)
   - `width / height >= 4`: `320 x 1280` (`H x W`)
3. Run the raw dynamic layout graph (`logits`, `pred_boxes`, `order_logits`).
4. Apply sigmoid scores, reading-order voting, global top-query selection,
   coordinate restoration, thresholding, and overlap filtering identically in
   Python and Rust.
5. Recognize each layout crop with UniRec.
6. Apply only the normalization rules below.
7. Attach `\tag{...}` only when an independently detected
   `formula_number` block passes the geometric and confidence checks.

The layout session disables ONNX Runtime's memory pattern because its input
shape changes between buckets. Encoder and decoder sessions retain memory
patterns because their contracts remain reusable.

## Normalization rules

Allowed repairs are deliberately small and idempotent:

- remove model control tokens;
- rewrite complete control words only: `\bm` to `\boldsymbol`,
  `\varmathbb` to `\mathbb`, and `\upmu` to `\mu`;
- repair the exact split token `\in fty` to `\infty`;
- repair malformed sized delimiters such as `\Bigg{[}` to `\Bigg[`;
- canonicalize one display result as `$$...$$`;
- join multiple OCR `\[...\]` lines with a TeX line break;
- reject a candidate rewrite when braces, `\left` / `\right`, or environments
  become unbalanced.

Command scanning uses full token boundaries. In particular, `\mu` must never
corrupt `\multicolumn`. Existing valid delimiters and author style are kept.

## Equation-number policy

Recognized text such as a trailing `(20)` is not sufficient evidence for an
equation number: it may be a function call or part of the expression. The
pipeline first removes unverified `\tag` and line-end numeric parentheses from
display-formula recognition. A tag is added back only when layout supplies a
`formula_number` block satisfying all of these gates:

- layout score at least `0.65`;
- number width at most `20%` of formula width;
- vertical center within a `25%` formula-height band;
- horizontal overlap at most `70%` of number width.

Bounded overlap is allowed because the dynamic detector can put the number box
slightly inside the formula box. A high-confidence number crop that decodes to
invalid text is retried once on its exact layout rectangle in the standalone
pipeline; invalid retry output is discarded rather than emitted as `$$$$`.

## Cross-language drift prevention

The Python and standalone Rust implementations share
`ocr-pipeline/tests/latex_sanitize_cases.json`. Every case asserts exact output
and idempotence. LocalTeX has matching unit coverage for dynamic bucket
selection, conservative number handling, tag injection, and command behavior.

Any future normalization rule should be accepted only when it is:

1. justified by a repeated model error;
2. recognizable without interpreting mathematical semantics;
3. idempotent;
4. represented in the shared golden cases;
5. verified in Python, standalone Rust, and LocalTeX.

## Evaluation protocol and result

The current model is
`opendoc_dynamic_int8_20260903_d8c4e76/layout.onnx`, SHA-256
`21eb60d0ac5b1f410724d5b0506be767ed22a053d0cf795f4b77548e1a325230`.
This revision adds metadata only; its inference graph is identical to the
evaluated 20260902 artifact.
Its position-embedding trigonometric subgraph is explicitly FP32 so the graph
loads in both Python ORT and the Rust ORT build used by LocalTeX.

### Layout regression

On 185 stratified OmniDocBench pages, dynamic INT8 versus the previous static
ship model achieved `0.998088` micro recall and `0.998565` micro precision at
IoU 0.5, with `0.998380` mean matched IoU. Three pages fell below 0.95 recall;
these remain the first candidates for a targeted layout regression set.

On the paired 30-page end-to-end subset, dynamic INT8 changed mean similarity
from `0.743078` to `0.743589` (delta `+0.000511`, bootstrap 95% CI
`[-0.002930, +0.004698]`). Peak RSS fell from `2064.09 MiB` to `1958.64 MiB`;
layout latency changed from `0.229149 s` to `0.243550 s` per page.

### Normalization regression

With the conservative sanitizer enabled on the same 30 pages, mean similarity
was `0.741192`, a delta of `-0.002398` from the earlier dynamic run. Twenty of
30 pages had exactly the same similarity, no page lost more than `0.0351`, and
the small lexical decrease is consistent with canonical command spelling such
as `\boldsymbol` rather than `\mathbf`; it is not evidence of a layout
regression.

On five Mathpix-reference samples, mean similarity was `0.8282` in Python and
`0.8395` in standalone Rust. This metric is diagnostic only: table renderings
and other equivalent LaTeX/HTML representations make raw character similarity
an unsuitable release gate across engines.

### Targeted and extra samples

- Three problem cases: tags are `22`, none, and `19`, respectively. Case 1's
  two OCR display lines are now one valid multiline formula.
- Six extra-wide images: Python and Rust Markdown are exact matches on all six;
  detected tags are `20`, `23`, `25`, `5`, `6`, and `7`.
- Across Python and Rust outputs for both sets, all 22 extracted display
  formulas compiled successfully with `pdflatex` plus
  `amsmath, amssymb, mathtools, bm`.

## Deployment plan

1. Keep the old model pack available as a rollback artifact.
2. The dated model pack, Linux/Windows download scripts, release manifest,
   checksums, and model documentation were updated together on 2026-09-03.
   The previous OpenDoc asset remains available for manual rollback.
3. Roll out first to an opt-in channel and log only aggregate counters:
   selected bucket, layout box counts, zero-layout pages, retry counts, and
   inference latency. Do not log captured document content.
4. Compare at least one week of traffic with the previous release. Roll back
   if zero-layout frequency materially rises, P95 latency exceeds the product
   budget, crash/OOM rate increases, or the targeted worst-page set regresses.
5. Promote after the packaging smoke tests and both platform installers verify
   the exact manifest hash.

## Remaining risks

- Dynamic shapes trade a smaller on-disk model for shape-dependent temporary
  allocations; disabling only the layout memory pattern is intentional.
- The aspect-ratio boundary is a two-bucket policy, not arbitrary continuous
  resizing. New buckets require a measured improvement and their own memory
  benchmark.
- LaTeX compilation checks syntax and package availability, not semantic
  equivalence to the source image.
- The remaining major recognition failures are primarily layout misses or
  UniRec content errors. Aggressive postprocessing would hide symptoms while
  introducing silent mathematical changes, so it is outside this plan.
