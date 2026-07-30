"""Command-line entry point: run the live presence detector as a sidecar.

Events are written to **stdout** as newline-delimited JSON (the bus contract);
all logs and diagnostics go to **stderr** so the event channel stays clean.

    python -m amow_presence --config ~/.config/com.automute.walkaway/config.json

Ctrl-C (SIGINT) or SIGTERM stops it cleanly and releases the camera.
"""

from __future__ import annotations

import argparse
import logging
import signal
import sys
from types import FrameType

from .config import DetectorConfig
from .event_bus import StdoutEventBus
from .face_source import MediaPipeFaceSource
from .runner import DetectorRunner


def _parse_args(argv: list[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        prog="amow_presence",
        description="Local webcam presence detector for Auto-Mute on Walkaway.",
    )
    parser.add_argument(
        "--config",
        help="Path to the app config.json; only its presence grace periods are read.",
    )
    parser.add_argument("--camera", type=int, default=None, help="Camera index (default 0).")
    parser.add_argument("--fps", type=float, default=None, help="Target frames per second.")
    parser.add_argument(
        "--confidence",
        type=float,
        default=None,
        help="MediaPipe minimum detection confidence in [0, 1].",
    )
    parser.add_argument(
        "--model",
        type=int,
        choices=(0, 1),
        default=None,
        help="MediaPipe model: 0 short-range (default), 1 full-range.",
    )
    parser.add_argument(
        "--log-level",
        default="info",
        choices=("error", "warning", "info", "debug"),
        help="Diagnostic log level (written to stderr).",
    )
    return parser.parse_args(argv)


def _build_config(args: argparse.Namespace) -> DetectorConfig:
    overrides = {
        "target_fps": args.fps,
        "camera_index": args.camera,
        "min_detection_confidence": args.confidence,
        "model_selection": args.model,
    }
    if args.config:
        return DetectorConfig.from_app_config_file(args.config, **overrides)
    supplied = {k: v for k, v in overrides.items() if v is not None}
    return DetectorConfig(**supplied)  # type: ignore[arg-type]


def main(argv: list[str] | None = None) -> int:
    args = _parse_args(argv)
    logging.basicConfig(
        stream=sys.stderr,
        level=getattr(logging, args.log_level.upper()),
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
    )

    try:
        config = _build_config(args)
    except ValueError as exc:
        print(f"invalid configuration: {exc}", file=sys.stderr)
        return 2

    source = MediaPipeFaceSource(
        camera_index=config.camera_index,
        min_detection_confidence=config.min_detection_confidence,
        model_selection=config.model_selection,
    )
    runner = DetectorRunner(source=source, bus=StdoutEventBus(), config=config)

    def _handle_stop(_signum: int, _frame: FrameType | None) -> None:
        runner.stop()

    signal.signal(signal.SIGINT, _handle_stop)
    signal.signal(signal.SIGTERM, _handle_stop)

    try:
        runner.run()
    finally:
        source.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
