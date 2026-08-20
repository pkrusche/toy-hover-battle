#!/usr/bin/env python3
"""Generate deterministic Stable Audio sound effects from prompts.json."""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import tempfile
from typing import Any, Callable

SAMPLE_RATE = 44_100
EXPECTED_NAMES = {
    "gun_fire", "rocket_launch", "shield_hit", "hull_hit", "explosion",
    "vehicle_collision", "rock_impact", "menu_move", "menu_confirm", "rocket_warning",
}


def load_manifest(path: Path) -> list[dict[str, Any]]:
    try:
        entries = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        raise ValueError(f"cannot read manifest {path}: {exc}") from exc
    if not isinstance(entries, list):
        raise ValueError("manifest must be a JSON array")
    required = {"name", "filename", "event", "prompt", "duration", "seed"}
    for index, entry in enumerate(entries):
        if not isinstance(entry, dict) or set(entry) != required:
            raise ValueError(f"entry {index} must contain exactly {sorted(required)}")
        if not all(isinstance(entry[key], str) and entry[key].strip() for key in ("name", "filename", "event", "prompt")):
            raise ValueError(f"entry {index} has an empty or non-string field")
        if Path(entry["filename"]).name != entry["filename"] or not entry["filename"].endswith(".wav"):
            raise ValueError(f"invalid WAV filename: {entry['filename']}")
        if not isinstance(entry["duration"], (int, float)) or not 0.05 <= entry["duration"] <= 30:
            raise ValueError(f"invalid duration for {entry['name']}")
        if not isinstance(entry["seed"], int) or isinstance(entry["seed"], bool) or entry["seed"] < 0:
            raise ValueError(f"invalid seed for {entry['name']}")
    names = [entry["name"] for entry in entries]
    filenames = [entry["filename"] for entry in entries]
    seeds = [entry["seed"] for entry in entries]
    if set(names) != EXPECTED_NAMES or len(names) != len(EXPECTED_NAMES):
        raise ValueError(f"manifest names must be exactly {sorted(EXPECTED_NAMES)}")
    if len(filenames) != len(set(filenames)):
        raise ValueError("manifest filenames must be unique")
    if len(seeds) != len(set(seeds)):
        raise ValueError("manifest seeds must be unique")
    return entries


def select_entries(entries: list[dict[str, Any]], names: list[str]) -> list[dict[str, Any]]:
    if not names:
        return entries
    by_name = {entry["name"]: entry for entry in entries}
    unknown = [name for name in names if name not in by_name]
    if unknown:
        raise ValueError(f"unknown effect(s): {', '.join(unknown)}")
    if len(names) != len(set(names)):
        raise ValueError("effect names may not be repeated")
    return [by_name[name] for name in names]


