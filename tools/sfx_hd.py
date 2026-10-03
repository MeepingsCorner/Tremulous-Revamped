#!/usr/bin/env python3
"""sfx_hd.py - the enhanced mode's high-band sound pack (spectral band replication).

The original sounds are mostly 22 050 Hz: nothing above 11 kHz. This rebuilds
that octave the way SBR codecs (HE-AAC, mp3PRO) do: resample to 44 100 Hz, then
for every STFT frame copy the 5.5-11 kHz band one octave up into 11-22 kHz,
scaled by that frame's own measured spectral slope (dB per octave over
2.75-11 kHz, clamped), with a soft crossover at 11 kHz and the original peak
level kept. It is signal processing, not a learned model: it adds plausible
"air", not lost detail.

    python3 tools/sfx_hd.py OUT_DIR PK3 [PK3 ...] [--only SUBSTR] [--limit N]

Writes OUT_DIR/<path inside the pk3>.wav (44.1 kHz, 16-bit) for each 22 kHz
.wav, and OUT_DIR/manifest.tsv (path, frames in, frames out, peak in, peak out,
high-band share). 44.1 kHz originals are skipped: they already have the band.
The pack is read by the game from user://sfx_hd (outside the pk3 search path),
so it never changes what servers see.
"""
import io
import os
import sys
import wave
import zipfile

import numpy as np
from scipy.signal import resample_poly, stft, istft

N = 1024
HOP = 256


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


def write_wav(path, x, rate):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    y = np.clip(np.round(x * 32767.0), -32768, 32767).astype("<i2")
    with wave.open(path, "wb") as w:
        w.setnchannels(x.shape[1])
        w.setsampwidth(2)
        w.setframerate(44100)
        w.writeframes(y.tobytes())


def extend(mono):
    """One channel at 44.1 kHz (band-limited to 11 kHz) -> with the top octave."""
    f, _, Z = stft(mono, fs=44100, nperseg=N, noverlap=N - HOP, boundary="even")
    mag, ph = np.abs(Z), np.angle(Z)
    nb = len(f)
    k11 = np.searchsorted(f, 11025.0)
    k55 = np.searchsorted(f, 5512.5)
    k275 = np.searchsorted(f, 2756.0)
    out = np.zeros_like(Z)
    for t in range(Z.shape[1]):
        m = mag[:, t]
        lo = m[k275:k55].mean() + 1e-12
        hi = m[k55:k11].mean() + 1e-12
        slope = 20.0 * np.log10(hi / lo)  # dB per octave (2.75-5.5 -> 5.5-11 kHz)
        slope = float(np.clip(slope, -18.0, -3.0)) - 3.0  # a little under the trend: never brighter
        g = 10.0 ** (slope / 20.0)
        for j in range(k11, nb):
            src = j // 2  # one octave down
            if src < k55:
                continue
            # soft entry just above the crossover
            fade = min(1.0, (f[j] - 11025.0) / 1500.0)
            out[j, t] = m[src] * g * fade * np.exp(1j * (2.0 * ph[src, t]))
    _, y = istft(out, fs=44100, nperseg=N, noverlap=N - HOP, boundary=True)
    return y[: len(mono)] if len(y) >= len(mono) else np.pad(y, (0, len(mono) - len(y)))


def process(x, rate):
    if rate != 22050:
        return None
    up = resample_poly(x, 2, 1, axis=0)
    added = np.stack([extend(up[:, c]) for c in range(up.shape[1])], axis=1)
    y = up + added
    peak_in = float(np.abs(x).max()) or 1.0
    peak_out = float(np.abs(y).max()) or 1.0
    if peak_out > peak_in:
        y *= peak_in / peak_out
    share = float((added ** 2).sum() / max((up ** 2).sum(), 1e-12))
    return y, peak_in, float(np.abs(y).max()), share


def main(argv):
    args = [a for a in argv if not a.startswith("--")]
    only = next((argv[i + 1] for i, a in enumerate(argv) if a == "--only"), None)
    limit = int(next((argv[i + 1] for i, a in enumerate(argv) if a == "--limit"), "0"))
    out_dir, pk3s = args[0], args[1:]
    rows, done = [], 0
    for p in pk3s:
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
            r = process(x, rate)
            if r is None:
                continue
            y, pin, pout, share = r
            write_wav(os.path.join(out_dir, name.lower()), y, 44100)
            rows.append(f"{name.lower()}\t{len(x)}\t{len(y)}\t{pin:.3f}\t{pout:.3f}\t{share:.4f}")
            done += 1
            if limit and done >= limit:
                break
        if limit and done >= limit:
            break
    with open(os.path.join(out_dir, "manifest.tsv"), "w") as f:
        f.write("path\tframes_in\tframes_out\tpeak_in\tpeak_out\thigh_share\n" + "\n".join(rows) + "\n")
    print(f"SUMMARY sfx_hd status=ok files={done} detail={os.path.join(out_dir, 'manifest.tsv')}")


if __name__ == "__main__":
    main(sys.argv[1:])
