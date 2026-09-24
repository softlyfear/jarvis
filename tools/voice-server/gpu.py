"""GPU detection for the Jarvis voice server: decides how Whisper and the voice clone run.

Profiles:
  cuda    NVIDIA                    faster-whisper on CUDA        + XTTS on CUDA
  rocm    AMD with a ROCm PyTorch    whisper.cpp on Vulkan         + XTTS on ROCm (falls back to CPU)
          build for Windows
  vulkan  other GPUs (Intel, AMD     whisper.cpp on Vulkan         + XTTS on the CPU
          without ROCm PyTorch)
  cpu     no usable GPU             whisper.cpp on the CPU        + XTTS on the CPU

Standard library only: install.ps1 runs it before any package is installed.

    python gpu.py            print the detected profile as JSON
    python gpu.py --write    also save it to gpu-profile.json next to this file

Overrides: JARVIS_GPU_PROFILE=cuda|rocm|vulkan|cpu, JARVIS_GFX=gfx1100.
"""

import ctypes
import json
import os
import re
import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
PROFILE_FILE = HERE / "gpu-profile.json"
PROFILES = ("cuda", "rocm", "vulkan", "cpu")

VENDORS = {"10de": "nvidia", "1002": "amd", "8086": "intel"}

# gfx targets with AMD PyTorch (ROCm) wheels for Windows: TheRock families
# gfx101X-dgpu, gfx103X-all, gfx110X-all, gfx1150, gfx1151, gfx120X-all
ROCM_WINDOWS_TARGETS = frozenset(
    ["gfx1010", "gfx1011", "gfx1012"]
    + [f"gfx103{i}" for i in range(7)]
    + ["gfx1100", "gfx1101", "gfx1102", "gfx1103", "gfx1150", "gfx1151", "gfx1200", "gfx1201"]
)

# integrated Radeon GPUs: a discrete card, when present, is preferred
INTEGRATED_TARGETS = frozenset(["gfx1033", "gfx1035", "gfx1036", "gfx1103", "gfx1150", "gfx1151", "gfx1152", "gfx1153"])

# product name -> gfx target, used when OpenCL cannot tell which card is which.
# Mobile variants first: "RX 6800M" is a different chip from "RX 6800".
_NAME_TO_GFX = [
    (r"rx\s*6[5-9]\d0s\b", "gfx1032"),
    (r"rx\s*6(?:8[05]0|700)m\b", "gfx1031"),
    (r"rx\s*66\d0m\b", "gfx1032"),
    (r"rx\s*7[67]\d0s\b", "gfx1102"),
    (r"rx\s*9070|ai\s*pro\s*r9[67]00", "gfx1201"),
    (r"rx\s*9060", "gfx1200"),
    (r"rx\s*79\d0|pro\s*w7[89]00", "gfx1100"),
    (r"rx\s*7[78]\d0|pro\s*w7700", "gfx1101"),
    (r"rx\s*76\d0|pro\s*w7[56]00", "gfx1102"),
    (r"rx\s*6[89]\d0|pro\s*w6800", "gfx1030"),
    (r"rx\s*67\d0", "gfx1031"),
    (r"rx\s*66\d0|pro\s*w6600", "gfx1032"),
    (r"rx\s*6[45]\d0", "gfx1034"),
    (r"rx\s*5[67]\d0", "gfx1010"),
    (r"rx\s*5[35]\d0|pro\s*w5500", "gfx1012"),
    (r"80[456]0s\b", "gfx1151"),
    (r"8[89]0m\b", "gfx1150"),
    (r"8[46]0m\b", "gfx1152"),
    (r"820m\b", "gfx1153"),
    (r"7[468]0m\b", "gfx1103"),
    (r"6[68]0m\b", "gfx1035"),
]

_IGNORED_ADAPTERS = ("microsoft basic", "remote display", "virtual", "parsec", "citrix", "vmware", "virtualbox")


def gfx_from_name(name):
    lower = name.lower()
    for pattern, gfx in _NAME_TO_GFX:
        if re.search(pattern, lower):
            return gfx
    return None


def is_discrete_amd(name, gfx=None):
    lower = name.lower()
    if re.search(r"\brx\s*\d{3,4}|\bpro\s*w\d{4}|ai\s*pro\s*r\d{4}|radeon\s*vii", lower):
        return True
    return gfx is not None and gfx not in INTEGRATED_TARGETS and "graphics" not in lower


# ---------------------------------------------------------------- probing (Windows)


