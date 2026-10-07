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
                  "@modeloid " followed by JSON: start, epoch, done (with a
                  summary: final scores, speed, model size, peak memory), error.
  tune <config>   Tunes hyperparameters with Optuna:
                  {"model", "task", "rtdetr", "args" (Ultralytics train()
                   arguments shared by every trial), "trials": 20,
                   "seed": 0, "space": [{"key": "lr0", "kind": "range",
                   "low": 1e-5, "high": 0.1, "log": true, "int": false} |
                   {"key": "optimizer", "kind": "candidates", "values": [...]}]}.
                  Each trial trains in args["project"]/trial-<n> and is scored
                  by Ultralytics' fitness (higher is better). Events: start
                  (trials), trial-start, epoch, trial (number, value, params,
                  state), done (best trial, value, params), error.
  augment <config> Augmented copies of images with Albumentations:
                  {"seed": 0, "out_dir": "...", "transforms": [{"name":
                   "HorizontalFlip", "p": 0.5, "params": {...}}],
                   "images": [{"index": 0, "file": "...", "width", "height",
                   "copies": 2, "objects": [{"class": 0, "bbox": [x, y, w, h],
                   "polygon": [[x, y], ...] | null}]}]} (pixels).
                  Writes the copies to out_dir. Events: start (images),
                  image (index, outputs: [{file, width, height, objects}],
                  or error), done.
  hf-download <config> Downloads a Hugging Face dataset repo with
                  huggingface_hub (Xet storage, several files at once):
                  {"repo": "owner/name", "revision": "main", "out_dir": "...",
                   "workers": 8}. The token comes from HF_TOKEN. Events:
                  start (files, bytes), progress (bytes so far), done (files:
                  the repo paths written).

Errors exit non-zero; `check` prints {"error": "..."}, the others an error event.
"""

import json
import math
import os
import platform
import shutil
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
    try:
        import optuna

        optuna_version = optuna.__version__
    except ImportError:
        optuna_version = ""
    try:
        import albumentations

        albumentations_version = albumentations.__version__
    except ImportError:
        albumentations_version = ""
    try:
        import huggingface_hub

        huggingface_hub_version = huggingface_hub.__version__
    except ImportError:
        huggingface_hub_version = ""
    emit({
        "optuna": optuna_version,
        "albumentations": albumentations_version,
        "huggingface_hub": huggingface_hub_version,
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
        event("start", epochs=trainer.epochs, save_dir=str(trainer.save_dir), device_name=device_name(trainer.device))

    def on_fit_epoch_end(trainer):
        metrics = {k: float(v) for k, v in (trainer.metrics or {}).items() if isinstance(v, (int, float))}
        event("epoch", epoch=trainer.epoch + 1, epochs=trainer.epochs, metrics=metrics)

    model.add_callback("on_train_start", on_train_start)
    model.add_callback("on_fit_epoch_end", on_fit_epoch_end)
    model.train(**config["args"])

    trainer = model.trainer
    best = trainer.best if trainer.best and trainer.best.exists() else trainer.last
    event("done", save_dir=str(trainer.save_dir), best=str(best) if best else "", summary=run_summary(model, trainer))


def device_name(device):
    """The name of the hardware training runs on, e.g. "NVIDIA GeForce RTX 3060",
    or "" when it can't be read."""
    try:
        if device.type == "cuda":
            import torch

            return torch.cuda.get_device_name(device)
        # CPU, and Apple GPUs (part of the chip): the processor's name.
        from ultralytics.utils.torch_utils import get_cpu_info

        return get_cpu_info()
    except Exception:
        return ""


def finite(value):
    """`value` as a float, or None when it isn't a finite number (JSON has no NaN)."""
    try:
        value = float(value)
    except (TypeError, ValueError):
        return None
    return value if math.isfinite(value) else None


def peak_ram():
    """Most memory this process used, in bytes, or None."""
    try:
        if sys.platform == "win32":
            import psutil

            return int(psutil.Process().memory_info().peak_wset)
        import resource

        peak = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
        return int(peak if sys.platform == "darwin" else peak * 1024)  # bytes on macOS, KiB on Linux
    except Exception:
        return None


