# ONNX model identity plan

## Goal

Make model/version mismatches diagnosable without weakening lazy loading or
duplicating the release manifest inside the application binary.

## Artifact contract

Stamp final ONNX files only after export, graph optimization, and quantization.
Every file in a pack carries these string properties:

- `localtex.metadata_schema=1`
- `localtex.pack_id`
- `localtex.component`
- `localtex.model_family`
- `localtex.contract`
- `localtex.precision`
- `localtex.source_revision`

Components are unique within each pack: `layout`, `unirec_encoder`, and
`unirec_decoder` for OpenDoc; `inktex_encoder` and `inktex_decoder_step` for
handwriting. Self hashes are intentionally excluded because writing one would
change the file being hashed. Release `SHA256SUMS` authenticates the tarballs
and manifest, while the manifest covers non-ONNX files and declares the pair of
packs intended to run together.

## Runtime states

Model sessions remain lazy. Settings reads the release manifest at startup and
shows **Declared** for complete packs. On first page or handwriting inference,
LocalTeX reads metadata from the sessions it has already opened:

1. all files must use schema 1;
2. every component must match its filename/role;
3. all ONNX files in the pack must declare the same pack ID;
4. that ID must match the release manifest.

Success becomes **Verified**. A legacy pack with no LocalTeX keys becomes
**Unstamped**. Partial metadata, mixed pack IDs, incorrect components, an
unsupported schema, or disagreement with the manifest becomes **Mismatch**.
These states are diagnostic and do not replace the existing structural tensor
contract checks performed while constructing each pipeline.

## Release procedure

1. Copy the evaluated model directories to new dated pack IDs.
2. Run `ocr-pipeline/scripts/stamp_model_metadata.py` and confirm the graph
   protobuf digest remains unchanged for every ONNX file.
3. Run Python ONNX Runtime and Rust end-to-end smoke tests.
4. Create the two dated tarballs with symlinks dereferenced so each archive is
   self-contained, update `models/manifest.json`, and regenerate `SHA256SUMS`
   for both tarballs plus the manifest.
5. Upload those four files to the `v0.0.0` release while retaining older packs
   as explicit rollback assets.
6. Exercise a fresh download and run the Linux bundle script locally.

## Rollback and compatibility

Older unstamped packs continue to run and are visibly marked **Unstamped**.
Rolling back requires changing both download scripts, the model card, and the
release manifest together. A future metadata schema must use a new schema value
and add an explicit runtime compatibility path; silently accepting unknown
schemas is not allowed.