def prepare_audio(audio: Any, duration: float) -> Any:
    import numpy as np

    # Own the buffer before applying normalization and fades. Model and test
    # pipelines may retain or reuse the returned array between calls.
    data = np.array(audio, dtype=np.float32, copy=True).squeeze()
    if data.ndim == 1:
        data = np.stack((data, data))
    elif data.ndim != 2:
        raise ValueError(f"model returned audio with unexpected shape {data.shape}")
    if data.shape[0] != 2 and data.shape[1] == 2:
        data = data.T
    if data.shape[0] != 2:
        data = np.stack((data[0], data[0]))
    sample_count = round(duration * SAMPLE_RATE)
    if data.shape[1] < sample_count:
        data = np.pad(data, ((0, 0), (0, sample_count - data.shape[1])))
    data = data[:, :sample_count]
    peak = float(np.max(np.abs(data))) if data.size else 0.0
    target_peak = 10 ** (-1 / 20)
    if peak > 0:
        data *= target_peak / peak
    fade_samples = min(round(0.005 * SAMPLE_RATE), sample_count // 2)
    if fade_samples:
        fade = np.linspace(0.0, 1.0, fade_samples, endpoint=True, dtype=np.float32)
        data[:, :fade_samples] *= fade
        data[:, -fade_samples:] *= fade[::-1]
    return data.T


def _patch_scheduler_noise_sampler_floor() -> None:
    """Widen the Stable Audio scheduler's SDE noise sampler down to sigma=0.

    The checkpoint's scheduler ends sampling at sigma=0 (its default final_sigmas_type="zero"),
    but diffusers builds the scheduler's SDE noise sampler with a sigma_min floor (0.3) taken
    from the same config. Querying the sampler at the final step's sigma=0 falls outside that
    floor and sends torchsde's Brownian-tree search into unbounded recursion, crashing every
    real (non-dry-run) generation on its last step -- reproduced independent of device and
    dtype (CPU/MPS, float32/float16). An earlier attempt to work around this by switching
    final_sigmas_type to "sigma_min" instead duplicates the schedule's last two sigma values,
    producing a zero-length solver step whose output is NaN. Widening the sampler's floor to 0
    keeps the final step's query in bounds without altering the schedule.
    """
    import diffusers.schedulers.scheduling_cosine_dpmsolver_multistep as scheduler_module

    if getattr(scheduler_module, "_toy_assets_zero_floor_patch", False):
        return
    original_sampler = scheduler_module.BrownianTreeNoiseSampler

    class ZeroFloorNoiseSampler(original_sampler):
        def __init__(self, x, sigma_min, sigma_max, seed=None, transform=lambda t: t):
            super().__init__(x, 0.0, sigma_max, seed=seed, transform=transform)

    scheduler_module.BrownianTreeNoiseSampler = ZeroFloorNoiseSampler
    scheduler_module._toy_assets_zero_floor_patch = True


def default_pipeline_loader(model_id: str, device: str) -> tuple[Any, str]:
    import torch
    from diffusers import StableAudioPipeline

    _patch_scheduler_noise_sampler_floor()

    if device == "auto":
        device = "cuda" if torch.cuda.is_available() else "mps" if torch.backends.mps.is_available() else "cpu"
    dtype = torch.float16 if device in {"cuda", "mps"} else torch.float32
    pipeline = StableAudioPipeline.from_pretrained(model_id, torch_dtype=dtype)
    return pipeline.to(device), device


def generate(
    entries: list[dict[str, Any]], output_dir: Path, force: bool, model_id: str,
    device: str, steps: int, guidance: float,
    pipeline_loader: Callable[[str, str], tuple[Any, str]] = default_pipeline_loader,
) -> list[Path]:
    pending = [entry for entry in entries if force or not (output_dir / entry["filename"]).exists()]
    for entry in entries:
        if entry not in pending:
            print(f"Skipping existing {output_dir / entry['filename']}")
    if not pending:
        print("All requested sound effects already exist")
        return []

    import soundfile as sf
    import torch

    print(f"Loading pipeline {model_id} (requested device={device})...")
    pipeline, resolved_device = pipeline_loader(model_id, device)
    print(f"Pipeline loaded on {resolved_device}")
    output_dir.mkdir(parents=True, exist_ok=True)
    written = []
    total = len(pending)
    for index, entry in enumerate(pending, start=1):
        print(f"[{index}/{total}] Generating {entry['name']} ({entry['duration']:.2f}s, seed {entry['seed']})")
        generator_device = resolved_device if resolved_device != "mps" else "cpu"
        generator = torch.Generator(device=generator_device).manual_seed(entry["seed"])
        result = pipeline(
            entry["prompt"], negative_prompt="speech, voice, words, music, melody, ambience, long reverb",
            num_inference_steps=steps, guidance_scale=guidance, audio_end_in_s=entry["duration"],
            num_waveforms_per_prompt=1, generator=generator,
        )
        audio = result.audios[0]
        if hasattr(audio, "detach"):
            audio = audio.detach().float().cpu().numpy()
        prepared = prepare_audio(audio, entry["duration"])
        destination = output_dir / entry["filename"]
        temp_name = None
        try:
            with tempfile.NamedTemporaryFile(prefix=f".{destination.stem}.", suffix=".wav", dir=output_dir, delete=False) as temp:
                temp_name = temp.name
            sf.write(temp_name, prepared, SAMPLE_RATE, format="WAV", subtype="PCM_16")
            os.replace(temp_name, destination)
        finally:
            if temp_name and os.path.exists(temp_name):
                os.unlink(temp_name)
        written.append(destination)
        print(f"[{index}/{total}] Wrote {destination}")
    return written


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("effects", nargs="*", help="effect names to generate (default: all)")
    parser.add_argument("--force", action="store_true", help="replace existing WAV files")
    parser.add_argument("--dry-run", action="store_true", help="validate and list work without loading the model")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    asset_dir = Path(__file__).resolve().parent
    try:
        entries = select_entries(load_manifest(asset_dir / "prompts.json"), args.effects)
    except ValueError as exc:
        raise SystemExit(f"error: {exc}") from exc
    output_dir = Path(os.environ.get("SFX_OUTPUT_DIR", "frames/sfx"))
    if args.dry_run:
        for entry in entries:
            status = "replace" if args.force and (output_dir / entry["filename"]).exists() else "skip" if (output_dir / entry["filename"]).exists() else "generate"
            print(f"{entry['name']}: {status} {output_dir / entry['filename']} ({entry['duration']:.2f}s, seed {entry['seed']})")
        return 0
    generate(
        entries, output_dir, args.force,
        os.environ.get("SFX_MODEL_ID", "stabilityai/stable-audio-open-1.0"),
        os.environ.get("SFX_DEVICE", "auto"),
        int(os.environ.get("SFX_INFERENCE_STEPS", "100")),
        float(os.environ.get("SFX_GUIDANCE_SCALE", "7")),
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
