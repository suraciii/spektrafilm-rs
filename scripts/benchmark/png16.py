"""16-bit RGB sRGB PNG display copies and lossless crops of those copies."""
import struct
import zlib
from pathlib import Path

import numpy as np


_SIGNATURE = b"\x89PNG\r\n\x1a\n"


def _chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xffffffff)


def save(path, samples):
    """Write uint16 RGB codes; filter zero preserves simple exact cropping."""
    codes = np.asarray(samples)
    if codes.dtype != np.uint16 or codes.ndim != 3 or codes.shape[2] != 3:
        raise ValueError("PNG display samples must be uint16 RGB")
    height, width, _ = codes.shape
    compressor = zlib.compressobj()
    with Path(path).open("wb") as output:
        output.write(_SIGNATURE)
        output.write(_chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 16, 2, 0, 0, 0)))
        output.write(_chunk(b"sRGB", b"\x00"))
        for row in codes:
            data = compressor.compress(b"\x00" + row.astype(">u2").tobytes())
            if data:
                output.write(_chunk(b"IDAT", data))
        output.write(_chunk(b"IDAT", compressor.flush()))
        output.write(_chunk(b"IEND", b""))


def crop(source, destination, rectangle):
    """Crop our retained RGB16 copies without Pillow's RGB8 conversion."""
    compressed = bytearray()
    width = height = None
    with Path(source).open("rb") as image:
        if image.read(8) != _SIGNATURE:
            raise ValueError("Invalid PNG signature")
        while True:
            length, kind = struct.unpack(">I4s", image.read(8))
            data = image.read(length)
            crc, = struct.unpack(">I", image.read(4))
            if zlib.crc32(kind + data) & 0xffffffff != crc:
                raise ValueError("Invalid PNG chunk checksum")
            if kind == b"IHDR":
                width, height, depth, color, compression, filtering, interlace = struct.unpack(">IIBBBBB", data)
                if (depth, color, compression, filtering, interlace) != (16, 2, 0, 0, 0):
                    raise ValueError("Expected generated RGB16 PNG")
            elif kind == b"IDAT":
                compressed.extend(data)
            elif kind == b"IEND":
                break
    rows = np.frombuffer(zlib.decompress(compressed), dtype=np.uint8).reshape(height, width * 6 + 1)
    if np.any(rows[:, 0]):
        raise ValueError("Expected generated PNG filter zero")
    x, y, w, h = rectangle
    codes = rows[y:y+h, 1+x*6:1+(x+w)*6].copy().view(">u2").reshape(h, w, 3).astype(np.uint16)
    save(destination, codes)