def list_adapters():
    """Display adapters from WMI: [{"name", "vendor"}]. Empty outside Windows."""
    if os.name != "nt":
        return []
    script = (
        "Get-CimInstance Win32_VideoController | Select-Object Name, PNPDeviceID | "
        "ConvertTo-Json -Compress"
    )
    try:
        out = subprocess.run(
            ["powershell", "-NoProfile", "-NonInteractive", "-Command", script],
            capture_output=True,
            timeout=30,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        ).stdout
        data = json.loads(out.decode("utf-8", "replace") or "[]")
    except Exception:
        return []
    return parse_adapters(data)


def parse_adapters(data):
    if isinstance(data, dict):
        data = [data]
    adapters = []
    for item in data or []:
        name = str(item.get("Name") or "").strip()
        pnp = str(item.get("PNPDeviceID") or "")
        m = re.search(r"VEN_([0-9A-Fa-f]{4})", pnp)
        vendor = VENDORS.get(m.group(1).lower(), "other") if m else "other"
        if not name or any(s in name.lower() for s in _IGNORED_ADAPTERS):
            continue
        if vendor == "other":
            lower = name.lower()
            vendor = "nvidia" if "nvidia" in lower else "amd" if ("radeon" in lower or "amd" in lower) else "intel" if "intel" in lower else "other"
        adapters.append({"name": name, "vendor": vendor})
    return adapters


def opencl_gfx_targets():
    """gfx targets of AMD GPUs via OpenCL: the AMD driver reports them as device names."""
    names = []
    try:
        lib = ctypes.WinDLL("OpenCL.dll") if os.name == "nt" else ctypes.CDLL("libOpenCL.so.1")
    except OSError:
        return names
    CL_DEVICE_TYPE_GPU = 1 << 2
    CL_DEVICE_NAME = 0x102B
    CL_PLATFORM_VENDOR = 0x0903
    count = ctypes.c_uint32()
    if lib.clGetPlatformIDs(0, None, ctypes.byref(count)) != 0 or count.value == 0:
        return names
    platforms = (ctypes.c_void_p * count.value)()
    if lib.clGetPlatformIDs(count.value, platforms, None) != 0:
        return names
    buf = ctypes.create_string_buffer(256)
    for platform in platforms:
        lib.clGetPlatformInfo(ctypes.c_void_p(platform), CL_PLATFORM_VENDOR, 256, buf, None)
        if b"advanced micro devices" not in buf.value.lower() and b"amd" not in buf.value.lower():
            continue
        n = ctypes.c_uint32()
        if lib.clGetDeviceIDs(ctypes.c_void_p(platform), CL_DEVICE_TYPE_GPU, 0, None, ctypes.byref(n)) != 0 or n.value == 0:
            continue
        devices = (ctypes.c_void_p * n.value)()
        lib.clGetDeviceIDs(ctypes.c_void_p(platform), CL_DEVICE_TYPE_GPU, n.value, devices, None)
        for device in devices:
            if lib.clGetDeviceInfo(ctypes.c_void_p(device), CL_DEVICE_NAME, 256, buf, None) == 0:
                m = re.match(rb"(gfx[0-9a-f]+)", buf.value.strip().lower())
                if m:
                    names.append(m.group(1).decode())
    return names


# ---------------------------------------------------------------- decision


def choose(adapters, gfx_targets=(), forced_profile=None, forced_gfx=None):
    """Pick the profile. Pure function: tested without hardware."""
    nvidia = [a for a in adapters if a["vendor"] == "nvidia"]
    amd = [a for a in adapters if a["vendor"] == "amd"]
    others = [a for a in adapters if a["vendor"] not in ("nvidia", "amd")]

    result = {"profile": "cpu", "gpu": None, "vendor": None, "gfx": None}
    if nvidia:
        result.update(profile="cuda", gpu=nvidia[0]["name"], vendor="nvidia")
    elif amd:
        known = [(a, forced_gfx or gfx_from_name(a["name"])) for a in amd]
        # prefer the discrete card, then anything with a known target
        known.sort(key=lambda x: (not is_discrete_amd(x[0]["name"], x[1]), x[1] is None))
        adapter, gfx = known[0]
        if gfx is None:
            # OpenCL names the chip; with an iGPU next to the card, take the non-integrated target
            targets = [t for t in gfx_targets if t not in INTEGRATED_TARGETS] or list(gfx_targets)
            gfx = targets[0] if targets else None
        result.update(gpu=adapter["name"], vendor="amd", gfx=gfx)
        result["profile"] = "rocm" if gfx in ROCM_WINDOWS_TARGETS else "vulkan"
    elif others:
        result.update(profile="vulkan", gpu=others[0]["name"], vendor=others[0]["vendor"])

    if forced_profile in PROFILES:
        result["profile"] = forced_profile
    return describe(result)


def describe(result):
    profile = result["profile"]
    result["stt"] = "faster-whisper" if profile == "cuda" else "whispercpp"
    result["tts_device"] = "gpu" if profile in ("cuda", "rocm") else "cpu"
    return result


