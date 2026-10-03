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

Errors exit non-zero; `check` prints {"error": "..."}, `train` an error event.
"""

import json
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
    emit({
        "optuna": optuna_version,
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


def main(argv):
    command = argv[1] if len(argv) > 1 else ""
    if command == "check":
        try:
            check()
        except Exception as e:  # report every failure as JSON
            emit({"error": f"{type(e).__name__}: {e}"})
            return 1
        return 0
    if command in ("train", "tune") and len(argv) > 2:
        try:
            (train if command == "train" else tune)(argv[2])
        except KeyboardInterrupt:
            return 130
        except Exception as e:
            import traceback

            traceback.print_exc()
            event("error", error=f"{type(e).__name__}: {e}")
            return 1
        return 0
    emit({"error": "usage: main.py check | train <config.json> | tune <config.json>"})
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
