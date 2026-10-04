#!/usr/bin/env python3
"""sfx_audiosr.py - the enhanced mode's HD sound pack, rebuilt with AudioSR.

AudioSR (Liu et al. 2023, a latent diffusion model for audio super-resolution)
regenerates the octave above 11 kHz that the original 22 050 Hz sounds never
had, from what the rest of each sound contains. It replaces the spectral band
replication of tools/sfx_hd.py (which copies the 5.5-11 kHz band up an
octave) where it succeeds; sfx_hd.py stays the fallback.

Only the new band is taken from the model: below the crossover the sound is
the original, resampled, so the sound the game has always had is unchanged and
the model cannot shift its timing, pitch or level. Above it the model's band
is joined with a 1.5 kHz crossfade and kept at least 6 dB under the original's
own 5.5-11 kHz band one octave down, frame by frame (the "never brighter" rule
of sfx_hd.py; the trial showed the model's band often reaching that level).

    python tools/sfx_audiosr.py OUT_DIR PK3 [PK3 ...] [--only SUBSTR] [--limit N]
        [--steps 50] [--device cuda] [--fallback-dir SBR_DIR] [--resume]
        [--redo-file LIST]

--resume keeps the sounds OUT_DIR already has (and their manifest rows), so an
interrupted build continues where it stopped; the sounds named in LIST (one
pk3 path per line, e.g. those that fell back to SBR) are built again.

Writes OUT_DIR/<path inside the pk3>.wav (48 kHz, 16-bit) and
OUT_DIR/manifest.tsv (path, method, frames, peak in / out, high-band share,
seconds). Needs: torch (CUDA), audiosr 0.0.7, numpy, scipy, soundfile.
The pack is read by the game from user://sfx_hd (outside the pk3 search path),
so servers never see it.
"""
import io
import os
import sys
import tempfile
import time
import wave
import zipfile

import numpy as np
from scipy.signal import istft, resample_poly, stft

OUT_RATE = 48000
N = 2048
HOP = 512
CROSS_LO, CROSS_HI = 10000.0, 11500.0


def read_wav(data):
    w = wave.open(io.BytesIO(data))
    ch, sw, rate, n = w.getnchannels(), w.getsampwidth(), w.getframerate(), w.getnframes()
    raw = w.readframes(n)
    if sw == 2:
        x = np.frombuffer(raw, dtype="<i2").astype(np.float64) / 32768.0
    elif sw == 1:
        x = (np.frombuffer(raw, dtype=np.uint8).astype(np.float64) - 128.0) / 128.0
    else:
        raise ValueError(f"{sw * 8}-bit")
    return x.reshape(-1, ch), rate


def write_wav(path, x):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    y = np.clip(np.round(x * 32767.0), -32768, 32767).astype("<i2")
    with wave.open(path, "wb") as w:
        w.setnchannels(x.shape[1])
        w.setsampwidth(2)
        w.setframerate(OUT_RATE)
        w.writeframes(y.tobytes())


