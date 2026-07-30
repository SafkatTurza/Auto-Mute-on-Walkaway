"""Tests for MediaPipeFaceSource.open() atomicity, with fake cv2/mediapipe.

The real OpenCV and MediaPipe are never imported: fakes are injected into
``sys.modules`` so ``open()`` exercises its acquisition and cleanup logic on any
machine, camera or not. The bug these guard against: a detector-build failure
that leaves the camera open, so every later tick crash-loops on a stale,
half-open source.
"""

from __future__ import annotations

import sys
import unittest
from types import ModuleType, SimpleNamespace

from amow_presence.face_source import FaceSourceError, MediaPipeFaceSource


class FakeCapture:
    def __init__(self, opened: bool = True) -> None:
        self._opened = opened
        self.released = 0

    def isOpened(self) -> bool:  # noqa: N802 - mirrors the cv2 API
        return self._opened

    def read(self):
        return True, object()

    def release(self) -> None:
        self.released += 1


def _install_fake_cv2(capture: FakeCapture) -> ModuleType:
    cv2 = ModuleType("cv2")
    cv2.VideoCapture = lambda _index: capture  # type: ignore[attr-defined]
    cv2.cvtColor = lambda frame, _code: frame  # type: ignore[attr-defined]
    cv2.COLOR_BGR2RGB = 4  # type: ignore[attr-defined]
    sys.modules["cv2"] = cv2
    return cv2


_MEDIAPIPE_MODULES = (
    "mediapipe",
    "mediapipe.solutions",
    "mediapipe.solutions.face_detection",
)


def _install_fake_mediapipe(detector_factory) -> None:
    """Register a fake mediapipe whose face-detection *submodule* is importable.

    The code under test does ``from mediapipe(.python).solutions import
    face_detection``, so the fake must exist as real ``sys.modules`` entries,
    not merely as an attribute on the top-level module.
    """
    mp = ModuleType("mediapipe")
    solutions = ModuleType("mediapipe.solutions")
    face_detection = ModuleType("mediapipe.solutions.face_detection")
    face_detection.FaceDetection = detector_factory  # type: ignore[attr-defined]
    solutions.face_detection = face_detection  # type: ignore[attr-defined]
    mp.solutions = solutions  # type: ignore[attr-defined]
    sys.modules["mediapipe"] = mp
    sys.modules["mediapipe.solutions"] = solutions
    sys.modules["mediapipe.solutions.face_detection"] = face_detection


class OpenAtomicityTests(unittest.TestCase):
    def tearDown(self) -> None:
        sys.modules.pop("cv2", None)
        for name in _MEDIAPIPE_MODULES:
            sys.modules.pop(name, None)

    def test_detector_failure_releases_camera_and_raises_capture_error(self) -> None:
        capture = FakeCapture(opened=True)
        _install_fake_cv2(capture)

        def broken_detector(**_kwargs):
            raise RuntimeError("incompatible mediapipe build")

        _install_fake_mediapipe(broken_detector)

        source = MediaPipeFaceSource(camera_index=0)
        with self.assertRaises(FaceSourceError):
            source.open()

        # The camera we opened must be released, and no half-open state left
        # behind — otherwise the next tick would skip re-init and crash.
        self.assertEqual(capture.released, 1)
        self.assertIsNone(source._capture)
        self.assertIsNone(source._detector)

    def test_open_can_recover_on_a_later_attempt(self) -> None:
        capture = FakeCapture(opened=True)
        _install_fake_cv2(capture)

        attempts = {"n": 0}

        def sometimes_detector(**_kwargs):
            attempts["n"] += 1
            if attempts["n"] == 1:
                raise RuntimeError("first attempt fails")
            return SimpleNamespace(process=lambda _rgb: SimpleNamespace(detections=[]))

        _install_fake_mediapipe(sometimes_detector)

        source = MediaPipeFaceSource(camera_index=0)
        with self.assertRaises(FaceSourceError):
            source.open()

        # A fresh camera is handed out on retry; the earlier one was released.
        recovered = FakeCapture(opened=True)
        _install_fake_cv2(recovered)
        source.open()
        self.assertIsNotNone(source._capture)
        self.assertIsNotNone(source._detector)
        # is_face_present now works without re-raising.
        self.assertFalse(source.is_face_present())

    def test_camera_open_failure_releases_and_raises(self) -> None:
        capture = FakeCapture(opened=False)
        _install_fake_cv2(capture)
        _install_fake_mediapipe(lambda **_kwargs: None)

        source = MediaPipeFaceSource(camera_index=3)
        with self.assertRaises(FaceSourceError):
            source.open()
        self.assertEqual(capture.released, 1)
        self.assertIsNone(source._capture)


if __name__ == "__main__":
    unittest.main()
