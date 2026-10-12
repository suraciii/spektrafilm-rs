"""LittleCMS RGB double transform for a disposable ICC-managed display copy."""
from __future__ import annotations

import ctypes
import ctypes.util
from functools import lru_cache
import numpy as np


@lru_cache(maxsize=1)
def library():
    name = ctypes.util.find_library("lcms2")
    if not name:
        raise ValueError("LittleCMS 2 native library is required for ICC conversion")
    lib = ctypes.CDLL(name)
    lib.cmsOpenProfileFromMem.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
    lib.cmsOpenProfileFromMem.restype = ctypes.c_void_p
    lib.cmsCreate_sRGBProfile.argtypes = []
    lib.cmsCreate_sRGBProfile.restype = ctypes.c_void_p
    lib.cmsCreateTransform.argtypes = [ctypes.c_void_p, ctypes.c_uint32, ctypes.c_void_p, ctypes.c_uint32, ctypes.c_uint32, ctypes.c_uint32]
    lib.cmsCreateTransform.restype = ctypes.c_void_p
    lib.cmsDoTransform.argtypes = [ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_uint32]
    lib.cmsDoTransform.restype = None
    lib.cmsDeleteTransform.argtypes = [ctypes.c_void_p]
    lib.cmsDeleteTransform.restype = None
    lib.cmsCloseProfile.argtypes = [ctypes.c_void_p]
    lib.cmsCloseProfile.restype = ctypes.c_int
    lib.cmsGetEncodedCMMversion.argtypes = []
    lib.cmsGetEncodedCMMversion.restype = ctypes.c_uint32
    return lib


def transform_rgb(samples, profile_bytes):
    """ICC relative colorimetric, no BPC, native samples remain untouched."""
    lib = library()
    buffer = ctypes.create_string_buffer(profile_bytes)
    source_profile = lib.cmsOpenProfileFromMem(buffer, len(profile_bytes))
    destination_profile = lib.cmsCreate_sRGBProfile()
    if not source_profile or not destination_profile:
        if source_profile:
            lib.cmsCloseProfile(source_profile)
        if destination_profile:
            lib.cmsCloseProfile(destination_profile)
        raise ValueError("ICC profile could not be opened as an RGB transform")
    transform = None
    try:
        # lcms2.h TYPE_RGB_DBL: FLOAT_SH(1)|COLORSPACE_SH(PT_RGB)|CHANNELS_SH(3).
        rgb_double = (1 << 22) | (4 << 16) | (3 << 3)
        transform = lib.cmsCreateTransform(source_profile, rgb_double, destination_profile, rgb_double, 1, 0)
        if not transform:
            raise ValueError("LittleCMS could not build the RGB ICC transform")
        native = np.ascontiguousarray(samples, dtype=np.float64)
        if native.ndim != 3 or native.shape[2] != 3 or not np.isfinite(native).all():
            raise ValueError("ICC conversion requires finite H x W x 3 samples")
        output = np.empty_like(native)
        flattened = native.reshape(-1, 3)
        transformed = output.reshape(-1, 3)
        # cmsDoTransform uses an unsigned 32-bit pixel count.
        for start in range(0, len(flattened), 1_000_000):
            count = min(1_000_000, len(flattened) - start)
            lib.cmsDoTransform(transform, flattened[start:].ctypes.data, transformed[start:].ctypes.data, count)
        if not np.isfinite(output).all():
            raise ValueError("ICC transform produced nonfinite display values")
        return output
    finally:
        if transform:
            lib.cmsDeleteTransform(transform)
        lib.cmsCloseProfile(source_profile)
        lib.cmsCloseProfile(destination_profile)
