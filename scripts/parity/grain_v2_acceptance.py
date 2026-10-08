#!/usr/bin/env python3
"""Run Grain V2 reference, real GPU dispatch parity and float export acceptance."""
import pathlib
import subprocess

ROOT = pathlib.Path(__file__).resolve().parents[2]
COMMANDS = [
    ["cargo", "test", "-p", "spektrafilm-model", "grain_v2::tests", "--", "--nocapture"],
    ["cargo", "test", "-p", "spektrafilm-model", "--features", "precision-f64", "grain_v2::tests", "--", "--nocapture"],
    ["cargo", "test", "-p", "spektrafilm-gpu", "--test", "grain_v2_shader", "--", "--nocapture"],
    ["cargo", "test", "-p", "spektrafilm-core", "grain_v2_profile_inheritance_and_overrides"],
    ["cargo", "run", "-p", "spektrafilm-core", "--example", "grain_v2_acceptance", "--features", "precision-f64"],
]

if __name__ == "__main__":
    for command in COMMANDS:
        print("+", " ".join(command), flush=True)
        subprocess.run(command, cwd=ROOT, check=True)