def run_summary(model, trainer):
    """What a finished training measured: the final validation of the best
    weights (Ultralytics runs it after the last epoch), the model's size, its
    speed and peak memory. Parts that can't be read are left out."""
    import torch

    summary = {}
    # After that final validation the trainer's metrics are the best weights' scores.
    final = {k: finite(v) for k, v in (trainer.metrics or {}).items() if k.startswith("metrics/")}
    summary["final_metrics"] = {k: v for k, v in final.items() if v is not None}
    speed = getattr(getattr(trainer, "validator", None), "speed", None) or {}
    summary["speed"] = {k: v for k, v in ((k, finite(v)) for k, v in speed.items()) if v is not None}
    try:
        # The trained model (train() loads the best weights back), counted like Ultralytics' summary line.
        from ultralytics.utils.torch_utils import get_flops, get_num_params

        net = model.model
        summary["layers"] = sum(1 for m in net.modules() if len(m._modules) == 0)
        summary["parameters"] = int(get_num_params(net))
        summary["gflops"] = finite(get_flops(net, trainer.args.imgsz)) or None
    except Exception:
        pass
    if getattr(trainer.device, "type", "") == "cuda" and torch.cuda.is_available():
        summary["peak_gpu_bytes"] = int(torch.cuda.max_memory_reserved(trainer.device))
    summary["peak_ram_bytes"] = peak_ram()
    return {k: v for k, v in summary.items() if v is not None}


def tune(config_path):
    with open(config_path, encoding="utf-8") as f:
        config = json.load(f)

    import optuna
    from ultralytics import RTDETR, YOLO

    space = config["space"]
    trials = int(config["trials"])
    base = dict(config["args"])

    def sample(trial):
        params = {}
        for p in space:
            key = p["key"]
            if p["kind"] == "candidates":
                params[key] = trial.suggest_categorical(key, p["values"])
            elif p.get("int"):
                params[key] = trial.suggest_int(key, int(p["low"]), int(p["high"]), log=bool(p.get("log")))
            else:
                params[key] = trial.suggest_float(key, float(p["low"]), float(p["high"]), log=bool(p.get("log")))
        return params

    def objective(trial):
        params = sample(trial)
        event("trial-start", number=trial.number + 1, trials=trials, params=params)
        if config.get("rtdetr"):
            model = RTDETR(config["model"])
        else:
            model = YOLO(config["model"], task=config.get("task"))

        def on_fit_epoch_end(trainer):
            metrics = {k: float(v) for k, v in (trainer.metrics or {}).items() if isinstance(v, (int, float))}
            event("epoch", trial=trial.number + 1, epoch=trainer.epoch + 1, epochs=trainer.epochs, metrics=metrics)

        model.add_callback("on_fit_epoch_end", on_fit_epoch_end)
        args = {**base, **params, "name": f"trial-{trial.number + 1}"}
        try:
            model.train(**args)
        except Exception as e:
            event("trial", number=trial.number + 1, value=None, params=params, state="failed", error=f"{type(e).__name__}: {e}")
            raise
        trainer = model.trainer
        # Trials are compared, not kept: Ultralytics saves weights on the last epoch even with save=False.
        shutil.rmtree(trainer.save_dir / "weights", ignore_errors=True)
        value = trainer.best_fitness if trainer.best_fitness is not None else trainer.fitness
        value = float(value) if value is not None else float("nan")
        metrics = {k: float(v) for k, v in (trainer.metrics or {}).items() if isinstance(v, (int, float))}
        event("trial", number=trial.number + 1, value=value, params=params, state="done", metrics=metrics)
        return value

    # Modeloid reports trials itself (numbered from 1); Optuna's own lines count from 0.
    optuna.logging.set_verbosity(optuna.logging.WARNING)
    sampler = optuna.samplers.TPESampler(seed=config.get("seed"))
    study = optuna.create_study(direction="maximize", sampler=sampler)
    event("start", trials=trials)
    # A failing trial (e.g. out of memory with a big batch) is recorded and the study goes on.
    study.optimize(objective, n_trials=trials, catch=(Exception,))

    done = [t for t in study.trials if t.state == optuna.trial.TrialState.COMPLETE]
    if not done:
        raise RuntimeError("no trial finished")
    best = study.best_trial
    event("done", trial=best.number + 1, value=best.value, params=best.params)


# Extensions OpenCV writes; other images are saved as JPEG.
WRITABLE = {".jpg", ".jpeg", ".png", ".bmp", ".webp", ".tif", ".tiff"}

