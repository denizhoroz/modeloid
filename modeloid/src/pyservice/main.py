"""Modeloid's Python service.

Modeloid runs `python main.py <command> [args]` inside modeloid/.venv.

Commands:
  check           Versions of Python and the ML packages, and which devices
                  torch sees. Prints one JSON object.
  train <config>  Trains a model as described by the JSON file <config>:
                  {"model": "yolo11n.pt", "task": "detect", "rtdetr": false,
                   "args": {Ultralytics train() arguments}}.
                  Ultralytics' own output passes through (the console Modeloid
                  shows); progress events are lines starting with
                  "@modeloid " followed by JSON: start, epoch, done, error.

Errors exit non-zero; `check` prints {"error": "..."}, `train` an error event.
"""

import json
import platform
import sys


def emit(obj):
    print(json.dumps(obj), flush=True)


def event(name, **data):
    """A progress event for Modeloid, on its own line among Ultralytics' output."""
    print("@modeloid " + json.dumps({"event": name, **data}), flush=True)


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


def train(config_path):
    with open(config_path, encoding="utf-8") as f:
        config = json.load(f)

    from ultralytics import RTDETR, YOLO

    if config.get("rtdetr"):
        model = RTDETR(config["model"])
    else:
        model = YOLO(config["model"], task=config.get("task"))

    def on_train_start(trainer):
        event("start", epochs=trainer.epochs, save_dir=str(trainer.save_dir))

    def on_fit_epoch_end(trainer):
        metrics = {k: float(v) for k, v in (trainer.metrics or {}).items() if isinstance(v, (int, float))}
        event("epoch", epoch=trainer.epoch + 1, epochs=trainer.epochs, metrics=metrics)

    model.add_callback("on_train_start", on_train_start)
    model.add_callback("on_fit_epoch_end", on_fit_epoch_end)
    model.train(**config["args"])

    trainer = model.trainer
    best = trainer.best if trainer.best and trainer.best.exists() else trainer.last
    event("done", save_dir=str(trainer.save_dir), best=str(best) if best else "")


def main(argv):
    command = argv[1] if len(argv) > 1 else ""
    if command == "check":
        try:
            check()
        except Exception as e:  # report every failure as JSON
            emit({"error": f"{type(e).__name__}: {e}"})
            return 1
        return 0
    if command == "train" and len(argv) > 2:
        try:
            train(argv[2])
        except KeyboardInterrupt:
            return 130
        except Exception as e:
            import traceback

            traceback.print_exc()
            event("error", error=f"{type(e).__name__}: {e}")
            return 1
        return 0
    emit({"error": "usage: main.py check | train <config.json>"})
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
