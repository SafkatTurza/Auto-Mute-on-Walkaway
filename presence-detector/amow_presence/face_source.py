"""Face-presence sources — the only place OpenCV and MediaPipe live.

Everything hardware-facing is quarantined behind the :class:`FaceSource` port so
the rest of the module (and its entire test suite) runs with no camera and
without OpenCV/MediaPipe installed. ``cv2`` and ``mediapipe`` are imported
lazily inside :meth:`MediaPipeFaceSource.open`, so importing this module is
always cheap and safe.

Detection uses MediaPipe's **Tasks** API (``mediapipe.tasks…FaceDetector``),
the API supported by current MediaPipe (1.0+, which removed the old
``mediapipe.solutions`` package) as well as 0.10.x. It runs a small local model
bundled with the package — no download, no network — in keeping with the app's
"everything runs locally" promise.
"""

from __future__ import annotations

import logging
from pathlib import Path
from typing import Protocol, runtime_checkable

logger = logging.getLogger(__name__)

# The bundled face-detection model (BlazeFace short-range, tuned for faces within
# ~2m of a webcam — exactly the desk case). Shipped in the package so the
# detector never reaches the network.
_MODEL_DIR = Path(__file__).resolve().parent / "models"
DEFAULT_MODEL_PATH = _MODEL_DIR / "blaze_face_short_range.tflite"


class FaceSourceError(RuntimeError):
    """Raised when a frame cannot be captured (camera lost, open failed).

    This is distinct from "a valid frame contained no face": a capture failure
    must never be interpreted as the user being away, so the runner skips the
    tick rather than feeding a false ``face_present=False``.
    """


@runtime_checkable
class FaceSource(Protocol):
    """Yields whether a face is currently visible to the camera."""

    def is_face_present(self) -> bool: ...

    def close(self) -> None: ...


class MediaPipeFaceSource:
    """OpenCV capture + MediaPipe (Tasks API) face detection.

    Detection is intentionally the only work done per frame, and the caller
    (the runner) governs how often that happens, so CPU cost scales with the
    configured frame rate rather than the camera's native rate.
    """

    def __init__(
        self,
        camera_index: int = 0,
        min_detection_confidence: float = 0.5,
        model_selection: int = 0,
        model_path: str | Path | None = None,
    ) -> None:
        self._camera_index = camera_index
        self._min_detection_confidence = min_detection_confidence
        # Retained for CLI/config compatibility. The bundled Tasks model is
        # short-range; the full-range variant the old solutions API exposed has
        # no standalone Tasks model, so both selections use the short-range one
        # (the right choice for desk-distance webcam use regardless).
        self._model_selection = model_selection
        self._model_path = Path(model_path) if model_path is not None else DEFAULT_MODEL_PATH
        self._capture: object | None = None
        self._detector: object | None = None
        # The mediapipe module itself, kept so per-frame Image wrapping needs no
        # re-import. Set together with the detector; cleared on close.
        self._mp: object | None = None

    def open(self) -> None:
        """Acquire the camera and detector. Idempotent and atomic.

        Both handles are assigned to ``self`` only once both are built, so a
        failure part-way through never leaves a half-open source (a camera with
        no detector). That matters because a partially-open source would make
        every later tick fail: the re-open guard would see the camera already
        set and skip re-initialising the detector. A camera opened here but
        orphaned by a detector failure is released before raising.
        """
        if self._capture is not None and self._detector is not None:
            return
        try:
            import cv2
            import mediapipe as mp
            from mediapipe.tasks import python as mp_python
            from mediapipe.tasks.python import vision as mp_vision
        except ImportError as exc:  # pragma: no cover - depends on host packages
            raise FaceSourceError(
                f"opencv-python and mediapipe are required to run the live detector: {exc}"
            ) from exc

        if not self._model_path.is_file():
            raise FaceSourceError(f"face-detection model not found at {self._model_path}")

        capture = cv2.VideoCapture(self._camera_index)
        if not capture.isOpened():
            capture.release()
            raise FaceSourceError(f"could not open camera index {self._camera_index}")

        try:
            base_options = mp_python.BaseOptions(model_asset_path=str(self._model_path))
            options = mp_vision.FaceDetectorOptions(
                base_options=base_options,
                min_detection_confidence=self._min_detection_confidence,
            )
            detector = mp_vision.FaceDetector.create_from_options(options)
        except Exception as exc:  # pragma: no cover - depends on host packages
            # Building the detector failed (a broken/incompatible MediaPipe
            # build, or a bad model). Release the camera we just opened and
            # surface it as a capture error the runner skips, rather than
            # leaking the device and crash-looping on the next tick.
            capture.release()
            raise FaceSourceError(f"could not initialise the face detector: {exc}") from exc

        self._capture = capture
        self._detector = detector
        self._mp = mp
        logger.info("camera %d opened; MediaPipe face detector ready", self._camera_index)

    def is_face_present(self) -> bool:
        """Capture one frame and report whether MediaPipe finds a face.

        Raises :class:`FaceSourceError` on a capture failure (so the runner can
        distinguish "camera broken" from "nobody there").
        """
        if self._capture is None or self._detector is None:
            self.open()
        import cv2  # local: already proven importable by open()

        # A successful open() guarantees all three handles; if we still lack
        # them, treat it as a capture failure the runner skips rather than
        # crashing.
        if self._capture is None or self._detector is None or self._mp is None:
            raise FaceSourceError("camera/detector not initialised")
        ok, frame = self._capture.read()  # type: ignore[attr-defined]
        if not ok or frame is None:
            raise FaceSourceError("failed to read a frame from the camera")

        # MediaPipe expects RGB; OpenCV delivers BGR.
        rgb = cv2.cvtColor(frame, cv2.COLOR_BGR2RGB)
        mp = self._mp
        image = mp.Image(image_format=mp.ImageFormat.SRGB, data=rgb)  # type: ignore[attr-defined]
        result = self._detector.detect(image)  # type: ignore[attr-defined]
        return bool(getattr(result, "detections", None))

    def close(self) -> None:
        """Release the camera and detector. Safe to call more than once."""
        if self._capture is not None:
            self._capture.release()  # type: ignore[attr-defined]
            self._capture = None
        if self._detector is not None:
            close = getattr(self._detector, "close", None)
            if callable(close):
                close()
            self._detector = None
        self._mp = None

    def __enter__(self) -> MediaPipeFaceSource:
        self.open()
        return self

    def __exit__(self, *_exc: object) -> None:
        self.close()