def detect():
    adapters = list_adapters()
    gfx = []
    if any(a["vendor"] == "amd" for a in adapters):
        try:
            gfx = opencl_gfx_targets()
        except Exception:
            gfx = []
    return choose(
        adapters,
        gfx,
        forced_profile=os.environ.get("JARVIS_GPU_PROFILE", "").strip().lower() or None,
        forced_gfx=os.environ.get("JARVIS_GFX", "").strip().lower() or None,
    )


def load_profile(path=PROFILE_FILE, fallback=None):
    """Profile saved by the installer. Without one (installed before profiles, or set up by
    hand) `fallback()` decides, e.g. `detect`; with no fallback, NVIDIA/CUDA as before."""
    try:
        data = json.loads(Path(path).read_text(encoding="utf-8"))
        if data.get("profile") in PROFILES:
            return describe(data)
    except (OSError, ValueError):
        pass
    if fallback is not None:
        return fallback()
    return describe({"profile": "cuda", "gpu": None, "vendor": None, "gfx": None})


# ---------------------------------------------------------------- Vulkan device choice


def vulkan_devices():
    """[(index, name, type)] from the Vulkan loader; type: 1 integrated, 2 discrete, 4 CPU."""
    try:
        lib = ctypes.WinDLL("vulkan-1.dll") if os.name == "nt" else ctypes.CDLL("libvulkan.so.1")
    except OSError:
        return []

    class AppInfo(ctypes.Structure):
        _fields_ = [
            ("sType", ctypes.c_int), ("pNext", ctypes.c_void_p), ("pApplicationName", ctypes.c_char_p),
            ("applicationVersion", ctypes.c_uint32), ("pEngineName", ctypes.c_char_p),
            ("engineVersion", ctypes.c_uint32), ("apiVersion", ctypes.c_uint32),
        ]

    class CreateInfo(ctypes.Structure):
        _fields_ = [
            ("sType", ctypes.c_int), ("pNext", ctypes.c_void_p), ("flags", ctypes.c_uint32),
            ("pApplicationInfo", ctypes.POINTER(AppInfo)), ("enabledLayerCount", ctypes.c_uint32),
            ("ppEnabledLayerNames", ctypes.c_void_p), ("enabledExtensionCount", ctypes.c_uint32),
            ("ppEnabledExtensionNames", ctypes.c_void_p),
        ]

    app = AppInfo(0, None, b"jarvis", 1, b"jarvis", 1, (1 << 22) | (1 << 12))  # Vulkan 1.1
    info = CreateInfo(1, None, 0, ctypes.pointer(app), 0, None, 0, None)
    instance = ctypes.c_void_p()
    lib.vkCreateInstance.argtypes = [ctypes.POINTER(CreateInfo), ctypes.c_void_p, ctypes.POINTER(ctypes.c_void_p)]
    if lib.vkCreateInstance(ctypes.byref(info), None, ctypes.byref(instance)) != 0:
        return []
    result = []
    try:
        lib.vkEnumeratePhysicalDevices.argtypes = [ctypes.c_void_p, ctypes.POINTER(ctypes.c_uint32), ctypes.c_void_p]
        count = ctypes.c_uint32()
        lib.vkEnumeratePhysicalDevices(instance, ctypes.byref(count), None)
        devices = (ctypes.c_void_p * count.value)()
        lib.vkEnumeratePhysicalDevices(instance, ctypes.byref(count), devices)
        lib.vkGetPhysicalDeviceProperties.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
        for i in range(count.value):
            props = ctypes.create_string_buffer(4096)  # VkPhysicalDeviceProperties is ~824 bytes
            lib.vkGetPhysicalDeviceProperties(ctypes.c_void_p(devices[i]), props)
            dev_type = int.from_bytes(props.raw[16:20], "little")
            name = props.raw[20:276].split(b"\0", 1)[0].decode("utf-8", "replace")
            result.append((i, name, dev_type))
    finally:
        lib.vkDestroyInstance.argtypes = [ctypes.c_void_p, ctypes.c_void_p]
        lib.vkDestroyInstance(instance, None)
    return result


def pick_vulkan_device(devices):
    """Index of the device whisper.cpp should use: a discrete GPU, else an integrated one."""
    for wanted in (2, 1):
        for index, _name, dev_type in devices:
            if dev_type == wanted:
                return index
    return None


def main():
    info = detect()
    if "--write" in sys.argv:
        PROFILE_FILE.write_text(json.dumps(info, ensure_ascii=False, indent=2), encoding="utf-8")
    print(json.dumps(info))  # ASCII: read by PowerShell through the console code page


if __name__ == "__main__":
    main()
