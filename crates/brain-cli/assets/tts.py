#!/usr/bin/env python3
"""Narration text -> wav.

Primary backend: Qwen3-TTS-12Hz-0.6B-Base (set BRAIN_TTS_MODEL to
override). It is used automatically when its stack is importable — where
the network policy allows Hugging Face:

    pip install torch soundfile transformers
    # weights download on first use: Qwen/Qwen3-TTS-12Hz-0.6B-Base

Fallback backend: espeak-ng (offline, apt-installable everywhere). The
pipeline degrades gracefully: same narration, humbler voice.

Usage: tts.py <narration.txt> <out.wav>
"""
import os
import subprocess
import sys

MODEL = os.environ.get("BRAIN_TTS_MODEL", "Qwen/Qwen3-TTS-12Hz-0.6B-Base")


def qwen3_tts(text: str, out_path: str) -> bool:
    """Qwen3-TTS via transformers; returns False when the stack is absent."""
    try:
        import soundfile as sf  # noqa: F401
        import torch  # noqa: F401
        from transformers import AutoModel, AutoProcessor
    except ImportError:
        return False
    try:
        processor = AutoProcessor.from_pretrained(MODEL, trust_remote_code=True)
        model = AutoModel.from_pretrained(MODEL, trust_remote_code=True)
        inputs = processor(text=text, return_tensors="pt")
        with torch.no_grad():
            audio = model.generate(**inputs)
        wave = audio[0].cpu().numpy().squeeze()
        rate = getattr(getattr(model, "config", None), "sampling_rate", 24000)
        sf.write(out_path, wave, rate)
        return True
    except Exception as e:  # noqa: BLE001 — any failure falls back honestly
        print(f"tts: {MODEL} unavailable ({e.__class__.__name__}); falling back", file=sys.stderr)
        return False


def espeak(text: str, out_path: str) -> bool:
    try:
        subprocess.run(
            ["espeak-ng", "-v", "en-us", "-s", "160", "-p", "40", "-w", out_path, text],
            check=True,
            capture_output=True,
        )
        return True
    except (FileNotFoundError, subprocess.CalledProcessError):
        return False


def main() -> int:
    narration_path, out_path = sys.argv[1], sys.argv[2]
    with open(narration_path) as f:
        text = f.read().strip()
    if not text:
        print("tts: empty narration", file=sys.stderr)
        return 1
    if qwen3_tts(text, out_path):
        print(f"tts: {out_path} via {MODEL}")
        return 0
    if espeak(text, out_path):
        print(f"tts: {out_path} via espeak-ng (fallback; install the {MODEL} stack for the good voice)")
        return 0
    print("tts: no backend available (need the Qwen3-TTS stack or espeak-ng)", file=sys.stderr)
    return 1


if __name__ == "__main__":
    sys.exit(main())
