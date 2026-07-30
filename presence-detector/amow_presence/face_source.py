"""Face-presence sources — the only place OpenCV and MediaPipe live.

Everything hardware-facing is quarantined behind the :class:`FaceSource` port so
the rest of the module (and its entire test suite) runs with no camera and
without OpenCV/MediaPipe installed. ``cv2`` and ``mediapipe`` are imported
lazily inside :meth:`MediaPipeFaceSource.open`, so importing this module is
always cheap and safe.
"""

from __future__ import annotations

import logging
from typing import Protocol, runtime_checkable

logger = logging.getLogger(__name__)


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
    """OpenCV capture + MediaPipe face detection.

    Detection is intentionally the only work done per frame, and the caller
    (the runner) governs how often that happens, so CPU cost scales with the
    configured frame rate rather than the camera's native rate.
    """

    def __init__(
        self,
        camera_index: int = 0,
        min_detection_confidence: float = 0.5,
        model_selection: int = 0,
    ) -> None:
        self._camera_index = camera_index
        self._min_detection_confidence = min_detection_confidence
        self._model_selection = model_selection
        self._capture: object | None = None
        self._detector: object | None = None

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
        except ImportError as exc:  # pragma: no cover - depends on host packages
            raise FaceSourceError(
                "opencv-python and mediapipe are required to run the live detector"
            ) from exc

        capture = cv2.VideoCapture(self._camera_index)
        if not capture.isOpened():
            capture.release()
            raise FaceSourceError(f"could not open camera index {self._camera_index}")

        try:
            detector = mp.solutions.face_detection.FaceDetection(
                model_selection=self._model_selection,
                min_detection_confidence=self._min_detection_confidence,
            )
        except Exception as exc:  # pragma: no cover - depends on host packages
            # An incompatible or broken MediaPipe build (a frequent problem on
            # unsupported Python versions) fails here. Release the camera we just
            # opened and surface it as a capture error the runner skips, rather
            # than leaking the device and crash-looping on the next tick.
            capture.release()
            raise FaceSourceError(f"could not initialise the face detector: {exc}") from exc

        self._capture = capture
        self._detector = detector
        logger.info("camera %d opened; MediaPipe face detection ready", self._camera_index)

    def is_face_present(self) -> bool:
        """Capture one frame and report whether MediaPipe finds a face.

        Raises :class:`FaceSourceError` on a capture failure (so the runner can
        distinguish "camera broken" from "nobody there").
        """
        if self._capture is None or self._detector is None:
            self.open()
        import cv2  # local: already proven importable by open()

        # A successful open() guarantees both handles; if we still lack them,
        # treat it as a capture failure the runner can skip rather than crashing.
        if self._capture is None or self._detector is None:
            raise FaceSourceError("camera/detector not initialised")
        ok, frame = self._capture.read()  # type: ignore[attr-defined]
        if not ok or frame is None:
            raise FaceSourceError("failed to read a frame from the camera")

        # MediaPipe expects RGB; OpenCV delivers BGR.
        rgb = cv2.cvtColor(frame, cv2.COLOR_BGR2RGB)
        results = self._detector.process(rgb)  # type: ignore[attr-defined]
        return bool(getattr(results, "detections", None))

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

    def __enter__(self) -> MediaPipeFaceSource:
        self.open()
        return self

    def __exit__(self, *_exc: object) -> None:
        self.close()
