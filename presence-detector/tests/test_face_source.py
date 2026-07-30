"""Tests for MediaPipeFaceSource.open() atomicity, with fake cv2/mediapipe.

The real OpenCV and MediaPipe are never imported: fakes are injected into
``sys.modules`` so ``open()`` exercises its acquisition and cleanup logic on any
machine, camera or not. The bug these guard against: a detector-build failure
that leaves the camera open, so every later tick crash-loops on a stale,
half-open source. Detection goes through MediaPipe's Tasks API, so the fake
mediapipe mirrors that surface (``mediapipe.tasks.python`` + ``.vision``).
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
    "mediapipe.tasks",
    "mediapipe.tasks.python",
    "mediapipe.tasks.python.vision",
)


def _install_fake_mediapipe(create_from_options) -> None:
    """Register a fake mediapipe exposing the Tasks face-detector surface.

    The code under test does ``import mediapipe as mp`` plus ``from
    mediapipe.tasks import python`` and ``from mediapipe.tasks.python import
    vision``, so those must be real ``sys.modules`` entries. ``create_from_options``
    stands in for ``FaceDetector.create_from_options`` — the test injects one
    that succeeds or raises.
    """
    mp = ModuleType("mediapipe")
    mp.ImageFormat = SimpleNamespace(SRGB=1)  # type: ignore[attr-defined]
    mp.Image = lambda image_format, data: SimpleNamespace(  # type: ignore[attr-defined]
        image_format=image_format, data=data
    )

    tasks = ModuleType("mediapipe.tasks")
    tasks_python = ModuleType("mediapipe.tasks.python")
    tasks_python.BaseOptions = lambda model_asset_path: SimpleNamespace(  # type: ignore[attr-defined]
        model_asset_path=model_asset_path
    )
    vision = ModuleType("mediapipe.tasks.python.vision")
    vision.FaceDetectorOptions = lambda **kwargs: SimpleNamespace(**kwargs)  # type: ignore[attr-defined]
    vision.FaceDetector = SimpleNamespace(create_from_options=create_from_options)  # type: ignore[attr-defined]

    tasks_python.vision = vision  # type: ignore[attr-defined]
    tasks.python = tasks_python  # type: ignore[attr-defined]
    mp.tasks = tasks  # type: ignore[attr-defined]

    sys.modules["mediapipe"] = mp
    sys.modules["mediapipe.tasks"] = tasks
    sys.modules["mediapipe.tasks.python"] = tasks_python
    sys.modules["mediapipe.tasks.python.vision"] = vision


def _working_detector():
    """A detector whose detect() reports no face (the simplest valid result)."""
    return SimpleNamespace(detect=lambda _image: SimpleNamespace(detections=[]))


class OpenAtomicityTests(unittest.TestCase):
    def tearDown(self) -> None:
        sys.modules.pop("cv2", None)
        for name in _MEDIAPIPE_MODULES:
            sys.modules.pop(name, None)

    def test_detector_failure_releases_camera_and_raises_capture_error(self) -> None:
        capture = FakeCapture(opened=True)
        _install_fake_cv2(capture)

        def broken(_options):
            raise RuntimeError("incompatible mediapipe build")

        _install_fake_mediapipe(broken)

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

        def sometimes(_options):
            attempts["n"] += 1
            if attempts["n"] == 1:
                raise RuntimeError("first attempt fails")
            return _working_detector()

        _install_fake_mediapipe(sometimes)

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
        _install_fake_mediapipe(lambda _options: _working_detector())

        source = MediaPipeFaceSource(camera_index=3)
        with self.assertRaises(FaceSourceError):
            source.open()
        self.assertEqual(capture.released, 1)
        self.assertIsNone(source._capture)

    def test_missing_model_file_raises_capture_error(self) -> None:
        capture = FakeCapture(opened=True)
        _install_fake_cv2(capture)
        _install_fake_mediapipe(lambda _options: _working_detector())

        source = MediaPipeFaceSource(camera_index=0, model_path="/no/such/model.tflite")
        with self.assertRaises(FaceSourceError):
            source.open()
        # The camera is never opened when the model is missing.
        self.assertEqual(capture.released, 0)
        self.assertIsNone(source._capture)


if __name__ == "__main__":
    unittest.main()
