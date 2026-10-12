"""Declared image loading and colour conversion for benchmark masters."""
from dataclasses import dataclass, field
from pathlib import Path
import json
import hashlib

import numpy as np
from PIL import Image, ImageCms

try:
    import tifffile
except ImportError:  # pragma: no cover
    tifffile = None


class ImageDataError(ValueError):
    def __init__(self, code, message):
        super().__init__(message)
        self.code = code


@dataclass
class ImageData:
    samples: np.ndarray
    color_space: str | None = None
    transfer: str | None = None
    path: str | None = None
    metadata: dict = field(default_factory=dict)

    @property
    def dimensions(self):
        return [int(self.samples.shape[1]), int(self.samples.shape[0])]


def srgb_decode(encoded):
    x = np.asarray(encoded, dtype=np.float64)
    return np.where(x <= 0.04045, x / 12.92, ((np.maximum(x, 0.04045) + .055) / 1.055) ** 2.4)


def srgb_encode(linear):
    x = np.asarray(linear, dtype=np.float64)
    return np.where(x <= .0031308, x * 12.92, 1.055 * np.maximum(x, .0031308) ** (1 / 2.4) - .055)


def _norm_space(value):
    if not isinstance(value, str):
        return value
    key = value.lower().replace("-", "").replace("_", "").replace(" ", "")
    return {"srgb": "sRGB", "adobergb1998": "Adobe RGB (1998)", "adobergb": "Adobe RGB (1998)",
            "prophotorgb": "ProPhoto RGB", "prophoto": "ProPhoto RGB", "displayp3": "Display P3", "p3": "Display P3",
            "dcip3": "DCI-P3", "iturbt2020": "ITU-R BT.2020", "rec2020": "ITU-R BT.2020", "bt2020": "ITU-R BT.2020",
            "acescg": "ACEScg", "aces2065": "ACES2065-1", "aces20651": "ACES2065-1"}.get(key, value)


def _norm_transfer(value):
    if isinstance(value, bool):
        return "encoded" if value else "linear"
    if not isinstance(value, str):
        return value
    key = value.lower().replace("-", "").replace("_", "").replace(" ", "")
    return {"srgb": "srgb", "encoded": "encoded", "linear": "linear", "identity": "linear",
            "true": "encoded", "false": "linear", "cctfencoded": "encoded"}.get(key, value)


def _load_tiff(path):
    if tifffile is None:
        raise ImageDataError("missing_dependency", "tifffile is required for TIFF masters")
    try:
        arr = tifffile.imread(path)
    except Exception as exc:
        raise ImageDataError("image_load_failed", str(exc)) from exc
    return np.asarray(arr)


def _icc_display(image):
    """Transform a disposable display copy; never replace native samples."""
    import io
    icc = image.metadata.get("icc") or image.metadata.get("color", {}).get("icc")
    if not isinstance(icc, dict) or not icc.get("sha256"):
        raise ImageDataError("missing_color_declaration", "ICC transform requires a hashed profile declaration")
    if icc.get("path"):
        profile = Path(icc["path"]).read_bytes()
    elif image.path:
        with Image.open(image.path) as original:
            profile = original.info.get("icc_profile")
        if not profile:
            raise ImageDataError("invalid_color_profile", "Declared embedded ICC profile is absent")
    else:
        raise ImageDataError("invalid_color_profile", "ICC transform has no profile source")
    if hashlib.sha256(profile).hexdigest() != icc["sha256"]:
        raise ImageDataError("invalid_color_profile", "ICC profile hash differs")
    from icc import transform_rgb
    try:
        return transform_rgb(image.samples, profile)
    except ValueError as error:
        raise ImageDataError("unsupported_color_profile", str(error)) from error


