"""Tests for detector configuration loading and validation."""

from __future__ import annotations

import json
import unittest
from pathlib import Path
from tempfile import TemporaryDirectory

from amow_presence.config import DetectorConfig


class DetectorConfigTests(unittest.TestCase):
    def test_defaults_are_valid(self) -> None:
        cfg = DetectorConfig()
        self.assertEqual(cfg.target_fps, 15.0)
        self.assertAlmostEqual(cfg.frame_interval_s, 1 / 15)

    def test_rejects_bad_values(self) -> None:
        for kwargs in (
            {"away_grace_ms": 0},
            {"return_grace_ms": -1},
            {"target_fps": 0},
            {"target_fps": 120},
            {"camera_index": -1},
            {"min_detection_confidence": 1.5},
            {"model_selection": 2},
        ):
            with self.subTest(kwargs=kwargs), self.assertRaises(ValueError):
                DetectorConfig(**kwargs)  # type: ignore[arg-type]

    def test_reads_presence_graces_from_app_config(self) -> None:
        with TemporaryDirectory() as d:
            path = Path(d) / "config.json"
            # Includes keys the detector must ignore (behavior/logging/version).
            path.write_text(
                json.dumps(
                    {
                        "version": 1,
                        "behavior": {"auto_mute": True, "sample_interval_ms": 500},
                        "presence": {"away_grace_ms": 4200, "return_grace_ms": 900},
                        "logging": {"level": "info"},
                    }
                ),
                encoding="utf-8",
            )
            cfg = DetectorConfig.from_app_config_file(path)
            self.assertEqual(cfg.away_grace_ms, 4200)
            self.assertEqual(cfg.return_grace_ms, 900)
            self.assertEqual(cfg.target_fps, 15.0)  # detector default, untouched

    def test_overrides_win_over_file(self) -> None:
        with TemporaryDirectory() as d:
            path = Path(d) / "config.json"
            path.write_text(json.dumps({"presence": {"away_grace_ms": 4200}}), encoding="utf-8")
            cfg = DetectorConfig.from_app_config_file(path, target_fps=10.0, away_grace_ms=2000)
            self.assertEqual(cfg.target_fps, 10.0)
            self.assertEqual(cfg.away_grace_ms, 2000)

    def test_missing_file_falls_back_to_defaults(self) -> None:
        cfg = DetectorConfig.from_app_config_file("/no/such/config.json")
        self.assertEqual(cfg.away_grace_ms, DetectorConfig().away_grace_ms)


if __name__ == "__main__":
    unittest.main()
