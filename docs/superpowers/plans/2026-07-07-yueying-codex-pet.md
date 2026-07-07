# Yueying Codex Pet Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a complete Codex pet named 月影 from `/Users/ushopal/Downloads/1.jpg` and `/Users/ushopal/Downloads/2.jpg`.

**Architecture:** Use the installed hatch-pet scripts to prepare prompts, layout guides, atlas geometry, validation, contact sheet, previews, and package files. Use image generation only for the base pet and animation row strips, then process those selected images deterministically into the final `1536x1872` spritesheet.

**Tech Stack:** Python scripts from `/Users/ushopal/.codex/skills/hatch-pet/scripts`, built-in image generation, Pillow-based deterministic sprite processing, `jq`, Git.

## Global Constraints

- Always respond in Chinese-simplified.
- Default to the current branch and do not create a new branch.
- Do not overwrite existing pet folders.
- Do not keep reference image watermarks, background grids, subtitles, black bars, or readable text in generated pet art.
- Preserve the Q-style anime identity: burgundy side ponytail and segmented long braid, closed confident expression, black-and-gold cape, warm orange cape lining, gold beads, black boots, and star accents.
- Output under `pet-output/yueying-cape-dancer/`.
- Package files under `pet-output/yueying-cape-dancer/package/`.
- Verify `final/spritesheet.webp`, `final/validation.json`, `qa/contact-sheet.png`, `qa/previews/*.gif`, `package/pet.json`, and `package/spritesheet.webp`.

---

### Task 1: Prepare Run Folder

**Files:**
- Create: `pet-output/yueying-cape-dancer/run/pet_request.json`
- Create: `pet-output/yueying-cape-dancer/run/imagegen-jobs.json`
- Create: `pet-output/yueying-cape-dancer/run/prompts/base-pet.md`
- Create: `pet-output/yueying-cape-dancer/run/prompts/rows/*.md`
- Create: `pet-output/yueying-cape-dancer/run/references/reference-01.png`
- Create: `pet-output/yueying-cape-dancer/run/references/reference-02.png`

**Interfaces:**
- Consumes: `/Users/ushopal/Downloads/1.jpg`, `/Users/ushopal/Downloads/2.jpg`.
- Produces: a hatch-pet run manifest whose ready jobs can be completed by copying selected generated images into `decoded/`.

- [ ] **Step 1: Prepare hatch-pet run**

Run:

```bash
python /Users/ushopal/.codex/skills/hatch-pet/scripts/prepare_pet_run.py \
  --pet-name "月影" \
  --description "参考图里的 Q 版酒红长辫黑金披风角色宠物，闭眼从容，金色珠饰和星形装饰清晰可读。" \
  --reference /Users/ushopal/Downloads/1.jpg \
  --reference /Users/ushopal/Downloads/2.jpg \
  --output-dir /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run \
  --pet-notes "保持参考图的 Q 版动漫人物宠物：酒红色侧高束发和分段长辫，闭眼从容小表情，黑色短上衣，黑金披风，暖橙色内衬，金色珠饰，黑色短靴，星形小饰物；不要水印、网格、字幕、黑边或任何文字。" \
  --style-preset auto \
  --style-notes "贴近参考图的干净 Q 版动漫贴纸风，粗细适中的深色描边，柔和阴影，紧凑全身轮廓；透明处理前使用纯色 chroma key 背景。" \
  --force
```

Expected: command exits with status `0` and creates `pet-output/yueying-cape-dancer/run/imagegen-jobs.json`.

- [ ] **Step 2: Inspect ready jobs**

Run:

```bash
jq '.jobs[] | {id, kind, status, depends_on, prompt_file, input_images, output_path}' /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/imagegen-jobs.json
```

Expected: `base` has no dependencies, and every row job depends on the canonical base.

### Task 2: Generate Base And Row Strips

**Files:**
- Create: `pet-output/yueying-cape-dancer/run/decoded/base.png`
- Create: `pet-output/yueying-cape-dancer/run/references/canonical-base.png`
- Create: `pet-output/yueying-cape-dancer/run/decoded/idle.png`
- Create: `pet-output/yueying-cape-dancer/run/decoded/running-right.png`
- Create: `pet-output/yueying-cape-dancer/run/decoded/running-left.png`
- Create: `pet-output/yueying-cape-dancer/run/decoded/waving.png`
- Create: `pet-output/yueying-cape-dancer/run/decoded/jumping.png`
- Create: `pet-output/yueying-cape-dancer/run/decoded/failed.png`
- Create: `pet-output/yueying-cape-dancer/run/decoded/waiting.png`
- Create: `pet-output/yueying-cape-dancer/run/decoded/running.png`
- Create: `pet-output/yueying-cape-dancer/run/decoded/review.png`
- Modify: `pet-output/yueying-cape-dancer/run/imagegen-jobs.json`

**Interfaces:**
- Consumes: prompt files and input images listed in `imagegen-jobs.json`.
- Produces: complete decoded base image and all row strip images ready for deterministic extraction.

- [ ] **Step 1: Generate base**

Use the built-in image generation tool with the text from `prompts/base-pet.md` and both reference images as visual guidance. Save the selected output to:

```text
/Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/decoded/base.png
```

Expected: one full-body centered pet on a flat chroma-key background, with no text, no watermark, no scenery, and no detached effects.