def load_image(source):
    if isinstance(source, ImageData):
        arr = np.asarray(source.samples)
        metadata = dict(source.metadata or {})
        if arr.ndim != 3 or arr.shape[2] != 3 or min(arr.shape[:2]) <= 0 or arr.dtype.kind != "f":
            raise ImageDataError("invalid_image_layout", "Expected nonempty floating H x W x 3 RGB samples")
        return ImageData(arr, _norm_space(source.color_space), _norm_transfer(source.transfer), source.path, metadata)
    declaration = dict(source) if isinstance(source, dict) else {"path": str(source)}
    if "path" not in declaration:
        raise ImageDataError("missing_image", "Image declaration requires path")
    path = Path(declaration["path"])
    try:
        metadata = dict(declaration)
        sidecar = Path(str(path) + ".json")
        if sidecar.exists():
            side = json.loads(sidecar.read_text(encoding="utf-8"))
            if not isinstance(side, dict):
                raise ImageDataError("invalid_image_metadata", "Image sidecar must be an object")
            side.update(metadata)
            metadata = side
        layout = dict(metadata.get("layout", {}))
        layout.update({k: metadata[k] for k in ("width", "height", "channels", "dtype", "channel_order", "endianness") if k in metadata})
        suffix = path.suffix.lower()
        if suffix == ".npy":
            arr = np.load(path, allow_pickle=False)
        elif suffix in (".tif", ".tiff"):
            arr = _load_tiff(path)
        elif suffix in (".raw", ".bin", ".f32", ".f64"):
            required = ("width", "height", "channels", "dtype", "channel_order", "endianness")
            if any(k not in layout for k in required) or layout["channels"] != 3 or layout["channel_order"] != "RGB":
                raise ImageDataError("missing_raw_metadata", "Raw images require declared RGB dimensions, dtype, order, and endianness")
            dtype = np.dtype({"f32": "float32", "f64": "float64"}.get(layout["dtype"], layout["dtype"]))
            if dtype.kind != "f" or dtype.itemsize not in (4, 8):
                raise ImageDataError("unsupported_image_layout", "Raw masters require float32 or float64")
            if layout["endianness"] not in ("little", "big"):
                raise ImageDataError("invalid_image_metadata", "Raw endianness must be little or big")
            dtype = dtype.newbyteorder("<" if layout["endianness"] == "little" else ">")
            width, height = int(layout["width"]), int(layout["height"])
            expected = width * height * 3 * dtype.itemsize
            if path.stat().st_size != expected:
                raise ImageDataError("invalid_image_layout", "Raw file length differs from declaration")
            arr = np.fromfile(path, dtype=dtype).reshape(height, width, 3)
        elif suffix in (".png", ".jpg", ".jpeg", ".webp"):
            with Image.open(path) as im:
                if im.mode != "RGB":
                    raise ImageDataError("unsupported_image_layout", "Raster must have three native RGB channels")
                arr = np.asarray(im)
        else:
            raise ImageDataError("unsupported_image_encoding", "Supported images are NPY, declared raw, TIFF, PNG, JPEG, and WebP")
        arr = np.asarray(arr)
        if arr.ndim != 3 or arr.shape[2] != 3 or min(arr.shape[:2]) <= 0:
            raise ImageDataError("invalid_image_layout", "Expected nonempty H x W x 3 RGB samples")
        metadata["container_dtype"] = str(arr.dtype)
        if arr.dtype.kind in "ui":
            arr = arr.astype(np.float64) / np.iinfo(arr.dtype).max
        elif arr.dtype.kind != "f":
            raise ImageDataError("unsupported_image_layout", "Samples must be unsigned raster or floating values")
        for key, actual in (("width", arr.shape[1]), ("height", arr.shape[0]), ("channels", 3)):
            if key in layout and int(layout[key]) != actual:
                raise ImageDataError("invalid_image_layout", f"Decoded image disagrees with declared {key}")
        color = metadata.get("color", {}) if isinstance(metadata.get("color", {}), dict) else {}
        space = _norm_space(metadata.get("color_space", color.get("color_space")))
        transfer_value = metadata.get("transfer", metadata.get("cctf_decoding", color.get("transfer", color.get("cctf_decoding"))))
        transfer = _norm_transfer(transfer_value)
        return ImageData(arr, space, transfer, str(path), metadata)
    except ImageDataError:
        raise
    except (OSError, ValueError, TypeError, KeyError) as exc:
        raise ImageDataError("image_load_failed", str(exc)) from exc


def _colour_to_srgb(samples, space, transfer):
    try:
        import colour
        cs = colour.RGB_COLOURSPACES.get(space)
        if cs is None:
            raise ImageDataError("unsupported_color_transform", f"Unknown colour space: {space}")
        linear = np.asarray(samples, dtype=np.float64)
        if transfer in ("srgb", "encoded"):
            linear = cs.cctf_decoding(linear)
        elif transfer != "linear":
            raise ImageDataError("unsupported_color_transform", f"Unknown transfer: {transfer}")
        return colour.RGB_to_RGB(linear, cs, colour.RGB_COLOURSPACES["sRGB"],
                                 chromatic_adaptation_transform="CAT02", apply_cctf_encoding=False)
    except ImageDataError:
        raise
    except Exception as exc:
        raise ImageDataError("unsupported_color_transform", str(exc)) from exc


def display_rgb(image):
    if image.metadata.get("icc") or image.metadata.get("color", {}).get("icc"):
        return _icc_display(image)
    space, transfer = _norm_space(image.color_space), _norm_transfer(image.transfer)
    if not space or not transfer:
        raise ImageDataError("missing_color_declaration", "Color space and transfer must be explicit")
    if space == "sRGB":
        if transfer not in ("srgb", "encoded", "linear"):
            raise ImageDataError("unsupported_color_transform", f"Unknown sRGB transfer: {transfer}")
        linear = srgb_decode(image.samples) if transfer in ("srgb", "encoded") else np.asarray(image.samples, dtype=np.float64)
    else:
        linear = _colour_to_srgb(image.samples, space, transfer)
    return srgb_encode(linear)


def rgb_to_lab(encoded_srgb):
    import colour
    return colour.XYZ_to_Lab(
        colour.sRGB_to_XYZ(np.asarray(encoded_srgb, dtype=np.float64)),
        illuminant=colour.RGB_COLOURSPACES["sRGB"].whitepoint)


def linear_rgb(image):
    """Unclipped linear sRGB under the same declared CAT02 conversion."""
    return srgb_decode(display_rgb(image))