# Albumentations takes OpenCV's numbers for how empty corners are filled.
BORDERS = {"constant": 0, "replicate": 1, "reflect": 4}


def build_transform(A, spec):
    """An Albumentations transform from {"name", "p", "params"}."""
    params = {k: tuple(v) if isinstance(v, list) else v for k, v in (spec.get("params") or {}).items()}
    if "border_mode" in params:
        params["border_mode"] = BORDERS[params["border_mode"]]
    for key in ("height", "width", "max_size"):
        if key in params:
            params[key] = int(params[key])
    name = spec["name"]
    if name == "RandomResizedCrop":
        params["size"] = (params.pop("height"), params.pop("width"))
    if name == "RandomCrop":
        params["pad_if_needed"] = True
    return getattr(A, name)(**params, p=float(spec["p"]))


def clip_polygon(points, width, height):
    """`points` cut to the image (Sutherland-Hodgman against its four edges)."""

    def at_x(a, b, x):
        return (x, a[1] + (b[1] - a[1]) * (x - a[0]) / (b[0] - a[0]))

    def at_y(a, b, y):
        return (a[0] + (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]), y)

    edges = [
        (lambda p: p[0] >= 0, lambda a, b: at_x(a, b, 0.0)),
        (lambda p: p[0] <= width, lambda a, b: at_x(a, b, width)),
        (lambda p: p[1] >= 0, lambda a, b: at_y(a, b, 0.0)),
        (lambda p: p[1] <= height, lambda a, b: at_y(a, b, height)),
    ]
    for inside, cut in edges:
        result = []
        for i, current in enumerate(points):
            previous = points[i - 1]
            if inside(current):
                if not inside(previous):
                    result.append(cut(previous, current))
                result.append(current)
            elif inside(previous):
                result.append(cut(previous, current))
        points = result
    return points


def augment_image(compose, item, out_dir):
    """Augmented copies of one image: written to out_dir, with their labels."""
    import os

    import cv2
    import numpy as np

    # np.fromfile + imdecode: cv2.imread can't open non-ASCII paths on Windows.
    image = cv2.imdecode(np.fromfile(item["file"], dtype=np.uint8), cv2.IMREAD_COLOR | cv2.IMREAD_IGNORE_ORIENTATION)
    if image is None:
        raise ValueError("not a readable image")
    image = cv2.cvtColor(image, cv2.COLOR_BGR2RGB)
    height, width = image.shape[:2]
    # Labels are in the size Modeloid read from the header; follow the pixels if they differ.
    sx = width / item["width"] if item.get("width") else 1.0
    sy = height / item["height"] if item.get("height") else 1.0

    objects = item["objects"]
    boxes, box_ids, points, point_ids = [], [], [], []
    for i, o in enumerate(objects):
        if o.get("polygon"):
            for x, y in o["polygon"]:
                points.append((x * sx, y * sy))
                point_ids.append(i)
            continue
        x, y, w, h = o["bbox"]
        x0, y0 = max(0.0, x * sx), max(0.0, y * sy)
        x1, y1 = min(width, (x + w) * sx), min(height, (y + h) * sy)
        if x1 - x0 >= 1 and y1 - y0 >= 1:
            boxes.append((x0, y0, x1 - x0, y1 - y0))
            box_ids.append(i)

    ext = os.path.splitext(item["file"])[1].lower()
    if ext not in WRITABLE:
        ext = ".jpg"
    outputs = []
    for copy in range(1, int(item["copies"]) + 1):
        r = compose(image=image, bboxes=boxes, box_ids=box_ids, keypoints=points, point_ids=point_ids)
        out = r["image"]
        oh, ow = out.shape[:2]
        found = []
        for box, i in zip(r["bboxes"], r["box_ids"]):
            x, y, w, h = (float(v) for v in box[:4])
            found.append((int(i), {"class": objects[int(i)]["class"], "bbox": [x, y, w, h], "polygon": None}))
        outlines = {}
        for point, i in zip(r["keypoints"], r["point_ids"]):
            outlines.setdefault(int(i), []).append((float(point[0]), float(point[1])))
        for i, outline in outlines.items():
            outline = clip_polygon(outline, float(ow), float(oh))
            if len(outline) < 3:
                continue
            xs, ys = [p[0] for p in outline], [p[1] for p in outline]
            x, y, w, h = min(xs), min(ys), max(xs) - min(xs), max(ys) - min(ys)
            if w >= 1 and h >= 1:
                found.append((i, {"class": objects[i]["class"], "bbox": [x, y, w, h], "polygon": [list(p) for p in outline]}))
        found.sort(key=lambda f: f[0])

        path = os.path.join(out_dir, f"{item['index']}_{copy}{ext}")
        ok, encoded = cv2.imencode(ext, cv2.cvtColor(out, cv2.COLOR_RGB2BGR))
        if not ok:
            raise ValueError(f"couldn't encode {ext}")
        encoded.tofile(path)
        outputs.append({"file": path, "width": ow, "height": oh, "objects": [f[1] for f in found]})
    return outputs