- [ ] **Step 2: Mark base complete**

Run:

```bash
cp /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/decoded/base.png /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/references/canonical-base.png
```

Then update `imagegen-jobs.json` so the `base` job has `status: "complete"` and a `source_path` pointing to the generated source.

Expected: row jobs become ready because `base` is complete.

- [ ] **Step 3: Generate row strips**

Generate these row jobs from their prompt files, using the canonical base and row layout guide listed in `imagegen-jobs.json`:

```text
idle
running-right
running-left
waving
jumping
failed
waiting
running
review
```

Expected: each decoded row strip has the exact requested frame count, flat chroma-key background, complete unclipped poses, and the same 月影 identity.

### Task 3: Compose And Validate Atlas

**Files:**
- Create: `pet-output/yueying-cape-dancer/run/frames/`
- Create: `pet-output/yueying-cape-dancer/run/final/spritesheet.png`
- Create: `pet-output/yueying-cape-dancer/run/final/spritesheet.webp`
- Create: `pet-output/yueying-cape-dancer/run/final/validation.json`
- Create: `pet-output/yueying-cape-dancer/run/qa/review.json`
- Create: `pet-output/yueying-cape-dancer/run/qa/contact-sheet.png`
- Create: `pet-output/yueying-cape-dancer/run/qa/previews/*.gif`

**Interfaces:**
- Consumes: all files in `run/decoded/`.
- Produces: validated atlas, contact sheet, and animation previews.

- [ ] **Step 1: Extract frames**

Run:

```bash
python /Users/ushopal/.codex/skills/hatch-pet/scripts/extract_strip_frames.py \
  --decoded-dir /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/decoded \
  --output-dir /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/frames \
  --states all \
  --method auto
```

Expected: `run/frames/frames-manifest.json` exists.

- [ ] **Step 2: Inspect frames**

Run:

```bash
python /Users/ushopal/.codex/skills/hatch-pet/scripts/inspect_frames.py \
  --frames-root /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/frames \
  --json-out /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/qa/review.json \
  --require-components
```

Expected: `review.json` contains no errors.

- [ ] **Step 3: Compose atlas**

Run:

```bash
python /Users/ushopal/.codex/skills/hatch-pet/scripts/compose_atlas.py \
  --frames-root /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/frames \
  --output /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/final/spritesheet.png \
  --webp-output /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/final/spritesheet.webp
```

Expected: WebP atlas exists at `run/final/spritesheet.webp`.

- [ ] **Step 4: Validate atlas**

Run:

```bash
python /Users/ushopal/.codex/skills/hatch-pet/scripts/validate_atlas.py \
  /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/final/spritesheet.webp \
  --json-out /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/final/validation.json
```

Expected: validation reports the expected `1536x1872` atlas with valid transparent unused cells.

- [ ] **Step 5: Create QA media**

Run:

```bash
python /Users/ushopal/.codex/skills/hatch-pet/scripts/make_contact_sheet.py \
  /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/final/spritesheet.webp \
  --output /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/qa/contact-sheet.png
python /Users/ushopal/.codex/skills/hatch-pet/scripts/render_animation_previews.py \
  --frames-root /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/frames \
  --output-dir /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/qa/previews
```

Expected: contact sheet and nine preview GIFs exist.

### Task 4: Package Pet

**Files:**
- Create: `pet-output/yueying-cape-dancer/package/pet.json`
- Create: `pet-output/yueying-cape-dancer/package/spritesheet.webp`
- Create: `pet-output/yueying-cape-dancer/run/qa/run-summary.json`

**Interfaces:**
- Consumes: `run/final/spritesheet.webp`, `run/final/validation.json`, `run/qa/contact-sheet.png`, `run/qa/review.json`.
- Produces: package directory that Codex can install as a custom pet.

- [ ] **Step 1: Copy package files**

Run:

```bash
mkdir -p /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/package
cp /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/final/spritesheet.webp /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/package/spritesheet.webp
jq -n \
  --arg id "yueying-cape-dancer" \
  --arg displayName "月影" \
  --arg description "Q 版酒红长辫黑金披风角色宠物，闭眼从容，带金色珠饰和星形装饰。" \
  '{id: $id, displayName: $displayName, description: $description, spritesheetPath: "spritesheet.webp"}' \
  > /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/package/pet.json
```

Expected: package folder contains `pet.json` and `spritesheet.webp`.

- [ ] **Step 2: Write run summary**

Run:

```bash
jq -n \
  --arg run_dir "/Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run" \
  --arg spritesheet "/Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/final/spritesheet.webp" \
  --arg validation "/Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/final/validation.json" \
  --arg contact_sheet "/Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/qa/contact-sheet.png" \
  --arg review "/Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/qa/review.json" \
  --arg package "/Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/package" \
  '{ok: true, run_dir: $run_dir, spritesheet: $spritesheet, validation: $validation, contact_sheet: $contact_sheet, review: $review, package: $package}' \
  > /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/qa/run-summary.json
```

Expected: run summary points to all deliverables.

- [ ] **Step 3: Final verification**

Run:

```bash
test -f /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/package/pet.json
test -f /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/package/spritesheet.webp
test -f /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/qa/contact-sheet.png
test -f /Users/ushopal/workspace/myself/codex-pet/pet-output/yueying-cape-dancer/run/final/validation.json
```

Expected: all commands exit with status `0`.
