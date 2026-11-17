# Workflows for specific kinds of work

Moved out of CLAUDE.md (2026-11-17) so the always-loaded instructions stay small; read the section for
the work at hand (`tools/doc docs/workflows.md <title>`).

## Workflow for a new resource format

1. `mwtool extract` a bundle with the type; hexdump `chunk0` (and `chunk1` if it has a body) plus
   `imports.txt`. Import offsets show where the pointer-to-resource fields are.
2. Survey **all** instances across the install with `mwtool survey <Type>` to separate constants, pointers,
   import slots, counts, floats and enums.
3. Check burnout.wiki and DGIorio's `nfsmw_exporter` (its `write_*` functions show layouts) for prior
   knowledge.
4. Write the parser in `mw-formats` with a layout table in the doc comment. Use `mw_bundle::Reader`
   (bounds-checked) and `Resource::import_at(offset)` for resource pointers.
5. Verify against every instance, plus one ground-truth check (visual or real-world value).
6. Record findings and open questions in `docs/research-log.md`; move settled facts into `docs/formats.md`.

## Shader work

The game's shaders are DXBC: port the real maths instead of guessing. Run `mwtool shaders` to find a family's
samplers, parameters and vertex layout, and `mwtool shader-asm <name>` for the pixel and vertex programs (the
longest `ps_4_0` is usually the main G-buffer pass; render target 1 holds `sqrt(albedo)`). Ported families live
in `crates/mostrusted/src/layered.wgsl`, selected by `materials::Look`.

## Audio work

I can't listen to audio, so verify decoders numerically (see the research log): spectral flatness against
white noise, sample-to-sample smoothness (diff/RMS), discontinuities at codec frame or grain boundaries, and
spectral centroid against RPM for engine sounds. Export with `mwtool audio` and analyse the `.wav` in Python.
The engine's A/B switches (`MW_AUDIO_*`, `MW_SOUND_TRACE`) are in docs/commands.md "Audio".
