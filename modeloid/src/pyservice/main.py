"""Modeloid's Python service.

Modeloid runs `python main.py <command> [args]` inside modeloid/.venv. Each
command writes JSON lines to stdout (one object per line); the last line is
the result. Errors exit non-zero with {"error": "..."}. Human-readable logs go
to stderr.

Commands:
  check   Versions of Python and the ML packages, and which devices torch sees.
"""

import json
import platform
import sys


def emit(obj):
    print(json.dumps(obj), flush=True)


def check():
    import torch
    import torchvision
    import ultralytics

    cuda = torch.cuda.is_available()
    mps = bool(getattr(torch.backends, "mps", None) and torch.backends.mps.is_available())
    if cuda:
        device_name = torch.cuda.get_device_name(0)
    elif mps:
        device_name = "Apple GPU (MPS)"
    else:
        device_name = "CPU"
    emit({
        "python": platform.python_version(),
        "torch": torch.__version__,
        "torchvision": torchvision.__version__,
        "ultralytics": ultralytics.__version__,
        "cuda_available": cuda,
        "cuda_version": torch.version.cuda,
        "mps_available": mps,
        "device_name": device_name,
    })


COMMANDS = {"check": check}


def main(argv):
    if len(argv) < 2 or argv[1] not in COMMANDS:
        emit({"error": f"usage: main.py <{'|'.join(COMMANDS)}>"})
        return 2
    try:
        COMMANDS[argv[1]]()
    except Exception as e:  # report every failure as JSON
        emit({"error": f"{type(e).__name__}: {e}"})
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