def augment(config_path):
    with open(config_path, encoding="utf-8") as f:
        config = json.load(f)

    import warnings

    import albumentations as A

    # Colour changes alone have nothing to do with the boxes and points; that's fine.
    warnings.filterwarnings("ignore", message=".*processor for.*")
    compose = A.Compose(
        [build_transform(A, t) for t in config["transforms"]],
        bbox_params=A.BboxParams(format="coco", label_fields=["box_ids"], clip=True, min_visibility=0.1, min_area=1),
        keypoint_params=A.KeypointParams(format="xy", label_fields=["point_ids"], remove_invisible=False),
        seed=config.get("seed"),
    )
    images = config["images"]
    event("start", images=len(images))
    for item in images:
        # One unreadable image doesn't stop the others.
        try:
            event("image", index=item["index"], outputs=augment_image(compose, item, config["out_dir"]))
        except Exception as e:
            event("image", index=item["index"], error=f"{type(e).__name__}: {e}")
    event("done", images=len(images))


def folder_size(path):
    """Bytes under `path`, half-written files included; unreadable ones skipped."""
    total = 0
    for root, _, files in os.walk(path):
        for name in files:
            try:
                total += os.path.getsize(os.path.join(root, name))
            except OSError:
                pass
    return total


def hf_download(config_path):
    with open(config_path, encoding="utf-8") as f:
        config = json.load(f)

    import threading

    from huggingface_hub import HfApi, snapshot_download

    repo, revision, out_dir = config["repo"], config.get("revision") or "main", config["out_dir"]
    skip = [".gitattributes"]
    info = HfApi().dataset_info(repo, revision=revision, files_metadata=True)
    files = [s for s in info.siblings or [] if s.rfilename not in skip]
    if not files:
        raise RuntimeError(f"Hugging Face dataset {repo} has no files")
    event("start", files=len(files), bytes=sum(s.size or 0 for s in files))

    # huggingface_hub reports per file; the folder's size is the progress.
    finished = threading.Event()

    def report():
        while not finished.wait(1.0):
            event("progress", bytes=folder_size(out_dir))

    reporter = threading.Thread(target=report, daemon=True)
    reporter.start()
    try:
        snapshot_download(
            repo_id=repo,
            repo_type="dataset",
            revision=revision,
            local_dir=out_dir,
            ignore_patterns=skip,
            max_workers=config.get("workers", 8),
        )
    finally:
        finished.set()
        reporter.join()
    # Download bookkeeping huggingface_hub keeps in the folder.
    shutil.rmtree(os.path.join(out_dir, ".cache", "huggingface"), ignore_errors=True)
    try:
        os.rmdir(os.path.join(out_dir, ".cache"))
    except OSError:
        pass
    event("done", files=[s.rfilename for s in files])


def main(argv):
    command = argv[1] if len(argv) > 1 else ""
    if command == "check":
        try:
            check()
        except Exception as e:  # report every failure as JSON
            emit({"error": f"{type(e).__name__}: {e}"})
            return 1
        return 0
    commands = {"train": train, "tune": tune, "augment": augment, "hf-download": hf_download}
    if command in commands and len(argv) > 2:
        try:
            commands[command](argv[2])
        except KeyboardInterrupt:
            return 130
        except Exception as e:
            import traceback

            traceback.print_exc()
            event("error", error=f"{type(e).__name__}: {e}")
            return 1
        return 0
    emit({"error": "usage: main.py check | train | tune | augment | hf-download <config.json>"})
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
