"""Nanonis files and raw TCP calls, for quick analysis from a terminal.

Run inside the analysis shell:

    nix develop .#analysis -c python dev/nanonis/nanonis.py dat FILE.dat
    nix develop .#analysis -c python dev/nanonis/nanonis.py sxm FILE.sxm
    nix develop .#analysis -c python dev/nanonis/nanonis.py psd FILE.dat COLUMN --fs 2000
    nix develop .#analysis -c python dev/nanonis/nanonis.py call Signals.NamesGet --port 6501

Or import it: `from nanonis import read_dat, read_sxm, psd, call`.

Data Logger `.dat` headers carry neither the sample rate nor the bias. Both
are whatever was set on the instrument, so `psd` takes `fs` explicitly and has
no default: the elapsed-time readout over the row count gives a wrong rate.
"""

import argparse
import re
import socket
import struct

import numpy as np


# --- files -----------------------------------------------------------------


def read_dat(path):
    """A `.dat` file as (header dict, column names, rows x columns array)."""
    with open(path, encoding="latin-1") as f:
        lines = f.read().replace("\r", "").split("\n")
    at = lines.index("[DATA]")
    header = {}
    for line in lines[:at]:
        key, _, value = line.partition("\t")
        if key:
            header[key] = value.rstrip("\t")
    columns = lines[at + 1].split("\t")
    rows = [line.split("\t") for line in lines[at + 2 :] if line.strip()]
    return header, columns, np.array(rows, dtype=float)


def read_sxm(path):
    """An `.sxm` scan as a dict: header text, pixels (nx, ny), range (x, y) in
    m, scan direction, and frames keyed by (channel, "fwd"|"bwd").

    Backward frames are mirrored so both directions share the x axis."""
    raw = open(path, "rb").read()
    end = raw.index(b":SCANIT_END:")
    header = raw[:end].decode("latin-1")
    nx, ny = (int(v) for v in re.search(r":SCAN_PIXELS:\s*(\d+)\s+(\d+)", header).groups())
    width, height = (float(v) for v in re.search(r":SCAN_RANGE:\s*(\S+)\s+(\S+)", header).groups())

    channels = []
    for line in header[header.index(":DATA_INFO:") :].split("\n")[2:]:
        fields = line.strip().split("\t")
        if len(fields) < 4:
            break
        channels.append((fields[1], fields[3]))

    data = np.frombuffer(raw[raw.index(b"\x1a\x04", end) + 2 :], dtype=">f4")
    frames, k = {}, 0
    for name, direction in channels:
        for d in ["fwd", "bwd"] if direction == "both" else [direction]:
            frame = data[k * nx * ny : (k + 1) * nx * ny].reshape(ny, nx).astype(float)
            frames[(name, d)] = frame[:, ::-1] if d == "bwd" else frame
            k += 1

    return {
        "header": header,
        "pixels": (nx, ny),
        "range_m": (width, height),
        "direction": "down" if ":SCAN_DIR: down" in header else "up",
        "frames": frames,
    }


def psd(x, *, fs, nperseg=1024):
    """Welch PSD of a linearly detrended series. `fs` in Hz, required."""
    from scipy.signal import welch

    t = np.arange(len(x))
    r = x - np.polyval(np.polyfit(t, x, 1), t)
    return welch(r, fs=fs, nperseg=min(nperseg, len(r)), window="hann")


# --- raw TCP ---------------------------------------------------------------
#
# Header (40 bytes, big-endian): command name padded to 32 bytes, body size
# (int32), send-response flag (uint16), 2 zero bytes. A reply carries the same
# header, then the body, which ends with an error block (status uint32,
# description size int32, description). Nanonis takes one client per port, so
# a port the workbench holds refuses a second connection.


def call(name, body=b"", *, port=6501, host="127.0.0.1"):
    """Send one command, return the reply body (error block included)."""
    with socket.create_connection((host, port), timeout=5) as s:
        s.sendall(name.encode().ljust(32, b"\0") + struct.pack(">iH", len(body), 1) + b"\0\0")
        head = _recv_exact(s, 40)
        return _recv_exact(s, struct.unpack(">i", head[32:36])[0])


def _recv_exact(sock, n):
    data = b""
    while len(data) < n:
        chunk = sock.recv(n - len(data))
        if not chunk:
            raise EOFError("connection closed")
        data += chunk
    return data


class Reader:
    """Cursor over a reply body, reading Nanonis' big-endian types."""

    def __init__(self, data):
        self.data, self.at = data, 0

    def _take(self, fmt):
        (v,) = struct.unpack_from(fmt, self.data, self.at)
        self.at += struct.calcsize(fmt)
        return v

    def i32(self):
        return self._take(">i")

    def u32(self):
        return self._take(">I")

    def f32(self):
        return self._take(">f")

    def string(self, n):
        s = self.data[self.at : self.at + n].decode(errors="replace")
        self.at += n
        return s

    def strings(self):
        """String array: total bytes, count, then (length, chars) each."""
        self.i32()
        return [self.string(self.i32()) for _ in range(self.i32())]

    def error(self):
        status = self.u32()
        return status, self.string(self.i32())


# --- command line ----------------------------------------------------------


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = p.add_subparsers(dest="cmd", required=True)
    sub.add_parser("dat").add_argument("path")
    sub.add_parser("sxm").add_argument("path")
    q = sub.add_parser("psd")
    q.add_argument("path")
    q.add_argument("column", help="column name, or its index")
    q.add_argument("--fs", type=float, required=True, help="sample rate in Hz, as set on the logger")
    c = sub.add_parser("call")
    c.add_argument("name")
    c.add_argument("--port", type=int, default=6501)
    a = p.parse_args()

    if a.cmd == "dat":
        header, columns, data = read_dat(a.path)
        for k, v in header.items():
            print(f"{k}: {v}")
        print(f"\n{data.shape[0]} rows")
        for i, name in enumerate(columns):
            col = data[:, i]
            print(f"  [{i}] {name}: mean {col.mean():.6g}  std {col.std():.6g}")
    elif a.cmd == "sxm":
        s = read_sxm(a.path)
        print(f"pixels {s['pixels']}, range {s['range_m']} m, scanned {s['direction']}")
        for (name, d), frame in s["frames"].items():
            print(f"  {name} {d}: {np.nanmin(frame):.6g} .. {np.nanmax(frame):.6g}, {np.isnan(frame).sum()} NaN")
    elif a.cmd == "psd":
        _, columns, data = read_dat(a.path)
        i = int(a.column) if a.column.isdigit() else columns.index(a.column)
        f, S = psd(data[:, i], fs=a.fs)
        print(f"{columns[i]}: {len(data)} rows at {a.fs:g} Hz, {len(data) / a.fs:.2f} s")
        edges = np.geomspace(f[1], f[-1], 12)
        for lo, hi in zip(edges[:-1], edges[1:]):
            m = (f >= lo) & (f < hi)
            if m.any():
                print(f"  {lo:9.2f} - {hi:9.2f} Hz: {np.sqrt(S[m].mean()):.4g} /rtHz")
    elif a.cmd == "call":
        body = call(a.name, port=a.port)
        print(f"{len(body)} bytes: {body.hex()}")


if __name__ == "__main__":
    main()