def to48(x, rate):
    """Resample (frames, ch) to 48 kHz."""
    from math import gcd

    g = gcd(OUT_RATE, rate)
    return resample_poly(x, OUT_RATE // g, rate // g, axis=0)


def join_bands(orig48, sr48):
    """The original below the crossover, the model's band above it, capped
    per frame at the original's 5.5-11 kHz level an octave down."""
    n = len(orig48)
    sr48 = sr48[:n] if len(sr48) >= n else np.pad(sr48, (0, n - len(sr48)))
    f, _, A = stft(orig48, fs=OUT_RATE, nperseg=N, noverlap=N - HOP, boundary="even")
    _, _, B = stft(sr48, fs=OUT_RATE, nperseg=N, noverlap=N - HOP, boundary="even")
    w = np.clip((f - CROSS_LO) / (CROSS_HI - CROSS_LO), 0.0, 1.0)[:, None]
    k55, k11 = np.searchsorted(f, 5512.5), np.searchsorted(f, 11025.0)
    ref = np.abs(A[k55:k11]).mean(axis=0) + 1e-12  # per frame
    hi = np.abs(B[np.searchsorted(f, 11025.0):]).mean(axis=0) + 1e-12
    cap = np.minimum(1.0, 0.5 * ref / hi)[None, :]  # at most 6 dB under the octave below (a natural roll-off)
    Z = A * (1.0 - w) + B * w * cap
    _, y = istft(Z, fs=OUT_RATE, nperseg=N, noverlap=N - HOP, boundary=True)
    y = y[:n] if len(y) >= n else np.pad(y, (0, n - len(y)))
    return y


class Model:
    def __init__(self, device, steps):
        from audiosr import build_model, super_resolution  # noqa: F401

        self.sr = super_resolution
        self.m = build_model(model_name="basic", device=device)
        self.steps = steps

    def run(self, mono48):
        """AudioSR on one channel (48 kHz in, 48 kHz out). The input is
        zero-padded to whole 5.12 s windows: other lengths can make the
        model's latent one frame short ("Expected size 64 but got size 63");
        a second try pads one more window."""
        n = len(mono48)
        win = int(5.12 * OUT_RATE)
        for extra in (0, 1):
            m = (-(-n // win) + extra) * win
            x = np.pad(mono48, (0, m - n))
            try:
                with tempfile.TemporaryDirectory() as d:
                    p = os.path.join(d, "in.wav")
                    write_wav(p, x[:, None])
                    y = self.sr(self.m, p, seed=42, guidance_scale=3.5, ddim_steps=self.steps, latent_t_per_second=12.8)
                return np.asarray(y, dtype=np.float64).reshape(-1)[:n]
            except RuntimeError:
                if extra:
                    raise
        raise RuntimeError("unreachable")


def be_polite():
    """Idle CPU priority (Windows), so the game always comes first."""
    if os.name == "nt":
        import ctypes

        ctypes.windll.kernel32.SetPriorityClass(ctypes.windll.kernel32.GetCurrentProcess(), 0x40)  # IDLE_PRIORITY_CLASS


def wait_while_playing():
    """The model takes most of the GPU: while Godot (the game, a test, the
    editor) runs, wait between sounds."""
    if os.name != "nt":
        return
    import subprocess

    waited = 0
    while True:
        out = subprocess.run(["tasklist", "/FO", "CSV", "/NH"], capture_output=True, text=True).stdout.lower()
        if '"godot' not in out:
            break
        if waited == 0:
            print("paused: Godot is running", flush=True)
        time.sleep(30)
        waited += 30
    if waited:
        print(f"resumed after {waited} s", flush=True)


def main(argv):
    args = [a for a in argv if not a.startswith("--")]
    opt = lambda k, d: next((argv[i + 1] for i, a in enumerate(argv) if a == k), d)  # noqa: E731
    only, limit = opt("--only", None), int(opt("--limit", "0"))
    steps, device, fallback = int(opt("--steps", "50")), opt("--device", "cuda"), opt("--fallback-dir", None)
    redo_file = opt("--redo-file", None)
    redo = set(l.strip().lower() for l in open(redo_file).read().splitlines() if l.strip()) if redo_file and os.path.exists(redo_file) else set()
    for v in (only, opt("--limit", None), opt("--steps", None), opt("--device", None), fallback, redo_file):
        if v in args:
            args.remove(v)
    out_dir = args[0]
    pk3s = sorted(args[1:], key=lambda q: os.path.basename(q).lower())  # the game's order (assets.rs)
    resume = "--resume" in argv
    old = {}
    mf = os.path.join(out_dir, "manifest.tsv")
    if resume and os.path.exists(mf):
        for line in open(mf).read().splitlines()[1:]:
            if line.strip():
                old[line.split("\t")[0]] = line
    be_polite()
    model = Model(device, steps)
    rows, done, t_all = [], 0, time.time()
    seen = set()
    # the Q3 search order: a later pk3 overrides an earlier one, so the last
    # pk3 that has a sound is the one the game plays: take each sound from it
    for p in reversed(pk3s):
        z = zipfile.ZipFile(p)
        for info in z.infolist():
            name = info.filename
            if not name.lower().endswith(".wav") or (only and only not in name):
                continue
            try:
                x, rate = read_wav(z.read(info))
            except Exception as e:  # noqa: BLE001 - report and go on
                print(f"skip {name}: {e}")
                continue
            if rate >= 44100:
                continue  # already has the band
            key = name.lower()
            if key in seen:
                continue  # a later pk3 overrides this one
            seen.add(key)
            if resume and key not in redo and os.path.exists(os.path.join(out_dir, key)):
                rows.append(old.get(key, f"{key}\tkept\t0\t0\t0\t0\t0"))
                done += 1
                continue
            wait_while_playing()
            t0 = time.time()
            x48 = to48(x, rate)
            method = "audiosr"
            try:
                y = np.stack([join_bands(x48[:, c], model.run(x48[:, c])) for c in range(x48.shape[1])], axis=1)
                if not np.isfinite(y).all():
                    raise ValueError("non-finite output")
            except Exception as e:  # noqa: BLE001 - fall back to the SBR pack
                print(f"audiosr failed on {name}: {e}")
                fb = fallback and os.path.join(fallback, name.lower())
                if fb and os.path.exists(fb):
                    fx, frate = read_wav(open(fb, "rb").read())
                    y, method = to48(fx, frate), "sbr"
                else:
                    y, method = x48, "orig"
            peak_in = float(np.abs(x).max()) or 1.0
            peak = float(np.abs(y).max()) or 1.0
            if peak > peak_in:
                y *= peak_in / peak
            share = float(((y - x48) ** 2).sum() / max((x48 ** 2).sum(), 1e-12))
            write_wav(os.path.join(out_dir, name.lower()), y)
            dt = time.time() - t0
            rows.append(f"{name.lower()}\t{method}\t{len(y)}\t{peak_in:.3f}\t{float(np.abs(y).max()):.3f}\t{share:.4f}\t{dt:.1f}")
            done += 1
            print(f"[{done}] {name} {method} {dt:.1f} s", flush=True)
            with open(mf, "w") as f:  # progressive: --resume can trust it
                f.write("path\tmethod\tframes\tpeak_in\tpeak_out\thigh_share\tseconds\n" + "\n".join(rows) + "\n")
            if limit and done >= limit:
                break
        if limit and done >= limit:
            break
    with open(os.path.join(out_dir, "manifest.tsv"), "w") as f:
        f.write("path\tmethod\tframes\tpeak_in\tpeak_out\thigh_share\tseconds\n" + "\n".join(rows) + "\n")
    n_sr = sum(1 for r in rows if "\taudiosr\t" in r)
    print(f"SUMMARY sfx_audiosr status=ok files={done} audiosr={n_sr} seconds={time.time() - t_all:.0f} detail={os.path.join(out_dir, 'manifest.tsv')}")


if __name__ == "__main__":
    main(sys.argv[1:])
